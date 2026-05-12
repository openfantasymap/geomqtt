"""cross-node-fanout scenario.

Writer publishes through ``GEOMQTT_BENCH_WRITER_RESP_HOST`` (one node).
Subscribers are split equally across the comma-separated list
``GEOMQTT_BENCH_SUBSCRIBER_MQTT_HOSTS`` (one or more nodes). The harness
records per-publish, per-subscriber receive latency and buckets the
observations by whether the subscriber's node is the same as the
writer's node (``local``) or a different node (``cross-node``).

Reports:

* Local fanout latency:    P50 / P95 / P99
* Cross-node fanout latency: P50 / P95 / P99
* Bridge overhead = cross-node P50 minus local P50.
"""
from __future__ import annotations

import asyncio
import json
import math
import os
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


def _resolve_writer_node_name(writer_host: str) -> str:
    """Strip ``:port`` and any tags so we can tag observations by node.

    ``geomqtt-1:6380`` → ``geomqtt-1``.
    """
    return writer_host.split(":", 1)[0]


def _resolve_subscriber_nodes(env_value: str | None, fallback: str) -> list[str]:
    """Parse the comma-separated subscriber-host env var.

    Each entry is ``host:port``; we keep both for connecting and the
    bare host for node-id tagging.
    """
    raw = (env_value or fallback).strip()
    items = [s.strip() for s in raw.split(",") if s.strip()]
    return items or [fallback]


