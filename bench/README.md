# geomqtt — scaling benchmark

A reproducible benchmark harness for measuring the operational ceilings
of a geomqtt deployment. Used to back the quantitative-scaling claims of
the GeoMQTT paper (Paper F in the broader research programme); also useful
locally to characterise a node's capacity before deployment.

The harness ships as a Docker container so a run is fully reproducible
from a clean checkout: nothing needs to be installed on the host beyond
Docker and Docker Compose.

## What it measures

Four scenarios, each isolating a different scaling axis:

| Scenario          | Stresses                              | Primary metric                       |
|-------------------|---------------------------------------|--------------------------------------|
| `write-rate`      | RESP + Lua atomic GEOADD + tile fanout | Max sustained `GEOADD/s` before RTT P95 exceeds a threshold |
| `fanout`          | MQTT delivery to N concurrent subscribers per tile | Per-publish fan-out latency P50 / P95 / P99 |
| `snapshot-burst`  | GEOSEARCH + HGETALL per-subscriber on SUBSCRIBE | Subscribe → first-event latency P50 / P95 / P99 |
| `viewport-churn`  | Realistic web/game pan-and-zoom subscription churn | Sub + unsub rate sustained without drop |

Every scenario emits `results/<scenario>-<timestamp>.csv` (per-event timing)
and `results/<scenario>-<timestamp>.json` (summary). The `analyze`
subcommand walks `results/` and emits a Markdown table for the paper.

## Quick start (Docker, recommended)

```sh
# From the repo root, bring up Redis + geomqtt.
docker compose -f bench/docker-compose.bench.yml up -d redis geomqtt

# Wait for healthchecks to pass (about 5 seconds).
docker compose -f bench/docker-compose.bench.yml ps

# Run the smoke scenario (~30 seconds; sanity check).
docker compose -f bench/docker-compose.bench.yml run --rm bench smoke

# Or run the full sweep (~10 minutes).
docker compose -f bench/docker-compose.bench.yml run --rm bench all

# Analyse all accumulated runs into bench/results/summary.md.
docker compose -f bench/docker-compose.bench.yml run --rm bench analyze

# Tear down when done.
docker compose -f bench/docker-compose.bench.yml down
```

Results land in `bench/results/` on the host via a volume mount, so they
survive `docker compose down`.

### Targeting a remote geomqtt

By default the bench container talks to the `geomqtt` service inside the
compose network. To benchmark a geomqtt deployment running elsewhere:

```sh
docker compose -f bench/docker-compose.bench.yml run --rm \
  -e GEOMQTT_BENCH_RESP_HOST=geomqtt.example.com:6380 \
  -e GEOMQTT_BENCH_MQTT_HOST=geomqtt.example.com:1883 \
  -e GEOMQTT_BENCH_HTTP_HOST=geomqtt.example.com:8080 \
  bench all
```

(You can also `docker compose up -d redis` only, and skip the in-network
geomqtt — but then nothing answers on the default `geomqtt:*` hosts, so
either override or run the harness against a different deployment.)

### Single-scenario invocation

```sh
docker compose -f bench/docker-compose.bench.yml run --rm bench \
  write-rate --duration 120 --target-rate 5000 --rate-ramp exponential

docker compose -f bench/docker-compose.bench.yml run --rm bench \
  fanout --subscribers 500 --publish-rate 100

docker compose -f bench/docker-compose.bench.yml run --rm bench \
  snapshot-burst --subscribers 200 --viewport-tiles 16 --object-count 50000

docker compose -f bench/docker-compose.bench.yml run --rm bench \
  viewport-churn --clients 500 --churn-interval-ms 750
```

## Quick start (bare-metal Python, for development)

When iterating on the harness itself it's faster to run against a local
geomqtt:

```sh
# Have geomqtt running somewhere (e.g., docker compose up the demo stack
# from the repo root).
docker compose up -d redis geomqtt

# Install the harness dependencies once.
python -m venv .venv && . .venv/bin/activate
pip install -r bench/requirements.txt

# Run scenarios with the bare-metal entry point.
python -m bench smoke
python -m bench write-rate --duration 30 --target-rate 1000
python -m bench analyze
```

Bare-metal runs target `127.0.0.1:*` by default; if you want them to
target an in-cluster geomqtt instead, set `GEOMQTT_BENCH_*_HOST` env vars
as with the docker run.

## Configuration

| Env var (Docker)                  | Default              | Purpose                                        |
|-----------------------------------|----------------------|------------------------------------------------|
| `GEOMQTT_BENCH_RESP_HOST`         | `geomqtt:6380`       | geomqtt RESP listener                          |
| `GEOMQTT_BENCH_MQTT_HOST`         | `geomqtt:1883`       | geomqtt MQTT TCP listener                      |
| `GEOMQTT_BENCH_MQTT_WS_HOST`      | `geomqtt:8083`       | geomqtt MQTT WebSocket listener                |
| `GEOMQTT_BENCH_HTTP_HOST`         | `geomqtt:8080`       | geomqtt HTTP listener (`/config` + `/status`)  |
| `GEOMQTT_BENCH_RESULTS_DIR`       | `/results`           | Where the harness writes output                |

