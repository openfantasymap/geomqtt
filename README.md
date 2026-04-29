<div align="center">

# geomqtt

**A Redis-compatible proxy + embedded MQTT broker that turns Redis GEO sets
into a live, tile-keyed topic tree — so a web or game client can follow a
moving viewport by subscribing to the tiles it can see.**

[![ci](https://github.com/openfantasymap/geomqtt/actions/workflows/ci.yml/badge.svg?branch=main)](https://github.com/openfantasymap/geomqtt/actions/workflows/ci.yml)
[![tests](https://github.com/openfantasymap/geomqtt/actions/workflows/tests.yml/badge.svg?branch=main)](https://github.com/openfantasymap/geomqtt/actions/workflows/tests.yml)
[![release](https://github.com/openfantasymap/geomqtt/actions/workflows/release.yml/badge.svg)](https://github.com/openfantasymap/geomqtt/actions/workflows/release.yml)
[![npm](https://github.com/openfantasymap/geomqtt/actions/workflows/npm.yml/badge.svg)](https://github.com/openfantasymap/geomqtt/actions/workflows/npm.yml)
[![pages](https://github.com/openfantasymap/geomqtt/actions/workflows/pages.yml/badge.svg)](https://openfantasymap.github.io/geomqtt/)
[![ghcr](https://img.shields.io/github/v/release/openfantasymap/geomqtt?logo=docker&label=ghcr.io%2Fgeomqtt&color=2b3137)](https://github.com/openfantasymap/geomqtt/pkgs/container/geomqtt)
[![demo](https://img.shields.io/badge/live%20demo-openfantasymap.github.io-ff5722?logo=maplibre)](https://openfantasymap.github.io/geomqtt/?url=wss://geomqtt.fantasymaps.org/mqtt&set=iss)
[![license](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue)](#license)

[Quick start](#-quick-start) · [Architecture](#-architecture) · [Clients](#-clients) · [Observability](#-observability) · [Protocol](./PROTOCOL.md) · [Roadmap](#-roadmap)

</div>

---

## What it does

- **Proxies Redis (RESP).** Every standard command forwards to an upstream Redis; `GEOADD` / `ZREM` / `HSET` / `HDEL` / `DEL` on `obj:*` keys are intercepted to trigger MQTT fanout.
- **Embeds an MQTT broker.** QoS 0, clean session, over both raw TCP (`1883`) and WebSocket (`8083`). Browser clients just connect over WS — no extra bridge.
- **Projects GEO sets onto slippy-map tiles.** Topic tree is `geo/<set>/<z>/<x>/<y>`; a map viewport is literally a set of tile subscriptions.
- **Serves snapshots on subscribe.** Each new subscriber gets the current tile contents as a per-session burst (`GEOSEARCH`) followed by the live stream.
- **Exposes GeoJSON over HTTP.** `/tiles/<set>/<z>/<x>/<y>`, `/viewport/<set>?bbox=…`, `/objects/<obid>` for non-live callers.
- **Reports cheap Prometheus metrics.** `/status` emits in-process counters (sessions, tile fanouts, RESP commands) plus `process_resident_memory_bytes`. Every increment is a single atomic; rendering walks atomics + one `/proc/self/status` read.
- **Mirrors writes to InfluxDB (optional).** When `GEOMQTT_INFLUX_URL` is set, intercepted `GEOADD` positions and `HSET` attribute writes also flow as line-protocol points via a bounded mpsc + batching task — fire-and-forget on the hot path, zero-cost when disabled.
- **Scales horizontally.** Cross-node fanout rides on Redis pub/sub with a node-id envelope so nodes don't echo their own publishes.
- **Ships five clients.** TypeScript (Leaflet, MapLibre, core), Unity UPM, Unreal plugin.
- **Has a live demo.** [`openfantasymap.github.io/geomqtt`](https://openfantasymap.github.io/geomqtt/?url=wss://geomqtt.fantasymaps.org/mqtt&set=iss) shows a public geomqtt instance tracking the ISS, with the active MQTT subscription set rolling in the corner.

## 📐 Architecture

```
                ┌─────────────────────────────────┐
 writers ──RESP─┤                                 ├──▶ Upstream Redis
                │     geomqtt-server (Rust)       │    (GEO sets, obj:* hashes)
 browsers ──WS──┤  ┌─────┬──────┬──────┬───────┐  │
 native  ──TCP──┤  │RESP │ MQTT │ HTTP │ Redis │◀─┼──▶ Cross-node pub/sub
 scrapers──HTTP─┤  │proxy│broker│GeoJSN│coord. │  │    (node-id envelope)
                │  └─────┴──────┴──────┴───────┘  │
                └─────────────────────────────────┘
```

A single Rust binary hosts four listeners (RESP, MQTT/TCP, MQTT/WS, HTTP) and
a background bridge that relays cross-node publishes.

## 🚀 Quick start

```sh
docker compose up --build
```

Then, in another shell:

```sh
# write a point through the proxy
redis-cli -p 6380 GEOADD vehicles 11.34 44.49 veh-42
redis-cli -p 6380 HSET  obj:veh-42 icon truck color red

# read that tile as GeoJSON
curl http://localhost:8080/tiles/vehicles/10/544/370

# discover server tile-size + effective zooms
curl http://localhost:8080/config

# Prometheus-format counters (uptime, sessions, fanout volumes, …)
curl http://localhost:8080/status

# subscribe to live updates (raw TCP)
mosquitto_sub -h localhost -p 1883 -t 'geo/vehicles/10/544/370'
```

The compose file also runs [`examples/iss-demo`](examples/iss-demo/), a tiny
sidecar that polls the open-notify ISS position endpoint every 5 s and feeds
it into geomqtt — handy for watching live tile traffic without any local
data:

```sh
mosquitto_sub -h localhost -p 1883 -t 'geo/iss/#' -v
curl http://localhost:8080/objects/iss
```

A matching static web UI lives in [`examples/web-iss`](examples/web-iss/) —
a MapLibre page that subscribes to the `iss` set and is published to
GitHub Pages by `.github/workflows/pages.yml`. The bottom-left panel
shows the active MQTT subscription set (and a rolling sub/unsub log) so
you can watch the tile-keyed protocol in action as you pan the map.

[**▶ Open the live demo**](https://openfantasymap.github.io/geomqtt/?url=wss://geomqtt.fantasymaps.org/mqtt&set=iss)
— points at a public deployment at `wss://geomqtt.fantasymaps.org/mqtt`.
Override `?url=wss://your-host:8083` to point it at your own.

## 📦 Install

Pick the channel that matches how you're going to run or talk to geomqtt:

| Channel                   | Address                                                                      |
|---------------------------|------------------------------------------------------------------------------|
| **Docker (multi-arch)**   | `docker pull ghcr.io/openfantasymap/geomqtt:latest`                          |
| **Docker — ISS demo**     | `docker pull ghcr.io/openfantasymap/geomqtt-iss-demo:latest`                 |
| **Binaries**              | [GitHub Releases](https://github.com/openfantasymap/geomqtt/releases) — Linux / macOS / Windows, x86_64 + aarch64 |
| **npm — core library**    | `npm install @openfantasymap/geomqtt-core` *(published to GitHub Packages by [npm.yml](.github/workflows/npm.yml) on tag push or manual dispatch — see install note below)* |
| **npm — Leaflet adapter** | `npm install @openfantasymap/geomqtt-leaflet`                                |
| **npm — MapLibre adapter**| `npm install @openfantasymap/geomqtt-maplibre`                               |
| **Unity (UPM)**           | Add `https://github.com/openfantasymap/geomqtt.git#upm/v0.1.0` to `manifest.json` |
| **Unreal (UE 5.3+)**      | Drop [`clients/geomqtt-unreal/`](./clients/geomqtt-unreal) into your project's `Plugins/Geomqtt/` and rebuild |

> **GitHub Packages install note.** The npm packages live on GitHub Packages,
> which requires authentication even for public packages. Add a `.npmrc` at
> your project root (or `~/.npmrc`):
>
> ```ini
> @openfantasymap:registry=https://npm.pkg.github.com
> //npm.pkg.github.com/:_authToken=YOUR_GITHUB_PAT
> ```
>
> Any GitHub personal access token with the `read:packages` scope works.

## ⚙️ Configuration

All config is via environment variables:

| Variable                      | Default                   | Purpose                                           |
|-------------------------------|---------------------------|---------------------------------------------------|
| `GEOMQTT_REDIS_URL`           | `redis://127.0.0.1:6379`  | Upstream Redis connection string                  |
| `GEOMQTT_RESP_ADDR`           | `0.0.0.0:6380`            | RESP (Redis-compatible) listener                  |
| `GEOMQTT_MQTT_ADDR`           | `0.0.0.0:1883`            | MQTT TCP listener                                 |
| `GEOMQTT_MQTT_WS_ADDR`        | `0.0.0.0:8083`            | MQTT WebSocket listener (browser clients)         |
| `GEOMQTT_HTTP_ADDR`           | `0.0.0.0:8080`            | HTTP listener (GeoJSON + `/healthz` + `/config`)  |
| `GEOMQTT_ENRICH_ATTRS`        | *(empty)*                 | CSV of attribute keys embedded in tile payloads   |
| `GEOMQTT_ENRICH_ZOOMS`        | `6-12`                    | Zoom levels that receive tile-topic publishes. Accepts ranges (`6-12`) and mixed lists (`4,6-10,14`) |
| `GEOMQTT_TILE_SIZE`           | `256`                     | Tile edge in pixels, power of 2 in `1..=256`. Smaller = finer granularity (`128` doubles tile count per zoom) |
| `GEOMQTT_OBJECT_KEY_PREFIX`   | `obj:`                    | Prefix for the per-object attribute hash          |
| `GEOMQTT_INFLUX_URL`          | *(empty)*                 | InfluxDB 2.x base URL (e.g. `https://influx.example.com`). When unset, the sink is disabled and zero-cost. |
| `GEOMQTT_INFLUX_TOKEN`        | *(empty)*                 | Influx API token (used as `Authorization: Token <…>`) |
| `GEOMQTT_INFLUX_ORG`          | *(empty)*                 | Influx organization name                          |
| `GEOMQTT_INFLUX_BUCKET`       | *(empty)*                 | Influx bucket name                                |
| `RUST_LOG`                    | `info`                    | `tracing-subscriber` filter                       |

`GEOMQTT_TILE_SIZE` shifts every configured zoom upward by
`log2(256 / tile_size)`. For example, `GEOMQTT_ENRICH_ZOOMS=6-12,
GEOMQTT_TILE_SIZE=128` publishes on effective zooms `7-13`. The effective
list is returned by `GET /config` so clients can mirror it.

### Optional InfluxDB sink

If `GEOMQTT_INFLUX_URL` is set, every intercepted geo write also gets
mirrored as a time-series point. A bounded mpsc queue (4096 entries) and
a background batching task isolate Influx latency from the RESP hot path
— `try_send` drops on overflow and bumps `geomqtt_influx_writes_dropped_total`
rather than blocking. Two measurements are written, both via
`/api/v2/write` line protocol:

```text
geomqtt_position,set=<set>,obid=<member> lat=<lat>,lon=<lon>
geomqtt_attr,obid=<obid> <key>="<value>"[,<key>="<value>"…]
```

Position points come from `GEOADD`, attribute points from `HSET`/`HMSET`
on `obj:*` keys. Timestamps are assigned server-side by Influx.

## 📊 Observability

`GET /status` returns Prometheus-format text. Counters increment at hot
paths via `AtomicU64::fetch_add(Relaxed)`; gauges resolve from the broker
session table or one `/proc/self/status` read. No background ticker, no
allocator hooks, no Redis round-trips on a scrape.

```text
geomqtt_build_info{version="0.4.0",node_id="…"} 1
geomqtt_uptime_seconds 1234.5
geomqtt_mqtt_sessions 4
geomqtt_mqtt_subscriptions 71
geomqtt_mqtt_connections_total{transport="ws"} 42
geomqtt_mqtt_packets_received_total 318
geomqtt_resp_commands_total 1024
geomqtt_resp_geo_writes_total 220
geomqtt_tile_fanouts_total 1540
geomqtt_object_fanouts_total 12
geomqtt_redis_bridge_messages_total 87
geomqtt_http_requests_total 60
geomqtt_influx_writes_enqueued_total 220
geomqtt_influx_writes_dropped_total 0
geomqtt_influx_batches_sent_total 18
geomqtt_influx_batch_errors_total 0
process_resident_memory_bytes 12345678
process_virtual_memory_bytes  2861531136
process_resident_memory_max_bytes 14000000
```

Point a Prometheus scrape config at `http://<host>:8080/status` (use the
canonical metrics path of your scraper — geomqtt does not enforce the
`/metrics` convention so it can keep `/healthz` / `/config` /
`/tiles` / `/viewport` / `/objects` cohesive on `:8080`).

## 🧭 Clients

Four client packages under [`clients/`](./clients). Same protocol,
different runtimes:

<table>
  <tr>
    <td><b><a href="./clients/geomqtt-core">@openfantasymap/geomqtt-core</a></b></td>
    <td>TypeScript. MQTT transport, tile math, viewport-diff subscribe loop, state map. Runs in Node and the browser.</td>
  </tr>
  <tr>
    <td><b><a href="./clients/geomqtt-leaflet">@openfantasymap/geomqtt-leaflet</a></b></td>
    <td><code>L.LayerGroup</code> adapter. Wires <code>moveend</code>/<code>zoomend</code> to the core client; default <code>L.circleMarker</code> rendering with a <code>markerFor</code> hook.</td>
  </tr>
  <tr>
    <td><b><a href="./clients/geomqtt-maplibre">@openfantasymap/geomqtt-maplibre</a></b></td>
    <td>MapLibre / Mapbox GL adapter. Keeps a GeoJSON source fed from the current state; debounced via <code>updateThrottleMs</code>.</td>
  </tr>
  <tr>
    <td><b><a href="./clients/geomqtt-unity">com.geomqtt.unity</a></b></td>
    <td>Unity UPM package. Pure-C# <code>GeomqttClient</code> plus a <code>GeomqttWorld3D</code> MonoBehaviour that projects lat/lng → local ENU meters for 3D world-anchored scenes. 2D overlay also supported.</td>
  </tr>
  <tr>
    <td><b><a href="./clients/geomqtt-unreal">geomqtt (UE 5.3+)</a></b></td>
    <td>Unreal Engine plugin. Built-in MQTT v3.1.1 codec over UE's <code>WebSockets</code> module — no third-party MQTT plugin required. <code>UGeomqttClient</code> for protocol logic, <code>AGeomqttMarkerSpawner</code> for drag-and-drop 3D world-anchored scenes, <code>UGeomqttSubsystem</code> for Blueprint access.</td>
  </tr>
</table>

Build the TypeScript workspace:

```sh
cd clients
npm install
npm run build
```

## 🗂 Repository layout

```
.
├── Cargo.toml                  # workspace root
├── crates/
│   └── geomqtt-server/         # the single Rust binary
│       └── src/
│           ├── main.rs         # orchestration, signal handling
│           ├── config.rs       # GEOMQTT_* env var parsing + zoom-range syntax
│           ├── coord.rs        # slippy-map XYZ tile math
│           ├── resp.rs         # RESP proxy with command interception
│           ├── mqtt.rs         # embedded MQTT broker (TCP + WS)
│           ├── broker.rs       # in-memory session registry + fanout
│           ├── fanout.rs       # point → tile events (add/move/remove)
│           ├── http.rs         # axum router: GeoJSON + /config + /healthz + /status
│           ├── metrics.rs      # AtomicU64 counters + Prometheus text rendering
│           ├── influx.rs       # optional InfluxDB 2.x sink (bounded mpsc + batching)
│           ├── payload.rs      # JSON payloads (mirrored in client packages)
│           └── redis.rs        # fred client + cross-node pub/sub bridge
├── clients/
│   ├── geomqtt-core/           # @openfantasymap/geomqtt-core — TypeScript
│   ├── geomqtt-leaflet/        # @openfantasymap/geomqtt-leaflet
│   ├── geomqtt-maplibre/       # @openfantasymap/geomqtt-maplibre
│   ├── geomqtt-unity/          # com.geomqtt.unity (UPM)
│   └── geomqtt-unreal/         # Unreal Engine plugin (UE 5.3+, built-in MQTT codec over WS)
├── examples/
│   ├── iss-demo/               # alpine sidecar polling api.open-notify.org → RESP
│   └── web-iss/                # static MapLibre demo (esbuild → GitHub Pages)
├── .github/workflows/
│   ├── ci.yml                  # Rust fmt + clippy, TS build + typecheck
│   ├── tests.yml               # Rust unit + integration (Redis service), TS vitest
│   ├── npm.yml                 # Publishes @openfantasymap/geomqtt-* to GH Packages
│   ├── release.yml             # Binaries + Docker (geomqtt + geomqtt-iss-demo) + UPM split
│   └── pages.yml               # Builds + deploys the web demo to GitHub Pages
├── Dockerfile
├── docker-compose.yml
├── PROTOCOL.md                 # wire contract
└── CLAUDE.md                   # developer context
```

## 🛣 Roadmap

- [x] Workspace scaffold, four listeners binding their ports
- [x] Env-driven configuration (with range + tile-size syntax)
- [x] Protocol spec ([`PROTOCOL.md`](./PROTOCOL.md))
- [x] RESP parsing + upstream forwarding via `fred::custom_raw`
- [x] GEOADD / ZREM / HSET / HDEL / DEL interception and MQTT fanout
- [x] Embedded MQTT broker (hand-rolled on `mqttbytes` — QoS 0, clean session)
- [x] Per-subscriber snapshot burst on SUBSCRIBE
- [x] Cross-node Redis pub/sub with node-id envelope
- [x] HTTP GeoJSON endpoints + `/config`
- [x] `@openfantasymap/geomqtt-{core,leaflet,maplibre}` TS packages on GH Packages
- [x] Unity UPM package with `GeomqttClient` + `GeomqttWorld3D`
- [x] Unreal Engine plugin (`UGeomqttClient` + `AGeomqttMarkerSpawner`, built-in MQTT codec)
- [x] CI + release automation (binaries, Docker, npm, UPM, GitHub Pages)
- [x] `/status` Prometheus endpoint + process memory metrics
- [x] ISS demo (`examples/iss-demo`) and static MapLibre web demo (`examples/web-iss`) with live subscription panel
- [x] CORS on the HTTP API — `fetchServerConfig()` works cross-origin
- [x] Optional InfluxDB 2.x sink for `GEOADD` positions + `HSET` attribute writes
- [ ] Tile-side `attr` fanout (attribute-only updates also reach tile topics)
- [ ] Lua-scripted atomic GEOADD + old-pos capture
- [ ] `SPUBLISH` / `SSUBSCRIBE` for Redis Cluster sharded pub/sub

## 📄 License

Dual-licensed under either:

- MIT license ([`LICENSE-MIT`](./LICENSE-MIT) or <http://opensource.org/licenses/MIT>)
- Apache License, Version 2.0 ([`LICENSE-APACHE`](./LICENSE-APACHE) or <http://www.apache.org/licenses/LICENSE-2.0>)

at your option.
