"""Shared scaffolding for benchmark scenarios.

Defines the per-run config, recorders, percentile helpers, and the
analyze-results walker that emits a Markdown summary across all
results in ``results/``.
"""
from __future__ import annotations

import csv
import dataclasses
import datetime as dt
import json
import os
import pathlib
import statistics
from collections.abc import Iterable
from typing import Any

import httpx
import numpy as np
from rich.console import Console

console = Console()


@dataclasses.dataclass(slots=True)
class BenchConfig:
    """Per-run configuration shared across scenarios."""

    resp_host: str
    mqtt_host: str
    mqtt_ws_host: str
    http_host: str
    duration: int       # seconds
    warmup: int         # seconds; events from this window are tagged but excluded from summary
    results_dir: str

    def resolve(self) -> "BenchConfig":
        """Mkdirs the results dir if missing; idempotent."""
        pathlib.Path(self.results_dir).mkdir(parents=True, exist_ok=True)
        return self


def fetch_geomqtt_config(http_host: str, timeout: float = 5.0) -> dict[str, Any]:
    """Pull effective config from GET /config so the JSON summary is reproducible."""
    url = f"http://{http_host}/config"
    try:
        return httpx.get(url, timeout=timeout).json()
    except Exception as exc:  # noqa: BLE001 — we want the run to proceed
        console.print(f"[yellow]warning:[/yellow] could not fetch {url}: {exc}")
        return {"_error": str(exc)}


def fetch_geomqtt_status(http_host: str, timeout: float = 5.0) -> dict[str, str]:
    """Pull Prometheus-text /status as a parsed flat dict for the summary."""
    url = f"http://{http_host}/status"
    out: dict[str, str] = {}
    try:
        text = httpx.get(url, timeout=timeout).text
    except Exception as exc:  # noqa: BLE001
        return {"_error": str(exc)}
    for line in text.splitlines():
        line = line.strip()
        if not line or line.startswith("#"):
            continue
        # Prometheus text format: name{labels?} value [timestamp]
        try:
            head, value = line.rsplit(" ", 1)
        except ValueError:
            continue
        out[head] = value
    return out


@dataclasses.dataclass(slots=True)
class _Event:
    """A single timed observation."""

    ts_ms: int          # wall-clock ms since recorder start
    event: str          # short label (geoadd / publish_recv / subscribe / ...)
    latency_ms: float
    extra: dict[str, Any] = dataclasses.field(default_factory=dict)
    warmup: bool = False


class Recorder:
    """Time-stamped event recorder. CSV on disk + in-memory percentile summary."""

    def __init__(self, scenario: str, cfg: BenchConfig) -> None:
        self.scenario = scenario
        self.cfg = cfg
        ts = dt.datetime.now(dt.UTC).strftime("%Y%m%dT%H%M%SZ")
        self.run_id = f"{scenario}-{ts}"
        self._events: list[_Event] = []
        self._start_ns: int | None = None

    # --- timing helpers -----------------------------------------------------

    def start(self) -> None:
        self._start_ns = _now_ns()

    @property
    def elapsed_s(self) -> float:
        if self._start_ns is None:
            return 0.0
        return (_now_ns() - self._start_ns) / 1e9

    def record(
        self,
        event: str,
        latency_ms: float,
        warmup: bool = False,
        **extra: Any,
    ) -> None:
        if self._start_ns is None:
            self.start()
        assert self._start_ns is not None
        ts_ms = int((_now_ns() - self._start_ns) / 1e6)
        self._events.append(_Event(ts_ms, event, latency_ms, dict(extra), warmup))

    # --- output -------------------------------------------------------------

    def write_csv(self) -> str:
        cfg = self.cfg.resolve()
        path = os.path.join(cfg.results_dir, f"{self.run_id}.csv")
        with open(path, "w", newline="") as f:
            w = csv.writer(f)
            w.writerow(["ts_ms", "event", "latency_ms", "warmup", "extra"])
            for e in self._events:
                w.writerow([
                    e.ts_ms, e.event, f"{e.latency_ms:.4f}",
                    int(e.warmup), json.dumps(e.extra, separators=(",", ":")),
                ])
        return path

    def write_summary(self, summary: dict[str, Any]) -> str:
        cfg = self.cfg.resolve()
        path = os.path.join(cfg.results_dir, f"{self.run_id}.json")
        payload = {
            "scenario": self.scenario,
            "run_id":   self.run_id,
            "duration": self.cfg.duration,
            "warmup":   self.cfg.warmup,
            "config":   dataclasses.asdict(self.cfg),
            "geomqtt_config": fetch_geomqtt_config(self.cfg.http_host),
            "geomqtt_status_post": fetch_geomqtt_status(self.cfg.http_host),
            **summary,
        }
        with open(path, "w") as f:
            json.dump(payload, f, indent=2, sort_keys=True)
        return path

    # --- percentile helpers -------------------------------------------------

    def latencies(self, event: str | None = None, post_warmup: bool = True) -> list[float]:
        return [
            e.latency_ms for e in self._events
            if (event is None or e.event == event)
            and (not post_warmup or not e.warmup)
        ]

    def percentiles(self, event: str | None = None) -> dict[str, float]:
        lats = self.latencies(event=event)
        return summarise_latencies(lats)


# ---------------------------------------------------------------------------


