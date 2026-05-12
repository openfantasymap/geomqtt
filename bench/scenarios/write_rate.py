"""write-rate ceiling scenario.

A pool of asyncio writer tasks issue ``GEOADD <set> <lon> <lat> <obid>`` at a
ramped target rate. The harness measures per-``GEOADD`` round-trip time and
reports the rate at which P95 RTT first exceeds the configured budget — the
**write-rate ceiling**.
"""
from __future__ import annotations

import asyncio
import math
import random
import time
from typing import Any

import redis.asyncio as aioredis

from bench.scenarios.common import (
    BenchConfig,
    Recorder,
    console,
    summarise_latencies,
)


async def _writer(
    name: str,
    client: aioredis.Redis,
    rec: Recorder,
    set_name: str,
    object_pool: list[str],
    rate_at: callable,
    warmup_until: float,
    deadline: float,
) -> None:
    """Issue GEOADDs at the (time-varying) target rate, recording RTT."""
    rnd = random.Random(hash(name))
    next_emit = time.monotonic()
    while time.monotonic() < deadline:
        now = time.monotonic()
        rate = max(1.0, rate_at(now))
        interval = 1.0 / rate
        # Pick an object id and a perturbed position.
        obid = rnd.choice(object_pool)
        # Small bounding box centred on Bologna; values produce stable GEO scores.
        lon = 11.34 + rnd.uniform(-0.01, 0.01)
        lat = 44.49 + rnd.uniform(-0.01, 0.01)
        t0 = time.perf_counter_ns()
        try:
            await client.geoadd(set_name, [lon, lat, obid])
        except Exception as exc:  # noqa: BLE001
            rec.record("geoadd_error", 0.0, set=set_name, obid=obid, err=str(exc))
            await asyncio.sleep(interval)
            continue
        rtt_ms = (time.perf_counter_ns() - t0) / 1e6
        rec.record(
            "geoadd", rtt_ms,
            warmup=(now < warmup_until),
            set=set_name, obid=obid, rate=rate,
        )
        # Sleep enough to maintain the requested rate, accounting for the RTT
        # we just spent.
        next_emit += interval
        sleep = next_emit - time.monotonic()
        if sleep > 0:
            await asyncio.sleep(sleep)
        else:
            # We're behind schedule; reset to now so backpressure is observable.
            next_emit = time.monotonic()


async def _async_run(
    cfg: BenchConfig,
    target_rate: int,
    rate_ramp: str,
    set_name: str,
    object_count: int,
    p95_budget_ms: float,
) -> dict[str, Any]:
    cfg = cfg.resolve()
    rec = Recorder("write-rate", cfg)
    rec.start()
    object_pool = [f"obj-{i:06d}" for i in range(object_count)]

    # Build the rate-at-time function.
    start = time.monotonic()
    deadline = start + cfg.duration
    warmup_until = start + cfg.warmup

    if rate_ramp == "none":
        def rate_at(_now: float) -> float:
            return float(target_rate)
    elif rate_ramp == "linear":
        # Ramp from target_rate * 0.1 up to target_rate over the duration.
        def rate_at(now: float) -> float:
            frac = max(0.0, min(1.0, (now - start) / max(1.0, cfg.duration)))
            return target_rate * (0.1 + 0.9 * frac)
    elif rate_ramp == "exponential":
        # Geometric ramp: 0.1*target → target. Useful when you suspect the ceiling
        # is much higher or lower than target_rate.
        def rate_at(now: float) -> float:
            frac = max(0.0, min(1.0, (now - start) / max(1.0, cfg.duration)))
            return target_rate * 10 ** (frac - 1.0)
    else:
        raise ValueError(f"unknown --rate-ramp {rate_ramp!r}")

    # Choose writer count: enough to keep each task's per-event rate below 200/s.
    writers = max(1, math.ceil(target_rate / 200))
    console.print(
        f"[bold]write-rate[/bold]: duration={cfg.duration}s, target={target_rate}/s, "
        f"ramp={rate_ramp}, writers={writers}, objects={object_count}"
    )

    pool = aioredis.from_url(f"redis://{cfg.resp_host}", decode_responses=True)
    try:
        tasks = [
            asyncio.create_task(_writer(
                f"w{i}", pool, rec, set_name, object_pool,
                rate_at, warmup_until, deadline,
            ))
            for i in range(writers)
        ]
        await asyncio.gather(*tasks)
    finally:
        await pool.aclose()

    # Summarise.
    rtt = rec.percentiles(event="geoadd")
    post_warmup = [
        e for e in rec._events
        if e.event == "geoadd" and not e.warmup
    ]
    duration_post_warmup = max(0.001, cfg.duration - cfg.warmup)
    achieved_rate = len(post_warmup) / duration_post_warmup
    error_count = sum(1 for e in rec._events if e.event == "geoadd_error")

    # Find the rate at which P95 first exceeded the budget, scanning forward
    # through 1-second windows.
    ceiling = _find_p95_ceiling(post_warmup, p95_budget_ms, window_s=1.0)

    summary: dict[str, Any] = {
        "subscenario": "write-rate",
        "set_name":    set_name,
        "object_count": object_count,
        "target_rate":  target_rate,
        "rate_ramp":    rate_ramp,
        "achieved_rate": achieved_rate,
        "writers":      writers,
        "rtt_ms":       rtt,
        "errors":       error_count,
        "ceiling":      {
            "rate_at_p95_budget": ceiling,
            "p95_budget_ms":      p95_budget_ms,
        },
    }
    rec.write_csv()
    rec.write_summary(summary)
    console.print(
        f"[green]done[/green] write-rate: achieved {achieved_rate:.0f}/s, "
        f"P95={rtt['p95']:.2f}ms, ceiling≈{ceiling}/s @ P95<{p95_budget_ms}ms"
    )
    return summary


def _find_p95_ceiling(events: list, p95_budget_ms: float, window_s: float = 1.0) -> float | None:
    """Sweep through time-ordered events; return the rate at the first 1-s window where P95 > budget."""
    if not events:
        return None
    events = sorted(events, key=lambda e: e.ts_ms)
    win_ms = int(window_s * 1000)
    end_t = events[-1].ts_ms
    bucket: list[float] = []
    win_start = 0
    last_good_rate: float | None = None
    for e in events:
        if e.ts_ms - win_start < win_ms:
            bucket.append(e.latency_ms)
            continue
        # Close out the window.
        if bucket:
            stats = summarise_latencies(bucket)
            rate = len(bucket) / window_s
            if stats["p95"] <= p95_budget_ms:
                last_good_rate = rate
            else:
                return last_good_rate  # ceiling found
        win_start = e.ts_ms
        bucket = [e.latency_ms]
    return last_good_rate


def run(
    cfg: BenchConfig,
    target_rate: int = 1000,
    rate_ramp: str = "linear",
    set_name: str = "vehicles",
    object_count: int = 10_000,
    p95_budget_ms: float = 100.0,
) -> dict[str, Any]:
    """Entry point invoked by the CLI."""
    return asyncio.run(_async_run(
        cfg, target_rate=target_rate, rate_ramp=rate_ramp,
        set_name=set_name, object_count=object_count,
        p95_budget_ms=p95_budget_ms,
    ))
