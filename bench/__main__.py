"""CLI entry point for the geomqtt benchmark harness.

Subcommands:

* ``smoke``            short version of every scenario, ~30 seconds total.
* ``write-rate``       write-rate ceiling scenario.
* ``fanout``           per-publish fan-out latency scenario.
* ``snapshot-burst``   SUBSCRIBE-to-first-event scenario.
* ``viewport-churn``   pan/zoom subscription churn scenario.
* ``all``              run write-rate + fanout + snapshot-burst + viewport-churn
                       at full duration.
* ``analyze``          walk ``results/`` and emit a Markdown summary table.

All subcommands accept ``--resp-host``, ``--mqtt-host``, ``--http-host``,
``--duration``, ``--warmup``, and a results-directory override.

Host defaults are read from the environment so the same code works both inside
the docker-compose bench network (``geomqtt:6380`` etc.) and against a local
geomqtt deployment (``127.0.0.1:6380`` etc.). Set
``GEOMQTT_BENCH_RESP_HOST`` / ``..._MQTT_HOST`` / ``..._MQTT_WS_HOST`` /
``..._HTTP_HOST`` to override.
"""
from __future__ import annotations

import os

import typer

from bench.scenarios import (
    cross_node,
    fanout,
    snapshot_burst,
    viewport_churn,
    write_rate,
)
from bench.scenarios.common import (
    BenchConfig,
    analyze_results,
)

_DEFAULT_RESP    = os.environ.get("GEOMQTT_BENCH_RESP_HOST",    "127.0.0.1:6380")
_DEFAULT_MQTT    = os.environ.get("GEOMQTT_BENCH_MQTT_HOST",    "127.0.0.1:1883")
_DEFAULT_MQTT_WS = os.environ.get("GEOMQTT_BENCH_MQTT_WS_HOST", "127.0.0.1:8083")
_DEFAULT_HTTP    = os.environ.get("GEOMQTT_BENCH_HTTP_HOST",    "127.0.0.1:8080")
_DEFAULT_RESULTS = os.environ.get("GEOMQTT_BENCH_RESULTS_DIR",  "results")


app = typer.Typer(
    add_completion=False,
    no_args_is_help=True,
    help="Quantitative scaling benchmark for geomqtt.",
)


def _config(
    resp_host: str = _DEFAULT_RESP,
    mqtt_host: str = _DEFAULT_MQTT,
    mqtt_ws_host: str = _DEFAULT_MQTT_WS,
    http_host: str = _DEFAULT_HTTP,
    duration: int = 60,
    warmup: int = 5,
    results_dir: str = _DEFAULT_RESULTS,
) -> BenchConfig:
    return BenchConfig(
        resp_host=resp_host,
        mqtt_host=mqtt_host,
        mqtt_ws_host=mqtt_ws_host,
        http_host=http_host,
        duration=duration,
        warmup=warmup,
        results_dir=results_dir,
    )


@app.command("write-rate")
def write_rate_cmd(
    resp_host: str = _DEFAULT_RESP,
    http_host: str = _DEFAULT_HTTP,
    duration: int = 60,
    warmup: int = 5,
    target_rate: int = 1000,
    rate_ramp: str = "linear",
    set_name: str = "vehicles",
    object_count: int = 10_000,
    p95_budget_ms: float = 100.0,
    results_dir: str = _DEFAULT_RESULTS,
) -> None:
    """Write-rate ceiling: ramp GEOADD rate until RTT P95 exceeds budget."""
    cfg = _config(
        resp_host=resp_host, http_host=http_host,
        duration=duration, warmup=warmup, results_dir=results_dir,
    )
    write_rate.run(
        cfg,
        target_rate=target_rate,
        rate_ramp=rate_ramp,
        set_name=set_name,
        object_count=object_count,
        p95_budget_ms=p95_budget_ms,
    )


@app.command("fanout")
def fanout_cmd(
    resp_host: str = _DEFAULT_RESP,
    mqtt_host: str = _DEFAULT_MQTT,
    http_host: str = _DEFAULT_HTTP,
    duration: int = 60,
    warmup: int = 5,
    subscribers: int = 100,
    publish_rate: int = 50,
    set_name: str = "vehicles",
    results_dir: str = _DEFAULT_RESULTS,
) -> None:
    """Per-publish fan-out latency to N concurrent MQTT subscribers."""
    cfg = _config(
        resp_host=resp_host, mqtt_host=mqtt_host, http_host=http_host,
        duration=duration, warmup=warmup, results_dir=results_dir,
    )
    fanout.run(
        cfg,
        subscribers=subscribers,
        publish_rate=publish_rate,
        set_name=set_name,
    )


@app.command("snapshot-burst")
def snapshot_burst_cmd(
    resp_host: str = _DEFAULT_RESP,
    mqtt_host: str = _DEFAULT_MQTT,
    http_host: str = _DEFAULT_HTTP,
    duration: int = 60,
    warmup: int = 0,
    subscribers: int = 100,
    viewport_tiles: int = 9,
    object_count: int = 10_000,
    set_name: str = "vehicles",
    results_dir: str = _DEFAULT_RESULTS,
) -> None:
    """SUBSCRIBE-to-first-event and SUBSCRIBE-to-full-burst latency."""
    cfg = _config(
        resp_host=resp_host, mqtt_host=mqtt_host, http_host=http_host,
        duration=duration, warmup=warmup, results_dir=results_dir,
    )
    snapshot_burst.run(
        cfg,
        subscribers=subscribers,
        viewport_tiles=viewport_tiles,
        object_count=object_count,
        set_name=set_name,
    )