CLI flag overrides win over env-var defaults. See `python -m bench <cmd>
--help` for the full per-scenario flag set.

## What each scenario actually does

### `write-rate`

A pool of asyncio writer tasks issue `GEOADD <set> <lon> <lat> <obid>` at
a target rate. Object ids are drawn from a pool of `--object-count`, with
positions perturbed so each write is a real position update. The harness
measures per-`GEOADD` RTT and the actual achieved rate. When `--rate-ramp
linear|exponential` is set, the target rate increases over the duration;
the run terminates when the 1-second-window RTT P95 first exceeds
`--p95-budget-ms`, and the rate at the last good window is reported as
the **write-rate ceiling**.

### `fanout`

A single writer issues `GEOADD` at a fixed rate within a small geographic
area that maps to a single tile at the configured zoom. `--subscribers`
concurrent MQTT sessions are subscribed to that tile. The harness measures
the per-publish delay from write completion to receipt at each subscriber.
The P50 / P95 / P99 across all subscriber × publish events is the
**fan-out latency**.

### `snapshot-burst`

The harness pre-loads `--object-count` points into the test set,
distributed across a viewport-sized tile cluster. Then it opens
`--subscribers` concurrent MQTT sessions, each subscribing to a viewport
(`--viewport-tiles` tiles). For each session it measures (a) the
SUBSCRIBE → first-snapshot-message latency and (b) the
SUBSCRIBE → end-of-burst latency (heuristic: 500 ms of silence after the
most recent message). The P50 / P95 / P99 of both are the **snapshot-burst
latency**.

### `viewport-churn`

Models a web-map client panning and zooming. Each simulated client
maintains an active subscription set; every `--churn-interval-ms` it
picks a random direction (N/S/E/W/zoom-in/zoom-out) and emits the
subscribe/unsubscribe diffs corresponding to a one-tile hop. The harness
ramps client count linearly over the duration and reports the rate at
which sub+unsub ops are completed without drop, plus the steady-state
total subscription count. The **churn-rate ceiling** is reported.

## Output format

Each scenario emits two files into the results directory:

```
results/write-rate-20260512T103200Z.csv
results/write-rate-20260512T103200Z.json
```

The CSV is per-event:

```csv
ts_ms,event,latency_ms,warmup,extra
0001234,geoadd,1.34,0,{"set":"vehicles","obid":"obj-0042"}
0001235,geoadd,2.01,0,{"set":"vehicles","obid":"obj-0043"}
...
```

The JSON is the summary, with per-run config + a snapshot of geomqtt's
`/config` (zoom band, tile size, enrich attrs) and `/status` (Prometheus
counters at run end), making each run interpretable from the JSON alone:

```json
{
  "scenario":       "write-rate",
  "run_id":         "write-rate-20260512T103200Z",
  "duration":       60,
  "warmup":         5,
  "config":         { "resp_host": "geomqtt:6380", ... },
  "geomqtt_config": { "tileSize": 256, "zooms": [6,7,8,9,10,11,12], ... },
  "geomqtt_status_post": { "geomqtt_resp_geo_writes_total": "12345", ... },
  "achieved_rate":  4823.4,
  "target_rate":    5000,
  "rtt_ms":         { "p50": 1.2, "p95": 8.4, "p99": 19.1, "max": 78.0, "mean": 2.1, "n": 289400 },
  "ceiling":        { "rate_at_p95_budget": 4500.0, "p95_budget_ms": 100.0 },
  "errors":         0
}
```

## Reproducibility notes

- The geomqtt image is pinned at `ghcr.io/openfantasymap/geomqtt:latest`
  by default in `docker-compose.bench.yml`. **Bump that tag** to a
  specific version when running benchmarks for a paper; record the tag
  in your benchmark log.
- The bench image is built locally from `bench/Dockerfile`, so the
  harness version always matches the source you have checked out.
- The full effective geomqtt config (tile size, zoom band, enrich attrs)
  is captured in every run's JSON summary, so a run is interpretable
  from the file alone.

## Limitations

- Single-node only. Cross-node scaling (multiple geomqtt instances against
  one Redis) is left for a follow-up compose file.
- The harness and geomqtt are on the same Docker network, which removes
  realistic network RTT. Distributed runs require infrastructure beyond
  one compose file.
- The harness does not exercise the optional InfluxDB sink.
- `snapshot-burst` assumes `--object-count` is large enough that the
  snapshot is non-trivial; for tiny tiles you may need to drop the
  500 ms silence threshold by editing `scenarios/snapshot_burst.py`.
