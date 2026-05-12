"""viewport-churn scenario.

Models the sub/unsub churn produced by a web-map client panning and zooming.
Each simulated client maintains an active subscription set; every
``--churn-interval-ms`` it picks a random direction (N/S/E/W/zoom-in/zoom-out)
and emits the subscribe/unsubscribe diff corresponding to a one-tile hop.

The harness ramps the number of simulated clients up linearly over the
duration. We report:

* **Churn rate sustained** — the maximum sub+unsub operations per second
  successfully completed without observable drop.
* **Steady-state subscription count** — the total active subscription count
  across all clients at the end of the run.
"""
from __future__ import annotations

import asyncio
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
    lat_rad = math.radians(lat)
    n = 2 ** zoom
    x = int((lon + 180.0) / 360.0 * n)
    y = int((1.0 - math.log(math.tan(lat_rad) + 1 / math.cos(lat_rad)) / math.pi) / 2.0 * n)
    return x, y


class _Client:
    """One simulated browser-like map client."""

    def __init__(
        self,
        name: str,
        mqtt_host: str,
        set_name: str,
        zoom: int,
        centre_xy: tuple[int, int],
        viewport_radius: int,
        rec: Recorder,
        rnd_seed: int,
    ) -> None:
        host, port = mqtt_host.split(":")
        self.host = host
        self.port = int(port)
        self.name = name
        self.set_name = set_name
        self.zoom = zoom
        self.cx, self.cy = centre_xy
        self.r = viewport_radius
        self.rec = rec
        self.rnd = random.Random(rnd_seed)
        self.client = mqtt.Client(
            callback_api_version=mqtt.CallbackAPIVersion.VERSION2,
            client_id=name,
            clean_session=True,
        )
        self.connected = threading.Event()
        self.client.on_connect = self._on_connect
        self.subscribed: set[str] = set()
        self.ops_total = 0
        self.ops_dropped = 0

    def _on_connect(self, client, userdata, flags, rc, properties=None):
        self.connected.set()

    def _viewport_topics(self) -> set[str]:
        topics: set[str] = set()
        for dx in range(-self.r, self.r + 1):
            for dy in range(-self.r, self.r + 1):
                x = self.cx + dx
                y = self.cy + dy
                topics.add(f"geo/{self.set_name}/{self.zoom}/{x}/{y}")
        return topics

    def connect(self) -> None:
        self.client.connect(self.host, self.port, keepalive=30)
        self.client.loop_start()
        # Initial subscription set.
        self.subscribed = self._viewport_topics()
        self._do_subs(self.subscribed, set())

    def stop(self) -> None:
        self.client.loop_stop()
        try:
            self.client.disconnect()
        except Exception:  # noqa: BLE001
            pass

    def churn_step(self) -> None:
        """One hop: move centre, diff subscriptions."""
        direction = self.rnd.choice(["n", "s", "e", "w", "zoom-in", "zoom-out"])
        old_cx, old_cy, old_zoom = self.cx, self.cy, self.zoom
        if direction == "n":
            self.cy -= 1
        elif direction == "s":
            self.cy += 1
        elif direction == "e":
            self.cx += 1
        elif direction == "w":
            self.cx -= 1
        elif direction == "zoom-in":
            self.zoom = min(self.zoom + 1, 14)
            self.cx *= 2
            self.cy *= 2
        elif direction == "zoom-out":
            self.zoom = max(self.zoom - 1, 4)
            self.cx //= 2
            self.cy //= 2

        new_set = self._viewport_topics()
        to_sub = new_set - self.subscribed
        to_unsub = self.subscribed - new_set
        self._do_subs(to_sub, to_unsub)
        self.subscribed = new_set

    def _do_subs(self, to_sub: set[str], to_unsub: set[str]) -> None:
        for t in to_sub:
            res = self.client.subscribe(t, qos=0)
            self.ops_total += 1
            if res[0] != mqtt.MQTT_ERR_SUCCESS:
                self.ops_dropped += 1
        for t in to_unsub:
            res = self.client.unsubscribe(t)
            self.ops_total += 1
            if res[0] != mqtt.MQTT_ERR_SUCCESS:
                self.ops_dropped += 1


async def _preload(
    client: aioredis.Redis,
    set_name: str,
    object_count: int,
    centre_lon: float, centre_lat: float, half_deg: float,
) -> None:
    """Sequential awaits with bounded concurrency; see snapshot_burst preload note."""
    rnd = random.Random(0xFEED)
    sem = asyncio.Semaphore(32)

    async def _one(i: int) -> None:
        async with sem:
            lon = centre_lon + rnd.uniform(-half_deg, half_deg)
            lat = centre_lat + rnd.uniform(-half_deg, half_deg)
            await client.geoadd(set_name, [lon, lat, f"churn-{i:06d}"])

    await asyncio.gather(*[_one(i) for i in range(object_count)])


