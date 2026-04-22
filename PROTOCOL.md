# geomqtt protocol

The contract between the server and any client (Leaflet, MapLibre, Unity, CLI).
All three clients must agree on this document; the server is its reference
implementation.

Version: `0.1` — tested against Redis 7. Some commands used (notably `GEOSEARCH`)
require Redis 6.2 or newer; Redis 4/5 works for basic GEOADD/GEOPOS but the
snapshot-on-SUB path will fail without `GEOSEARCH`.

## 1. Transports

| Port | Protocol              | Purpose                                              |
|------|-----------------------|------------------------------------------------------|
| 6380 | RESP (Redis wire)     | Redis-compatible proxy — all standard commands work  |
| 1883 | MQTT v3.1.1 / v5 TCP  | Server-to-server and native app clients              |
| 8083 | MQTT over WebSocket   | Browser clients (Leaflet / MapLibre)                 |
| 8080 | HTTP                  | GeoJSON snapshots, `/healthz`                        |

Only **clean session, QoS 0** is supported in v0.1. There is no offline queue,
no retained messages, no persistent subscriptions. On reconnect a client
re-subscribes and receives a fresh snapshot.

## 2. Data model

Two kinds of keys in the upstream Redis:

* **Geo sets.** A Redis GEO set (sorted set with geohash scores) per "layer",
  e.g. `vehicles`, `sensors`. Written with `GEOADD <set> <lon> <lat> <obid>`.
* **Object hashes.** One hash per object at `<object_key_prefix><obid>`
  (default prefix `obj:`), e.g. `HSET obj:veh-42 icon truck color red`.

The geo set is the position source of truth. The object hash carries
attributes. An object may appear in multiple geo sets (same `obid`, different
"layers") — its hash is shared.

## 3. MQTT topic layout

```
geo/<set>/<z>/<x>/<y>       position events for tile (z,x,y) of <set>
objects/<obid>              attribute events for a specific object
```

`<set>` is the name of a Redis GEO set. `<z>/<x>/<y>` are slippy-map XYZ tile
coordinates. The server publishes to an **effective** zoom list that depends
on two env vars:

* `GEOMQTT_ENRICH_ZOOMS` — list or range of raw zooms (default `6-12`, i.e.
  inclusive `6,7,8,9,10,11,12`). Accepts mixed notation: `4,6-10,14`.
* `GEOMQTT_TILE_SIZE` — tile edge in pixels (default `256`; must be a
  power of 2 in `1..=256`). Smaller values shift every raw zoom up by
  `log2(256 / tile_size)` — `128` is +1, `64` is +2, etc. — which gives
  finer geographic granularity without typing higher zoom numbers.

Clients that need the authoritative effective list should call `GET /config`
(see §6) and seed their subscription zoom list from the returned `zooms`.

Wildcards work as standard MQTT: `geo/vehicles/10/+/+` subscribes to an entire
zoom band; `geo/+/10/523/391` to one tile across all sets.

## 4. Payloads

All payloads are UTF-8 JSON objects with an `op` field. Timestamps are
milliseconds since Unix epoch.

### 4.1 Tile topic (`geo/<set>/<z>/<x>/<y>`)

```jsonc
// op="snapshot" — delivered per-subscriber immediately after SUBSCRIBE.
// One message per point currently in the tile's bbox.
{ "op": "snapshot", "id": "veh-42", "lat": 44.49, "lng": 11.34,
  "attrs": { "icon": "truck", "color": "red" }, "ts": 1714000000000 }

// op="add" — object entered the tile (first sighting OR moved in from elsewhere).
{ "op": "add", "id": "veh-42", "lat": 44.49, "lng": 11.34,
  "attrs": { "icon": "truck", "color": "red" }, "ts": 1714000000123 }

// op="move" — object already in this tile, updated position.
{ "op": "move", "id": "veh-42", "lat": 44.491, "lng": 11.341,
  "ts": 1714000000456 }

// op="remove" — object left this tile (moved to another, or deleted).
{ "op": "remove", "id": "veh-42", "ts": 1714000000789 }

// op="attr" — reserved. Attribute-only changes are v0.1 only fanned out to the
// object topic (see §4.2). Tile-side "attr" messages will be added in v0.2 so
// that clients subscribed to a tile see attribute changes without also having
// to subscribe to objects/<obid>. Clients MUST tolerate "attr" messages landing
// on tile topics in a forward-compatible way (treat as a partial update).
```

Enrichment rule: `attrs` on tile payloads contains only the intersection of
`GEOMQTT_ENRICH_ATTRS` and the keys actually present in the object's hash.
Missing configured keys are simply omitted — they are never `null`.

### 4.2 Object topic (`objects/<obid>`)

```jsonc
// op="snapshot" — full current attribute set, delivered per-subscriber on SUB.
{ "op": "snapshot", "id": "veh-42",
  "attrs": { "icon": "truck", "color": "red", "plate": "BO-123" },
  "ts": 1714000000000 }

// op="attr" — one or more attributes changed.
{ "op": "attr", "id": "veh-42", "attrs": { "color": "green" },
  "ts": 1714000001000 }

// op="delete" — the object's hash was DELd.
{ "op": "delete", "id": "veh-42", "ts": 1714000002000 }
```

Object topic payloads are **not** filtered by `GEOMQTT_ENRICH_ATTRS` — they
carry the full attribute set. Filtering only applies to tile payloads where
bandwidth is the concern.

## 5. Subscription lifecycle

```
client → SUBSCRIBE geo/vehicles/10/523/391
server ← PUBLISH  geo/vehicles/10/523/391  {"op":"snapshot", ...}   (N times)
server ← PUBLISH  geo/vehicles/10/523/391  {"op":"add",      ...}   (live)
server ← PUBLISH  geo/vehicles/10/523/391  {"op":"move",     ...}
client → UNSUBSCRIBE geo/vehicles/10/523/391
```

Ordering guarantee within a topic is best-effort. Clients MUST dedup by `id`:
the snapshot burst and the live stream can race, and a client may see the
same point arrive as both `snapshot` and `add`. Treat `add`/`move`/`snapshot`
as idempotent position updates keyed by `id`.

## 6. HTTP endpoints

GeoJSON endpoints return `application/geo+json`; `/config` returns
`application/json`.

```
GET /healthz
  → 200 "ok"

GET /config
  → { "tileSize": 256,
      "zooms":        [6,7,8,9,10,11,12],   // effective zooms used in topics
      "rawZooms":     [6,7,8,9,10,11,12],   // pre-shift GEOMQTT_ENRICH_ZOOMS
      "enrichAttrs":  ["icon","color",...],
      "objectKeyPrefix": "obj:" }

GET /tiles/<set>/<z>/<x>/<y>
  → FeatureCollection of points in tile bbox. Properties = full object hash.

GET /viewport/<set>?bbox=<w>,<s>,<e>,<n>
  → FeatureCollection of points inside the arbitrary bbox.

GET /objects/<obid>
  → single Feature (Point) with properties = full object hash.
    404 if the obid has no known position OR no hash.
```

HTTP is synchronous and hits Redis directly on each call — there is no
dependency on the MQTT live-tile machinery.

## 7. Constraints (v0.1)

* Clean session, QoS 0 only.
* Single Redis database (no cluster yet, though `fred` will support it).
* Tile publishes only at `GEOMQTT_ENRICH_ZOOMS` levels.
* No authentication layer — deploy behind a reverse proxy that terminates
  auth/TLS.
* Topic payloads are JSON. A binary format (CBOR/MessagePack) is a future
  consideration, not v0.1.
