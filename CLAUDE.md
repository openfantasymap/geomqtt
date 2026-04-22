# CLAUDE.md

Context for Claude Code sessions working on this repository.

## What this is

`geomqtt` is a single Rust service that:

1. Presents a **Redis-compatible (RESP)** endpoint that forwards commands to
   an upstream Redis, while intercepting writes on `GEOADD` / `HSET` / `DEL`
   on tracked keys.
2. Runs an **embedded MQTT broker** (TCP + WebSocket) whose topic tree is a
   direct projection of Redis GEO sets onto slippy-map XYZ tiles:
   `geo/<set>/<z>/<x>/<y>`.
3. Serves **GeoJSON over HTTP** for snapshot reads.
4. Scales **horizontally** via Redis sharded pub/sub (`SPUBLISH` /
   `SSUBSCRIBE`) — no in-process registry is the source of truth.

End goal: three clients — Leaflet, MapLibre/Mapbox, Unity — subscribe to the
tiles their viewport covers, receive a per-session snapshot, then live
updates.

## Design decisions (locked)

Each has a one-line "why" so edge cases stay interpretable.

* **Rust, single binary, deployed as a Docker container.**
  *Why:* user preference + operational simplicity for the target audience.
* **Clean session, QoS 0 only.**
  *Why:* a live map viewer redraws on reconnect; persistent sessions in a
  clustered embedded broker is a hard problem not worth solving for v0.1.
* **Horizontal scale via Redis sharded pub/sub, not an in-process registry.**
  *Why:* the user explicitly wants Redis to be the coordination backend.
  `SPUBLISH` with no subscribers is cheap, so the "live-tile registry"
  emerges from Redis subscription state rather than being maintained
  explicitly.
* **Per-subscriber snapshot burst on SUBSCRIBE, not retained messages.**
  *Why:* retained messages would require rewriting the retained payload on
  every `GEOADD`, reintroducing write amplification.
* **Attribute storage = one Redis hash per object** (`<prefix><obid>`).
  *Why:* atomic multi-attr updates, single HGETALL, easy to serialize as
  GeoJSON `properties`.
* **Partial enrichment on tile payloads, full attrs on `objects/<obid>`.**
  *Why:* tile bandwidth matters (many messages on pan/zoom); object-topic
  bandwidth doesn't (one per subscription).
* **Tile publish only at configured zoom levels** (`GEOMQTT_ENRICH_ZOOMS`,
  default `6-12` inclusive, with range syntax accepted).
  *Why:* publishing to every zoom is 19× write amplification per point;
  a contiguous band like 6-12 covers every zoom a typical city-scale map
  uses without writing to irrelevant global or micro scales.
* **Configurable tile pixel size** (`GEOMQTT_TILE_SIZE`, default `256`,
  power of 2 ≤ 256). Shifts every raw zoom up by `log2(256 / tile_size)` so
  "128-pixel tiles" really means "effective zoom = raw + 1" (finer tiles
  = tighter pub/sub fanout at the same nominal zoom). `/config` exposes
  both raw and effective lists so clients can mirror the shift.
* **GEOADD handled by a Lua script** that atomically returns old position +
  new position + enrichment HMGET.
  *Why:* one round-trip, and the "tile-crossing" case (point moves between
  tiles) needs old+new to emit correct `remove`/`add` pairs.

## Module map

```
crates/geomqtt-server/src/
  main.rs     entry: tracing init, config load, spawn four listeners + bridge, ctrl-c
  config.rs   GEOMQTT_* env vars → Config struct. Change here when adding a setting.
  coord.rs    tile_for_coord / bbox_for_tile / tiles_for_point. Pure, unit-tested.
  payload.rs  JSON payload types for tile + object topics; topic + channel helpers.
  broker.rs   In-memory session registry + topic-filter matching + local delivery.
  fanout.rs   "Given a geo write, publish tile/object events" — used by resp.rs.
  resp.rs     RESP2 proxy: parse → intercept GEOADD/ZREM/HSET/HDEL/DEL → forward via
              fred `custom_raw` → re-encode Resp3→Resp2 → write back to client.
  mqtt.rs     MQTT v3.1.1 (QoS 0, clean session). TCP + WebSocket accept loops share
              one session state machine; SUB triggers GEOSEARCH/HGETALL snapshot.
  http.rs     axum router: /healthz, /tiles/<..>, /viewport/<..>, /objects/<..>.
  redis.rs    fred `Client` + `SubscriberClient`; cross-node pub/sub bridge task
              that rebroadcasts Redis messages to the local broker (skipping self).
```

## Current status

**v0.1 working end-to-end** against Redis 7. Verified locally:

* `GEOADD` / `HSET` via RESP proxy (port 6380) → fans out `snapshot` / `add` /
  `move` / `remove` on `geo/<set>/<z>/<x>/<y>` and attribute events on
  `objects/<obid>`.
* MQTT SUBSCRIBE (TCP port 1883, WS port 8083) triggers a per-subscriber
  `snapshot` burst backed by `GEOSEARCH` for tile topics and `HGETALL` for
  object topics.
* `GET /tiles/<set>/<z>/<x>/<y>`, `/viewport/<set>?bbox=w,s,e,n`,
  `/objects/<obid>` return GeoJSON.
* Cross-node pub/sub uses ordinary Redis `PUBLISH`/`PSUBSCRIBE` on channels
  `gmq:tile:*` and `gmq:obj:*`. Every envelope starts with `<node_id>|` so
  each node skips its own echo.

Known gaps:

* **Tile-side `attr` fanout** is not wired yet (marked `#[allow(dead_code)]`
  on `TilePayload::Attr` and `TilePayload::attr()`). v0.2 work: on `HSET`
  of a tracked `obj:*` key, look up which sets the object is in (needs a
  `gmq:inset:<obid>` Redis set updated on GEOADD) and fan out an `attr`
  message to each currently-live tile.
* **No Lua script** for atomic GEOADD + old-position fetch. Current flow
  is two pipelined commands (GEOPOS then GEOADD); race-prone if a single
  member is updated from two clients simultaneously.
* **No sharded pub/sub.** Moving to `SPUBLISH`/`SSUBSCRIBE` is required
  for Redis Cluster deployments (per-channel sharding).

## How to work on this repo

```sh
# local build (needs rust toolchain)
cargo build
cargo test           # coord.rs has unit tests

# containerized
docker compose up --build

# smoke-test listeners are up
curl localhost:8080/healthz                       # → "ok"
nc -zv localhost 6380 1883 8083                    # all bound
```

`cargo update` may be needed after cloning; versions in `Cargo.toml` are
loose on purpose during scaffold.

## Conventions

* **Env var names:** `GEOMQTT_*`, SCREAMING_SNAKE. Defaults live in
  `config.rs`, documented in `README.md`.
* **Logging:** `tracing` with structured fields. Prefer
  `info!(%addr, "bound")` over format strings.
* **Errors:** `anyhow::Result` at the binary boundaries. Replace with
  `thiserror` enums if/when a module stabilizes into a library crate.
* **No multi-paragraph docstrings.** Module-level `//!` blocks state what
  the module does in one short paragraph; the `TODO:` block lists open
  work. Individual functions get one-line doc comments at most.
* **No retained messages.** All state lives in Redis; MQTT is transient.
* **Protocol changes go in `PROTOCOL.md` before code.** The three clients
  read that doc as their contract — changing payloads without updating it
  will drift them.

## Clients

Four packages under `clients/`. TS workspace uses npm workspaces — build
`@geomqtt/core` before the adapters since they resolve it via the `dist/`
emit:

```
cd clients && npm install && npm run build
```

Source layout:

```
clients/
  geomqtt-core/      @geomqtt/core — transport (mqtt.js), tile math, viewport
                     diff, feature state map. No DOM deps. ES module.
  geomqtt-leaflet/   @geomqtt/leaflet — GeomqttLayer (extends L.LayerGroup).
                     Wires moveend/zoomend → setViewport. markerFor hook for
                     custom rendering.
  geomqtt-maplibre/  @geomqtt/maplibre — GeomqttSource. Keeps a GeoJSON source
                     + default circle layer fed from client.snapshot().
                     Debounced via updateThrottleMs (default 33ms).
  geomqtt-unity/     com.geomqtt.unity — UPM package. GeomqttClient is pure C#
                     (MQTTnet 4.x + Newtonsoft.Json); events are queued on a
                     worker thread and drained by GeomqttWorld3D.Update() via
                     client.PumpEvents(). Default MonoBehaviour spawns sphere
                     markers in 3D world space via Geodesy.ToEnu(origin, …).
```

Unity shape: 3D world-anchored is the default; 2D map overlay inside Unity
works by instantiating `GeomqttClient` directly and rendering with sprites /
UI Toolkit without `GeomqttWorld3D`.

## Open questions (for the user, not to guess at)

* Per-set override for `GEOMQTT_ENRICH_ATTRS` — add when a second set needs
  a different attribute list; not before.
* Whether to add a WebGL sample page under `clients/examples/` so the
  Leaflet + MapLibre packages have visual smoke tests alongside the Rust
  integration tests.
