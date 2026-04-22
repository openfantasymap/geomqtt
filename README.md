<div align="center">

# geomqtt

**A Redis-compatible proxy + embedded MQTT broker that turns Redis GEO sets
into a live, tile-keyed topic tree — so a web or game client can follow a
moving viewport by subscribing to the tiles it can see.**

[![ci](https://github.com/openfantasymap/geomqtt/actions/workflows/ci.yml/badge.svg)](https://github.com/openfantasymap/geomqtt/actions/workflows/ci.yml)
[![tests](https://github.com/openfantasymap/geomqtt/actions/workflows/tests.yml/badge.svg)](https://github.com/openfantasymap/geomqtt/actions/workflows/tests.yml)
[![release](https://github.com/openfantasymap/geomqtt/actions/workflows/release.yml/badge.svg)](https://github.com/openfantasymap/geomqtt/actions/workflows/release.yml)
[![ghcr](https://img.shields.io/badge/ghcr.io-openfantasymap%2Fgeomqtt-2b3137?logo=docker)](https://github.com/openfantasymap/geomqtt/pkgs/container/geomqtt)
[![npm core](https://img.shields.io/npm/v/%40geomqtt%2Fcore?label=%40geomqtt%2Fcore&logo=npm)](https://www.npmjs.com/package/@geomqtt/core)
[![license](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue)](#license)

[Quick start](#-quick-start) · [Architecture](#-architecture) · [Clients](#-clients) · [Protocol](./PROTOCOL.md) · [Roadmap](#-roadmap)

</div>

---

## What it does

- **Proxies Redis (RESP).** Every standard command forwards to an upstream Redis; `GEOADD` / `ZREM` / `HSET` / `HDEL` / `DEL` on `obj:*` keys are intercepted to trigger MQTT fanout.
- **Embeds an MQTT broker.** QoS 0, clean session, over both raw TCP (`1883`) and WebSocket (`8083`). Browser clients just connect over WS — no extra bridge.
- **Projects GEO sets onto slippy-map tiles.** Topic tree is `geo/<set>/<z>/<x>/<y>`; a map viewport is literally a set of tile subscriptions.
- **Serves snapshots on subscribe.** Each new subscriber gets the current tile contents as a per-session burst (`GEOSEARCH`) followed by the live stream.
- **Exposes GeoJSON over HTTP.** `/tiles/<set>/<z>/<x>/<y>`, `/viewport/<set>?bbox=…`, `/objects/<obid>` for non-live callers.
- **Scales horizontally.** Cross-node fanout rides on Redis pub/sub with a node-id envelope so nodes don't echo their own publishes.
- **Ships four clients.** TypeScript for Leaflet and MapLibre, plus a Unity UPM package.

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

# subscribe to live updates (raw TCP)
mosquitto_sub -h localhost -p 1883 -t 'geo/vehicles/10/544/370'
```

## 📦 Install

Pick the channel that matches how you're going to run or talk to geomqtt:

| Channel                   | Address                                                                      |
|---------------------------|------------------------------------------------------------------------------|
| **Docker (multi-arch)**   | `docker pull ghcr.io/openfantasymap/geomqtt:latest`                          |
| **Binaries**              | [GitHub Releases](https://github.com/openfantasymap/geomqtt/releases) — Linux / macOS / Windows, x86_64 + aarch64 |
| **npm — core library**    | `npm install @geomqtt/core`                                                  |
| **npm — Leaflet adapter** | `npm install @geomqtt/leaflet`                                               |
| **npm — MapLibre adapter**| `npm install @geomqtt/maplibre`                                              |
| **Unity (UPM)**           | Add `https://github.com/openfantasymap/geomqtt.git#upm/v0.1.0` to `manifest.json` |

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
| `RUST_LOG`                    | `info`                    | `tracing-subscriber` filter                       |

`GEOMQTT_TILE_SIZE` shifts every configured zoom upward by
`log2(256 / tile_size)`. For example, `GEOMQTT_ENRICH_ZOOMS=6-12,
GEOMQTT_TILE_SIZE=128` publishes on effective zooms `7-13`. The effective
list is returned by `GET /config` so clients can mirror it.

## 🧭 Clients

Four client packages under [`clients/`](./clients). Same protocol,
different runtimes:

<table>
  <tr>
    <td><b><a href="./clients/geomqtt-core">@geomqtt/core</a></b></td>
    <td>TypeScript. MQTT transport, tile math, viewport-diff subscribe loop, state map. Runs in Node and the browser.</td>
  </tr>
  <tr>
    <td><b><a href="./clients/geomqtt-leaflet">@geomqtt/leaflet</a></b></td>
    <td><code>L.LayerGroup</code> adapter. Wires <code>moveend</code>/<code>zoomend</code> to the core client; default <code>L.circleMarker</code> rendering with a <code>markerFor</code> hook.</td>
  </tr>
  <tr>
    <td><b><a href="./clients/geomqtt-maplibre">@geomqtt/maplibre</a></b></td>
    <td>MapLibre / Mapbox GL adapter. Keeps a GeoJSON source fed from the current state; debounced via <code>updateThrottleMs</code>.</td>
  </tr>
  <tr>
    <td><b><a href="./clients/geomqtt-unity">com.geomqtt.unity</a></b></td>
    <td>Unity UPM package. Pure-C# <code>GeomqttClient</code> plus a <code>GeomqttWorld3D</code> MonoBehaviour that projects lat/lng → local ENU meters for 3D world-anchored scenes. 2D overlay also supported.</td>
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
│           ├── http.rs         # axum router for GeoJSON + /config + /healthz
│           ├── payload.rs      # JSON payloads (mirrored in client packages)
│           └── redis.rs        # fred client + cross-node pub/sub bridge
├── clients/
│   ├── geomqtt-core/           # @geomqtt/core — TypeScript
│   ├── geomqtt-leaflet/        # @geomqtt/leaflet
│   ├── geomqtt-maplibre/       # @geomqtt/maplibre
│   └── geomqtt-unity/          # com.geomqtt.unity (UPM)
├── .github/workflows/
│   ├── ci.yml                  # Rust fmt + clippy, TS build + typecheck
│   ├── tests.yml               # Rust unit + integration (Redis service), TS vitest
│   └── release.yml             # binaries + Docker + npm + UPM branch
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
- [x] `@geomqtt/core`, `@geomqtt/leaflet`, `@geomqtt/maplibre` TS packages
- [x] Unity UPM package with `GeomqttClient` + `GeomqttWorld3D`
- [x] CI + release automation (binaries, Docker, npm, UPM)
- [ ] Tile-side `attr` fanout (attribute-only updates also reach tile topics)
- [ ] Lua-scripted atomic GEOADD + old-pos capture
- [ ] `SPUBLISH` / `SSUBSCRIBE` for Redis Cluster sharded pub/sub

## 📄 License

Dual-licensed under either:

- MIT license ([`LICENSE-MIT`](./LICENSE-MIT) or <http://opensource.org/licenses/MIT>)
- Apache License, Version 2.0 ([`LICENSE-APACHE`](./LICENSE-APACHE) or <http://www.apache.org/licenses/LICENSE-2.0>)

at your option.