@app.command("viewport-churn")
def viewport_churn_cmd(
    resp_host: str = _DEFAULT_RESP,
    mqtt_host: str = _DEFAULT_MQTT,
    http_host: str = _DEFAULT_HTTP,
    duration: int = 60,
    warmup: int = 5,
    clients: int = 200,
    churn_interval_ms: int = 1000,
    viewport_tiles: int = 9,
    set_name: str = "vehicles",
    object_count: int = 5_000,
    results_dir: str = _DEFAULT_RESULTS,
) -> None:
    """Realistic pan/zoom client subscription churn."""
    cfg = _config(
        resp_host=resp_host, mqtt_host=mqtt_host, http_host=http_host,
        duration=duration, warmup=warmup, results_dir=results_dir,
    )
    viewport_churn.run(
        cfg,
        clients=clients,
        churn_interval_ms=churn_interval_ms,
        viewport_tiles=viewport_tiles,
        set_name=set_name,
        object_count=object_count,
    )


@app.command("cross-node-fanout")
def cross_node_fanout_cmd(
    writer_resp_host: str = "",
    writer_http_host: str = "",
    subscriber_mqtt_hosts: str = "",
    duration: int = 60,
    warmup: int = 5,
    subscribers: int = 99,
    publish_rate: int = 50,
    set_name: str = "bench-xnode",
    results_dir: str = _DEFAULT_RESULTS,
) -> None:
    """Multi-node fan-out: writer on one node, subscribers split across N nodes.

    Reports local vs cross-node fan-out latency separately and the bridge
    overhead between them. Reads writer + subscriber-host defaults from env
    (``GEOMQTT_BENCH_WRITER_RESP_HOST`` etc.) when CLI flags are empty.
    """
    cfg = _config(
        duration=duration, warmup=warmup, results_dir=results_dir,
    )
    cross_node.run(
        cfg,
        subscribers=subscribers,
        publish_rate=publish_rate,
        set_name=set_name,
        writer_resp_host=writer_resp_host or None,
        writer_http_host=writer_http_host or None,
        subscriber_mqtt_hosts=subscriber_mqtt_hosts or None,
    )


@app.command("smoke")
def smoke_cmd(
    resp_host: str = _DEFAULT_RESP,
    mqtt_host: str = _DEFAULT_MQTT,
    http_host: str = _DEFAULT_HTTP,
    results_dir: str = _DEFAULT_RESULTS,
) -> None:
    """Short version of every scenario — useful for CI sanity checks."""
    cfg = _config(
        resp_host=resp_host, mqtt_host=mqtt_host, http_host=http_host,
        duration=10, warmup=2, results_dir=results_dir,
    )
    write_rate.run(cfg, target_rate=500, rate_ramp="none",
                   set_name="bench-smoke", object_count=200, p95_budget_ms=200.0)
    fanout.run(cfg, subscribers=10, publish_rate=20, set_name="bench-smoke")
    snapshot_burst.run(cfg, subscribers=10, viewport_tiles=4,
                       object_count=500, set_name="bench-smoke")
    viewport_churn.run(cfg, clients=20, churn_interval_ms=500,
                       viewport_tiles=4, set_name="bench-smoke",
                       object_count=500)
    # cross-node only meaningful when multi-node env vars are set; the
    # scenario degenerates to a single-node fanout otherwise.
    cross_node.run(cfg, subscribers=9, publish_rate=20,
                   set_name="bench-smoke")


@app.command("all")
def all_cmd(
    resp_host: str = _DEFAULT_RESP,
    mqtt_host: str = _DEFAULT_MQTT,
    http_host: str = _DEFAULT_HTTP,
    duration: int = 60,
    warmup: int = 5,
    results_dir: str = _DEFAULT_RESULTS,
) -> None:
    """Run write-rate + fanout + snapshot-burst + viewport-churn at full duration."""
    cfg = _config(
        resp_host=resp_host, mqtt_host=mqtt_host, http_host=http_host,
        duration=duration, warmup=warmup, results_dir=results_dir,
    )
    write_rate.run(cfg, target_rate=1000, rate_ramp="linear",
                   set_name="bench-write", object_count=10_000,
                   p95_budget_ms=100.0)
    fanout.run(cfg, subscribers=100, publish_rate=50, set_name="bench-fanout")
    snapshot_burst.run(cfg, subscribers=100, viewport_tiles=9,
                       object_count=10_000, set_name="bench-snapshot")
    viewport_churn.run(cfg, clients=200, churn_interval_ms=1000,
                       viewport_tiles=9, set_name="bench-churn",
                       object_count=5_000)
    cross_node.run(cfg, subscribers=99, publish_rate=50,
                   set_name="bench-xnode")


@app.command("analyze")
def analyze_cmd(results_dir: str = _DEFAULT_RESULTS) -> None:
    """Walk results/ and emit a Markdown summary table."""
    analyze_results(results_dir)


if __name__ == "__main__":
    app()