async def _async_run(
    cfg: BenchConfig,
    clients: int,
    churn_interval_ms: int,
    viewport_tiles: int,
    set_name: str,
    object_count: int,
) -> dict[str, Any]:
    cfg = cfg.resolve()
    rec = Recorder("viewport-churn", cfg)
    rec.start()

    gconfig = fetch_geomqtt_config(cfg.http_host)
    zooms = gconfig.get("zooms") or [10]
    z = zooms[len(zooms) // 2]
    cx, cy = _lonlat_to_tile(11.34, 44.49, z)
    r = max(1, int(math.sqrt(viewport_tiles) / 2))

    console.print(
        f"[bold]viewport-churn[/bold]: clients={clients}, "
        f"interval={churn_interval_ms}ms, viewport_tiles≈{(2*r+1)**2}, zoom={z}"
    )

    # Pre-load some objects so there's actual snapshot traffic per subscription.
    pool = aioredis.from_url(f"redis://{cfg.resp_host}", decode_responses=True)
    try:
        await _preload(pool, set_name, object_count, 11.34, 44.49, 0.1)
    finally:
        await pool.aclose()

    # Spin up clients in waves so we observe a ramp.
    start = time.monotonic()
    deadline = start + cfg.duration
    warmup_until = start + cfg.warmup
    active: list[_Client] = []
    ramp_interval = max(0.1, cfg.duration / max(1, clients))

    async def _spawn_and_churn() -> None:
        for i in range(clients):
            if time.monotonic() >= deadline:
                break
            c = _Client(
                f"bench-churn-{i:04d}", cfg.mqtt_host, set_name,
                z, (cx, cy), r, rec, rnd_seed=i,
            )
            c.connect()
            c.connected.wait(timeout=5.0)
            active.append(c)
            await asyncio.sleep(ramp_interval)

    async def _churn_loop() -> None:
        interval = churn_interval_ms / 1000.0
        while time.monotonic() < deadline:
            t0 = time.perf_counter_ns()
            ops_before = sum(c.ops_total for c in active)
            for c in active:
                c.churn_step()
            ops_after = sum(c.ops_total for c in active)
            tick_ms = (time.perf_counter_ns() - t0) / 1e6
            now = time.monotonic()
            rec.record(
                "churn_tick", tick_ms,
                warmup=(now < warmup_until),
                active_clients=len(active),
                ops_this_tick=ops_after - ops_before,
            )
            await asyncio.sleep(max(0, interval - tick_ms / 1000.0))

    await asyncio.gather(_spawn_and_churn(), _churn_loop())

    # Compute sustained sub+unsub rate post-warmup.
    post_warmup_ticks = [
        e for e in rec._events
        if e.event == "churn_tick" and not e.warmup
    ]
    total_ops = sum(e.extra.get("ops_this_tick", 0) for e in post_warmup_ticks)
    if post_warmup_ticks:
        first_ts = post_warmup_ticks[0].ts_ms
        last_ts = post_warmup_ticks[-1].ts_ms
        duration_s = max(0.001, (last_ts - first_ts) / 1000.0)
    else:
        duration_s = 1.0
    churn_rate = total_ops / duration_s
    steady_state_subs = sum(len(c.subscribed) for c in active)
    drops = sum(c.ops_dropped for c in active)

    for c in active:
        c.stop()

    summary: dict[str, Any] = {
        "subscenario":            "viewport-churn",
        "set_name":               set_name,
        "clients":                len(active),
        "churn_interval_ms":      churn_interval_ms,
        "viewport_tiles":         (2 * r + 1) ** 2,
        "zoom":                   z,
        "churn_rate_sustained":   churn_rate,
        "total_ops":              total_ops,
        "ops_dropped":            drops,
        "steady_state_subs":      steady_state_subs,
    }
    rec.write_csv()
    rec.write_summary(summary)
    console.print(
        f"[green]done[/green] viewport-churn: clients={len(active)}, "
        f"sustained {churn_rate:.0f} sub+unsub /s, "
        f"steady-state subs={steady_state_subs}, drops={drops}"
    )
    return summary


def run(
    cfg: BenchConfig,
    clients: int = 200,
    churn_interval_ms: int = 1000,
    viewport_tiles: int = 9,
    set_name: str = "vehicles",
    object_count: int = 5_000,
) -> dict[str, Any]:
    return asyncio.run(_async_run(
        cfg, clients=clients, churn_interval_ms=churn_interval_ms,
        viewport_tiles=viewport_tiles, set_name=set_name,
        object_count=object_count,
    ))