def summarise_latencies(values: Iterable[float]) -> dict[str, float]:
    arr = np.array(list(values), dtype=float)
    if arr.size == 0:
        return {"n": 0, "p50": 0.0, "p95": 0.0, "p99": 0.0, "max": 0.0, "mean": 0.0}
    return {
        "n":   int(arr.size),
        "p50": float(np.percentile(arr, 50)),
        "p95": float(np.percentile(arr, 95)),
        "p99": float(np.percentile(arr, 99)),
        "max": float(arr.max()),
        "mean": float(arr.mean()),
    }


def _now_ns() -> int:
    import time
    return time.monotonic_ns()


# ---------------------------------------------------------------------------
# Analyzer
# ---------------------------------------------------------------------------


def analyze_results(results_dir: str) -> None:
    """Walk every ``*.json`` under ``results_dir`` and emit a Markdown table."""
    root = pathlib.Path(results_dir)
    if not root.exists():
        console.print(f"[red]no results directory at {root}[/red]")
        return
    summaries: list[dict[str, Any]] = []
    for path in sorted(root.glob("*.json")):
        try:
            with open(path) as f:
                summaries.append(json.load(f))
        except json.JSONDecodeError:
            console.print(f"[yellow]skipping malformed {path}[/yellow]")
    if not summaries:
        console.print(f"[yellow]no JSON summaries in {root}[/yellow]")
        return

    out = pathlib.Path(results_dir) / "summary.md"
    lines: list[str] = []
    lines.append(f"# geomqtt benchmark summary\n")
    lines.append(f"_Compiled from {len(summaries)} runs under `{root}`._\n")

    by_scenario: dict[str, list[dict[str, Any]]] = {}
    for s in summaries:
        by_scenario.setdefault(s.get("scenario", "unknown"), []).append(s)

    for scenario, runs in sorted(by_scenario.items()):
        lines.append(f"## `{scenario}`\n")
        if scenario == "write-rate":
            lines.append(
                "| Run | target/s | achieved/s | RTT P50 | P95 | P99 | ceiling /s (P95 budget) |"
            )
            lines.append("|---|---|---|---|---|---|---|")
            for s in runs:
                rtt = s.get("rtt_ms", {})
                ceiling = s.get("ceiling", {})
                lines.append(
                    f"| `{s['run_id']}` | {s.get('target_rate', '—')} | "
                    f"{s.get('achieved_rate', '—'):.0f} | "
                    f"{rtt.get('p50', '—'):.2f} | {rtt.get('p95', '—'):.2f} | "
                    f"{rtt.get('p99', '—'):.2f} | "
                    f"{ceiling.get('rate_at_p95_budget', '—')} |"
                )
        elif scenario == "fanout":
            lines.append(
                "| Run | subscribers | publish rate /s | P50 | P95 | P99 | max |"
            )
            lines.append("|---|---|---|---|---|---|---|")
            for s in runs:
                lat = s.get("fanout_ms", {})
                lines.append(
                    f"| `{s['run_id']}` | {s.get('subscribers', '—')} | "
                    f"{s.get('publish_rate', '—')} | "
                    f"{lat.get('p50', '—'):.2f} | {lat.get('p95', '—'):.2f} | "
                    f"{lat.get('p99', '—'):.2f} | {lat.get('max', '—'):.2f} |"
                )
        elif scenario == "snapshot-burst":
            lines.append(
                "| Run | subscribers | viewport tiles | objects | first-msg P95 | burst-end P95 |"
            )
            lines.append("|---|---|---|---|---|---|")
            for s in runs:
                first = s.get("first_message_ms", {})
                end = s.get("burst_end_ms", {})
                lines.append(
                    f"| `{s['run_id']}` | {s.get('subscribers', '—')} | "
                    f"{s.get('viewport_tiles', '—')} | "
                    f"{s.get('object_count', '—')} | "
                    f"{first.get('p95', '—'):.2f} | {end.get('p95', '—'):.2f} |"
                )
        elif scenario == "viewport-churn":
            lines.append(
                "| Run | clients | churn interval (ms) | sub+unsub /s sustained | steady-state subs |"
            )
            lines.append("|---|---|---|---|---|")
            for s in runs:
                lines.append(
                    f"| `{s['run_id']}` | {s.get('clients', '—')} | "
                    f"{s.get('churn_interval_ms', '—')} | "
                    f"{s.get('churn_rate_sustained', '—'):.0f} | "
                    f"{s.get('steady_state_subs', '—')} |"
                )
        elif scenario == "cross-node-fanout":
            lines.append(
                "| Run | subs | nodes | local P50 / P95 / P99 | cross P50 / P95 / P99 | bridge overhead P50 |"
            )
            lines.append("|---|---|---|---|---|---|")
            for s in runs:
                local = s.get("local_fanout_ms", {})
                cross = s.get("cross_fanout_ms", {})
                bridge = s.get("bridge_overhead_ms_p50", None)
                nodes = s.get("subscriber_nodes", [])
                bridge_str = f"{bridge:.2f}" if bridge is not None else "—"
                lines.append(
                    f"| `{s['run_id']}` | {s.get('subscribers', '—')} | "
                    f"{len(set(nodes))} | "
                    f"{local.get('p50', '—'):.2f} / {local.get('p95', '—'):.2f} / {local.get('p99', '—'):.2f} | "
                    f"{cross.get('p50', '—'):.2f} / {cross.get('p95', '—'):.2f} / {cross.get('p99', '—'):.2f} | "
                    f"{bridge_str} |"
                )
        else:
            lines.append(f"_Unknown scenario; raw runs:_ {len(runs)}")
        lines.append("")

    out.write_text("\n".join(lines))
    console.print(f"[green]wrote {out}[/green]")
    console.print("\n".join(lines))
