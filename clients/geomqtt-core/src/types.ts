/**
 * Payload shapes exchanged with the geomqtt server.
 * See PROTOCOL.md at the repo root for the authoritative contract.
 */

export interface TileCoord {
  z: number;
  x: number;
  y: number;
}

export type TilePayload =
  | { op: "snapshot"; id: string; lat: number; lng: number; attrs?: Record<string, unknown>; ts: number }
  | { op: "add"; id: string; lat: number; lng: number; attrs?: Record<string, unknown>; ts: number }
  | { op: "move"; id: string; lat: number; lng: number; ts: number }
  | { op: "remove"; id: string; ts: number }
  | { op: "attr"; id: string; attrs: Record<string, unknown>; ts: number };

export type ObjectPayload =
  | { op: "snapshot"; id: string; attrs: Record<string, unknown>; ts: number }
  | { op: "attr"; id: string; attrs: Record<string, unknown>; ts: number }
  | { op: "delete"; id: string; ts: number };

/** A point feature as maintained by the client state machine. */
export interface Feature {
  id: string;
  lat: number;
  lng: number;
  properties: Record<string, unknown>;
}

export interface GeomqttOptions {
  /** MQTT-over-WebSocket URL, e.g. `ws://localhost:8083`. */
  url: string;
  /** Default set name used when setViewport is called without one. */
  defaultSet?: string;
  /** Effective zoom levels the server publishes at. Must match the server's
   *  `zooms` (as reported by GET /config). Defaults to `[6..12]` which
   *  matches the server's default GEOMQTT_ENRICH_ZOOMS=6-12 with the default
   *  tile size of 256. Use `fetchServerConfig(url)` to discover this at
   *  runtime. */
  publishedZooms?: number[];
  /** Optional client id. */
  clientId?: string;
  /** Optional auth. */
  username?: string;
  password?: string;
}

/** The shape of `GET /config` on the geomqtt server's HTTP port. */
export interface ServerConfig {
  /** Tile edge in pixels; 256 is the standard slippy cell. */
  tileSize: number;
  /** Effective (shifted) zoom levels used in topics. */
  zooms: number[];
  /** Raw zoom levels as configured in GEOMQTT_ENRICH_ZOOMS. */
  rawZooms: number[];
  /** Attribute keys embedded in tile-topic payloads. */
  enrichAttrs: string[];
  /** Redis key prefix for object hashes (e.g. `obj:`). */
  objectKeyPrefix: string;
}

export type GeomqttEvent =
  | { type: "connected" }
  | { type: "disconnected"; reason?: string }
  | { type: "feature-upsert"; feature: Feature; op: "snapshot" | "add" | "move" | "attr" }
  | { type: "feature-remove"; id: string }
  | { type: "object"; id: string; payload: ObjectPayload }
  | { type: "error"; error: Error }
  | { type: "subscribed"; topics: string[] }
  | { type: "unsubscribed"; topics: string[] };
