"""snapshot-burst scenario.

Pre-loads ``--object-count`` points into the test set distributed across a
viewport-sized tile cluster. Then opens ``--subscribers`` concurrent MQTT
sessions, each subscribing to a viewport (``--viewport-tiles`` tiles), and
measures:

* SUBSCRIBE → first-message latency
* SUBSCRIBE → end-of-snapshot-burst latency (heuristic: 500 ms of silence
  after the most recent message)
* number of snapshot messages received per session

The P50 / P95 / P99 across all sessions of both latencies are the
**snapshot-burst latency**.
"""
from __future__ import annotations

import asyncio
import json
import math
import random
import threading
import time
from typing import Any

import paho.mqtt.client as mqtt
import redis.asyncio as aioredis

from bench.scenarios.common import (
    BenchConfig,
    Recorder,
    console,
    fetch_geomqtt_config,
    summarise_latencies,
)


def _lonlat_to_tile(lon: float, lat: float, zoom: int) -> tuple[int, int]:
    lat_rad = math.radians(lat)
    n = 2 ** zoom
    x = int((lon + 180.0) / 360.0 * n)
    y = int((1.0 - math.log(math.tan(lat_rad) + 1 / math.cos(lat_rad)) / math.pi) / 2.0 * n)
    return x, y


def _tile_to_lonlat(x: int, y: int, zoom: int) -> tuple[float, float]:
    n = 2 ** zoom
    lon = x / n * 360.0 - 180.0
    lat_rad = math.atan(math.sinh(math.pi * (1 - 2 * y / n)))
    return lon, math.degrees(lat_rad)


class _Subscriber:
    SILENCE_MS = 500   # heuristic: this much silence = burst is over

    def __init__(self, name: str, mqtt_host: str, topics: list[str]) -> None:
        host, port = mqtt_host.split(":")
        self.client = mqtt.Client(
            callback_api_version=mqtt.CallbackAPIVersion.VERSION2,
            client_id=name,
            clean_session=True,
        )
        self.name = name
        self.topics = topics
        self.t_subscribe_ns: int = 0
        self.t_first_msg_ns: int | None = None
        self.t_last_msg_ns: int | None = None
        self.message_count = 0
        self.snapshot_done = threading.Event()
        self._burst_watcher: threading.Thread | None = None
        self.client.on_message = self._on_message
        self.client.on_connect = self._on_connect

    def _on_connect(self, client, userdata, flags, rc, properties=None):
        for t in self.topics:
            client.subscribe(t, qos=0)
        self.t_subscribe_ns = time.perf_counter_ns()
        # Start a watcher that triggers snapshot_done when SILENCE_MS elapses
        # after the most recent message.
        self._burst_watcher = threading.Thread(target=self._watch_burst, daemon=True)
        self._burst_watcher.start()

    def _on_message(self, client, userdata, msg):
        recv_ns = time.perf_counter_ns()
        try:
            data = json.loads(msg.payload.decode())
        except Exception:
            return
        op = data.get("op")
        if op != "snapshot":
            return
        if self.t_first_msg_ns is None:
            self.t_first_msg_ns = recv_ns
        self.t_last_msg_ns = recv_ns
        self.message_count += 1

    def _watch_burst(self) -> None:
        # Wait until we see at least one message OR 2 seconds have passed.
        start = time.monotonic()
        while time.monotonic() - start < 2.0 and self.t_first_msg_ns is None:
            time.sleep(0.05)
        # Now wait until SILENCE_MS has elapsed since last message.
        while True:
            if self.t_last_msg_ns is None:
                # Never saw anything; declare snapshot empty after the 2s wait.
                self.snapshot_done.set()
                return
            elapsed_ms = (time.perf_counter_ns() - self.t_last_msg_ns) / 1e6
            if elapsed_ms >= self.SILENCE_MS:
                self.snapshot_done.set()
                return
            time.sleep(0.05)

    def start(self, host: str, port: int) -> None:
        self.client.connect(host, port, keepalive=30)
        self.client.loop_start()

    def stop(self) -> None:
        self.client.loop_stop()
        try:
            self.client.disconnect()
        except Exception:  # noqa: BLE001
            pass


async def _preload(
    client: aioredis.Redis,
    set_name: str,
    object_count: int,
    centre_lon: float, centre_lat: float, half_deg: float,
) -> None:
    """Write ``object_count`` synthetic points into the GEO set + attribute hashes.

    We deliberately avoid pipelining through the RESP proxy here: the proxy's
    response-count semantics for batched GEOADD / HSET-on-obj:* are not what
    redis-py's pipeline executor expects (intercepted commands produce
    side-effects on top of the upstream response). Sequential awaits with
    bounded concurrency keep this scenario portable across geomqtt versions.
    """
    rnd = random.Random(0xBEEF)
    sem = asyncio.Semaphore(32)  # bounded parallelism to keep RESP queue depth sane

    async def _one(i: int) -> None:
        async with sem:
            lon = centre_lon + rnd.uniform(-half_deg, half_deg)
            lat = centre_lat + rnd.uniform(-half_deg, half_deg)
            obid = f"snap-{i:06d}"
            await client.geoadd(set_name, [lon, lat, obid])
            await client.hset(f"obj:{obid}", mapping={"icon": "circle", "color": "blue"})

    await asyncio.gather(*[_one(i) for i in range(object_count)])


