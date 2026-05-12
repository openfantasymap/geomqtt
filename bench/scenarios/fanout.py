"""fanout scenario.

Single writer issues ``GEOADD`` at a fixed rate within a small geographic area
that maps to one tile at the configured zoom. ``--subscribers`` concurrent MQTT
sessions subscribe to that tile. The harness measures per-publish delay from
write completion to receipt at each subscriber. P50 / P95 / P99 across all
subscriber × publish events is the **fan-out latency**.

We compute the subscribed tile from the GEO point + the geomqtt /config zoom
list so the tile we publish to is exactly the tile we subscribe to.
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
)


def _lonlat_to_tile(lon: float, lat: float, zoom: int) -> tuple[int, int]:
    """Slippy-map tile coordinates for a (lon, lat) at the given zoom."""
    lat_rad = math.radians(lat)
    n = 2 ** zoom
    x = int((lon + 180.0) / 360.0 * n)
    y = int((1.0 - math.log(math.tan(lat_rad) + 1 / math.cos(lat_rad)) / math.pi) / 2.0 * n)
    return x, y


class _Subscriber:
    """One MQTT subscriber listening on a single tile topic."""

    def __init__(self, name: str, mqtt_host: str, topic: str, rec: Recorder) -> None:
        host, port = mqtt_host.split(":")
        self.client = mqtt.Client(
            callback_api_version=mqtt.CallbackAPIVersion.VERSION2,
            client_id=name,
            clean_session=True,
        )
        self.rec = rec
        self.topic = topic
        self.name = name
        self.last_id_seen_at: dict[str, int] = {}  # obid -> ns when this sub saw it
        self.ready = threading.Event()
        self.client.on_message = self._on_message
        self.client.on_connect = self._on_connect
        self.client.connect(host, int(port), keepalive=30)

    def _on_connect(self, client, userdata, flags, rc, properties=None):
        client.subscribe(self.topic, qos=0)
        self.ready.set()

    def _on_message(self, client, userdata, msg):
        recv_ns = time.perf_counter_ns()
        try:
            data = json.loads(msg.payload.decode())
        except Exception:
            return
        obid = data.get("id")
        if obid is None:
            return
        self.last_id_seen_at[obid] = recv_ns

    def start(self) -> None:
        self.client.loop_start()

    def stop(self) -> None:
        self.client.loop_stop()
        try:
            self.client.disconnect()
        except Exception:  # noqa: BLE001
            pass


async def _writer(
    client: aioredis.Redis,
    set_name: str,
    publish_rate: int,
    deadline: float,
    warmup_until: float,
    rec: Recorder,
    publish_log: dict[str, int],
) -> None:
    """Issue GEOADDs at a fixed rate; log a publish_ns per (obid, sequence)."""
    rnd = random.Random(0xC0DE)
    interval = 1.0 / max(1, publish_rate)
    next_emit = time.monotonic()
    seq = 0
    while time.monotonic() < deadline:
        now = time.monotonic()
        # Stay tightly within a single tile: perturb by ±0.0001°.
        lon = 11.34 + rnd.uniform(-1e-4, 1e-4)
        lat = 44.49 + rnd.uniform(-1e-4, 1e-4)
        obid = f"fanout-{seq:06d}"
        publish_ns = time.perf_counter_ns()
        publish_log[obid] = publish_ns
        try:
            await client.geoadd(set_name, [lon, lat, obid])
        except Exception as exc:  # noqa: BLE001
            rec.record("publish_error", 0.0, err=str(exc))
        rec.record(
            "publish", 0.0,
            warmup=(now < warmup_until),
            obid=obid, lon=lon, lat=lat,
        )
        seq += 1
        next_emit += interval
        sleep = next_emit - time.monotonic()
        if sleep > 0:
            await asyncio.sleep(sleep)
        else:
            next_emit = time.monotonic()


async def _async_run(
    cfg: BenchConfig,
    subscribers: int,
    publish_rate: int,
    set_name: str,
) -> dict[str, Any]:
    cfg = cfg.resolve()
    rec = Recorder("fanout", cfg)
    rec.start()

    gconfig = fetch_geomqtt_config(cfg.http_host)
    zooms = gconfig.get("zooms") or [10]
    chosen_zoom = zooms[len(zooms) // 2]  # middle zoom of the band
    tile_x, tile_y = _lonlat_to_tile(11.34, 44.49, chosen_zoom)
    topic = f"geo/{set_name}/{chosen_zoom}/{tile_x}/{tile_y}"
    console.print(
        f"[bold]fanout[/bold]: duration={cfg.duration}s, subscribers={subscribers}, "
        f"publish_rate={publish_rate}/s, topic={topic}"
    )

    # Spin up subscribers.
    subs: list[_Subscriber] = []
    for i in range(subscribers):
        s = _Subscriber(f"bench-sub-{i:04d}", cfg.mqtt_host, topic, rec)
        s.start()
        subs.append(s)
    # Wait for them all to ack the SUBSCRIBE.
    for s in subs:
        s.ready.wait(timeout=5.0)

    start = time.monotonic()
    deadline = start + cfg.duration
    warmup_until = start + cfg.warmup
    publish_log: dict[str, int] = {}

    pool = aioredis.from_url(f"redis://{cfg.resp_host}", decode_responses=True)
    try:
        await _writer(pool, set_name, publish_rate, deadline, warmup_until, rec, publish_log)
        # Small drain window so trailing publishes can reach subscribers.
        await asyncio.sleep(2.0)
    finally:
        await pool.aclose()
        for s in subs:
            s.stop()

    # Compute per-(sub, publish) latencies.
    fanout_latencies_ms: list[float] = []
    for s in subs:
        for obid, recv_ns in s.last_id_seen_at.items():
            publish_ns = publish_log.get(obid)
            if publish_ns is None:
                continue
            latency_ms = (recv_ns - publish_ns) / 1e6
            fanout_latencies_ms.append(latency_ms)
            rec.record("fanout_observed", latency_ms,
                       obid=obid, subscriber=s.name)

    from bench.scenarios.common import summarise_latencies
    lat = summarise_latencies(fanout_latencies_ms)
    summary: dict[str, Any] = {
        "subscenario":  "fanout",
        "set_name":     set_name,
        "topic":        topic,
        "zoom":         chosen_zoom,
        "tile":         [tile_x, tile_y],
        "subscribers":  subscribers,
        "publish_rate": publish_rate,
        "fanout_ms":    lat,
        "observations": len(fanout_latencies_ms),
    }
    rec.write_csv()
    rec.write_summary(summary)
    console.print(
        f"[green]done[/green] fanout: subs={subscribers}, observed={len(fanout_latencies_ms)}, "
        f"P50={lat['p50']:.2f}ms, P95={lat['p95']:.2f}ms, P99={lat['p99']:.2f}ms"
    )
    return summary


def run(
    cfg: BenchConfig,
    subscribers: int = 100,
    publish_rate: int = 50,
    set_name: str = "vehicles",
) -> dict[str, Any]:
    return asyncio.run(_async_run(
        cfg, subscribers=subscribers, publish_rate=publish_rate, set_name=set_name,
    ))