class _Subscriber:
    """One MQTT subscriber on a specific geomqtt node."""

    def __init__(self, name: str, mqtt_host: str, node_id: str, topic: str) -> None:
        host, port = mqtt_host.split(":")
        self.host = host
        self.port = int(port)
        self.node_id = node_id
        self.client = mqtt.Client(
            callback_api_version=mqtt.CallbackAPIVersion.VERSION2,
            client_id=name,
            clean_session=True,
        )
        self.name = name
        self.topic = topic
        self.last_id_seen_at: dict[str, int] = {}
        self.ready = threading.Event()
        self.client.on_connect = self._on_connect
        self.client.on_message = self._on_message

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
        # Keep only the first-seen timestamp per obid for this subscriber
        # (snapshots+adds for the same obid can race; we want delivery
        # latency not duplicate-detection).
        self.last_id_seen_at.setdefault(obid, recv_ns)

    def start(self) -> None:
        self.client.connect(self.host, self.port, keepalive=30)
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
    """Issue GEOADDs at a fixed rate; record publish_ns per obid."""
    rnd = random.Random(0xCAFE)
    interval = 1.0 / max(1, publish_rate)
    next_emit = time.monotonic()
    seq = 0
    while time.monotonic() < deadline:
        now = time.monotonic()
        lon = 11.34 + rnd.uniform(-1e-4, 1e-4)
        lat = 44.49 + rnd.uniform(-1e-4, 1e-4)
        obid = f"xnode-{seq:06d}"
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
    writer_resp_host: str,
    writer_http_host: str,
    subscriber_mqtt_hosts: list[str],
) -> dict[str, Any]:
    cfg = cfg.resolve()
    rec = Recorder("cross-node-fanout", cfg)
    rec.start()

    # Pull effective zoom band from the writer node's /config.
    gconfig = fetch_geomqtt_config(writer_http_host)
    zooms = gconfig.get("zooms") or [10]
    z = zooms[len(zooms) // 2]
    tile_x, tile_y = _lonlat_to_tile(11.34, 44.49, z)
    topic = f"geo/{set_name}/{z}/{tile_x}/{tile_y}"

    writer_node = _resolve_writer_node_name(writer_resp_host)
    sub_node_ids = [_resolve_writer_node_name(h) for h in subscriber_mqtt_hosts]
    console.print(
        f"[bold]cross-node-fanout[/bold]: writer={writer_node}, "
        f"subscriber-nodes={sub_node_ids}, subs={subscribers}, "
        f"publish_rate={publish_rate}/s, topic={topic}"
    )

    # Spin up subscribers, round-robin across nodes.
    subs: list[_Subscriber] = []
    for i in range(subscribers):
        host = subscriber_mqtt_hosts[i % len(subscriber_mqtt_hosts)]
        node = sub_node_ids[i % len(sub_node_ids)]
        s = _Subscriber(f"bench-xnode-{i:04d}", host, node, topic)
        s.start()
        subs.append(s)
    for s in subs:
        s.ready.wait(timeout=5.0)

    start = time.monotonic()
    deadline = start + cfg.duration
    warmup_until = start + cfg.warmup
    publish_log: dict[str, int] = {}

    pool = aioredis.from_url(f"redis://{writer_resp_host}", decode_responses=True)
    try:
        await _writer(pool, set_name, publish_rate, deadline, warmup_until, rec, publish_log)
        # Drain window so trailing cross-node bridge messages can reach subs.
        await asyncio.sleep(2.0)
    finally:
        await pool.aclose()
        for s in subs:
            s.stop()

    # Per-subscriber × per-publish latency, bucketed by local vs cross-node.
    local_ms: list[float] = []
    cross_ms: list[float] = []
    per_node_ms: dict[str, list[float]] = {n: [] for n in set(sub_node_ids)}
    for s in subs:
        is_local = (s.node_id == writer_node)
        for obid, recv_ns in s.last_id_seen_at.items():
            publish_ns = publish_log.get(obid)
            if publish_ns is None:
                continue
            latency_ms = (recv_ns - publish_ns) / 1e6
            (local_ms if is_local else cross_ms).append(latency_ms)
            per_node_ms[s.node_id].append(latency_ms)
            rec.record(
                "xnode_observed", latency_ms,
                obid=obid, subscriber=s.name,
                sub_node=s.node_id, local=int(is_local),
            )

    local = summarise_latencies(local_ms)
    cross = summarise_latencies(cross_ms)
    per_node = {n: summarise_latencies(v) for n, v in per_node_ms.items()}

    bridge_overhead_p50 = cross.get("p50", 0.0) - local.get("p50", 0.0) if local["n"] > 0 and cross["n"] > 0 else None

    summary: dict[str, Any] = {
        "subscenario":      "cross-node-fanout",
        "set_name":         set_name,
        "topic":            topic,
        "zoom":             z,
        "subscribers":      subscribers,
        "publish_rate":     publish_rate,
        "writer_node":      writer_node,
        "subscriber_nodes": sub_node_ids,
        "local_fanout_ms":   local,
        "cross_fanout_ms":   cross,
        "per_node_fanout_ms": per_node,
        "bridge_overhead_ms_p50": bridge_overhead_p50,
        "observations": {
            "local":     local["n"],
            "cross":     cross["n"],
            "per_node":  {n: v["n"] for n, v in per_node.items()},
        },
    }
    rec.write_csv()
    rec.write_summary(summary)
    if local["n"] > 0 and cross["n"] > 0:
        console.print(
            f"[green]done[/green] cross-node-fanout: "
            f"local P50/P95={local['p50']:.2f}/{local['p95']:.2f}ms, "
            f"cross P50/P95={cross['p50']:.2f}/{cross['p95']:.2f}ms, "
            f"bridge overhead P50≈{bridge_overhead_p50:.2f}ms"
        )
    else:
        console.print(
            f"[yellow]done[/yellow] cross-node-fanout: "
            f"local n={local['n']}, cross n={cross['n']} (missing one bucket)"
        )
    return summary


def run(
    cfg: BenchConfig,
    subscribers: int = 99,
    publish_rate: int = 50,
    set_name: str = "bench-xnode",
    writer_resp_host: str | None = None,
    writer_http_host: str | None = None,
    subscriber_mqtt_hosts: str | None = None,
) -> dict[str, Any]:
    """Entry point invoked by the CLI.

    Hosts default to the GEOMQTT_BENCH_WRITER_* / SUBSCRIBER_* env vars,
    which the multi-node compose sets; if absent we fall back to the
    single-node BENCH_* hosts (so this scenario degenerates to plain
    fanout against one node, useful for sanity-checking).
    """
    writer_resp = writer_resp_host or os.environ.get(
        "GEOMQTT_BENCH_WRITER_RESP_HOST", cfg.resp_host
    )
    writer_http = writer_http_host or os.environ.get(
        "GEOMQTT_BENCH_WRITER_HTTP_HOST", cfg.http_host
    )
    sub_hosts_env = subscriber_mqtt_hosts or os.environ.get(
        "GEOMQTT_BENCH_SUBSCRIBER_MQTT_HOSTS"
    )
    sub_hosts = _resolve_subscriber_nodes(sub_hosts_env, cfg.mqtt_host)
    return asyncio.run(_async_run(
        cfg,
        subscribers=subscribers, publish_rate=publish_rate,
        set_name=set_name,
        writer_resp_host=writer_resp,
        writer_http_host=writer_http,
        subscriber_mqtt_hosts=sub_hosts,
    ))