async def _async_run(
    cfg: BenchConfig,
    subscribers: int,
    viewport_tiles: int,
    object_count: int,
    set_name: str,
) -> dict[str, Any]:
    cfg = cfg.resolve()
    rec = Recorder("snapshot-burst", cfg)
    rec.start()

    gconfig = fetch_geomqtt_config(cfg.http_host)
    zooms = gconfig.get("zooms") or [10]
    z = zooms[len(zooms) // 2]
    # Centre on Bologna; spread points over an area that covers viewport_tiles tiles.
    side = int(math.sqrt(viewport_tiles) + 0.5)
    cx, cy = _lonlat_to_tile(11.34, 44.49, z)
    tiles = [(cx + dx, cy + dy) for dx in range(side) for dy in range(side)]
    # Compute the lon/lat bbox that exactly contains the chosen tile cluster.
    lon_min, lat_min = _tile_to_lonlat(tiles[0][0], tiles[0][1] + 1, z)
    lon_max, lat_max = _tile_to_lonlat(tiles[-1][0] + 1, tiles[-1][1], z)
    centre_lon = (lon_min + lon_max) / 2
    centre_lat = (lat_min + lat_max) / 2
    half_deg = max(abs(lon_max - lon_min), abs(lat_max - lat_min)) / 2

    console.print(
        f"[bold]snapshot-burst[/bold]: subscribers={subscribers}, "
        f"viewport_tiles={len(tiles)} ({side}x{side}), zoom={z}, objects={object_count}"
    )
    pool = aioredis.from_url(f"redis://{cfg.resp_host}", decode_responses=True)
    try:
        await _preload(pool, set_name, object_count, centre_lon, centre_lat, half_deg)
    finally:
        await pool.aclose()

    host, port = cfg.mqtt_host.split(":")
    port = int(port)
    topics = [f"geo/{set_name}/{z}/{x}/{y}" for (x, y) in tiles]

    subs: list[_Subscriber] = []
    for i in range(subscribers):
        s = _Subscriber(f"bench-snap-{i:04d}", cfg.mqtt_host, topics)
        # Stagger connect by a few ms to avoid trampling the broker accept loop.
        s.start(host, port)
        await asyncio.sleep(0.005)
        subs.append(s)

    # Wait for every subscriber's burst to drain (or timeout per session at 30s).
    DEADLINE_S = 30.0
    first_msg_ms: list[float] = []
    burst_end_ms: list[float] = []
    burst_size: list[int] = []
    for s in subs:
        s.snapshot_done.wait(timeout=DEADLINE_S)
        if s.t_first_msg_ns is not None:
            first_msg_ms.append((s.t_first_msg_ns - s.t_subscribe_ns) / 1e6)
        if s.t_last_msg_ns is not None:
            burst_end_ms.append((s.t_last_msg_ns - s.t_subscribe_ns) / 1e6)
        burst_size.append(s.message_count)
        rec.record(
            "subscriber_burst",
            (s.t_last_msg_ns - s.t_subscribe_ns) / 1e6 if s.t_last_msg_ns else 0.0,
            subscriber=s.name, count=s.message_count,
        )

    for s in subs:
        s.stop()

    summary: dict[str, Any] = {
        "subscenario":     "snapshot-burst",
        "set_name":        set_name,
        "subscribers":     subscribers,
        "viewport_tiles":  len(tiles),
        "object_count":    object_count,
        "zoom":            z,
        "first_message_ms": summarise_latencies(first_msg_ms),
        "burst_end_ms":     summarise_latencies(burst_end_ms),
        "burst_size":      summarise_latencies(burst_size),
    }
    rec.write_csv()
    rec.write_summary(summary)
    first = summary["first_message_ms"]
    end = summary["burst_end_ms"]
    console.print(
        f"[green]done[/green] snapshot-burst: subs={subscribers}, "
        f"first-msg P95={first['p95']:.2f}ms, burst-end P95={end['p95']:.2f}ms, "
        f"mean-burst-size={summary['burst_size']['mean']:.0f}"
    )
    return summary


def run(
    cfg: BenchConfig,
    subscribers: int = 100,
    viewport_tiles: int = 9,
    object_count: int = 10_000,
    set_name: str = "vehicles",
) -> dict[str, Any]:
    return asyncio.run(_async_run(
        cfg, subscribers=subscribers, viewport_tiles=viewport_tiles,
        object_count=object_count, set_name=set_name,
    ))
