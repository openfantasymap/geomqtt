/**
 * GeomqttClient — protocol-level client used by the Leaflet and MapLibre
 * adapters. Maintains an id-keyed state map and emits events on every change.
 */

import mqtt, { type MqttClient } from "mqtt";
import { closestPublishedZoom, tilesCoveringBbox } from "./coord.js";
import type {
  Feature,
  GeomqttEvent,
  GeomqttOptions,
  ObjectPayload,
  TileCoord,
  TilePayload,
} from "./types.js";
import { diffSubscriptions, tileTopic } from "./viewport.js";

type Listener = (ev: GeomqttEvent) => void;

export class GeomqttClient {
  private mqtt: MqttClient | null = null;
  private readonly listeners = new Set<Listener>();
  private readonly features = new Map<string, Feature>();
  private readonly tileSubs = new Set<string>();
  private readonly objectSubs = new Set<string>();
  private readonly publishedZooms: number[];

  constructor(private readonly opts: GeomqttOptions) {
    // Default matches the server's default GEOMQTT_ENRICH_ZOOMS=6-12 at tile_size=256.
    // If you run the server with a different tile_size, call `fetchServerConfig()`
    // first and pass `cfg.zooms` here.
    const defaults = Array.from({ length: 7 }, (_, i) => i + 6);
    this.publishedZooms = (opts.publishedZooms ?? defaults).slice().sort((a, b) => a - b);
  }

  async connect(): Promise<void> {
    if (this.mqtt) return;
    const client = mqtt.connect(this.opts.url, {
      clientId: this.opts.clientId,
      username: this.opts.username,
      password: this.opts.password,
      clean: true,
      protocolVersion: 4,
      reconnectPeriod: 2000,
    });
    this.mqtt = client;
    client.on("message", (topic, payload) => this.onMessage(topic, payload));
    client.on("connect", () => this.emit({ type: "connected" }));
    client.on("close", () => this.emit({ type: "disconnected" }));
    client.on("error", (error) => this.emit({ type: "error", error }));
    await new Promise<void>((resolve, reject) => {
      const onConnect = () => {
        client.off("error", onError);
        resolve();
      };
      const onError = (e: Error) => {
        client.off("connect", onConnect);
        reject(e);
      };
      client.once("connect", onConnect);
      client.once("error", onError);
    });
  }

  disconnect(): void {
    this.mqtt?.end(true);
    this.mqtt = null;
    this.features.clear();
    this.tileSubs.clear();
    this.objectSubs.clear();
  }

  on(listener: Listener): () => void {
    this.listeners.add(listener);
    return () => this.listeners.delete(listener);
  }

  /** Current state, as a snapshot. Safe to read at any time. */
  snapshot(): Feature[] {
    return Array.from(this.features.values());
  }

  /**
   * Update the viewport's tile coverage. Pass a bbox and a zoom; the client
   * picks the closest published zoom, computes tile coverage, diffs it
   * against the previous subscription set, and issues MQTT SUB/UNSUB.
   *
   * Returns the tiles that ended up being subscribed to, for debugging.
   */
  setViewport(params: {
    set: string;
    zoom: number;
    bbox: { w: number; s: number; e: number; n: number };
  }): TileCoord[] {
    const z = closestPublishedZoom(params.zoom, this.publishedZooms);
    const tiles = tilesCoveringBbox(z, params.bbox.w, params.bbox.s, params.bbox.e, params.bbox.n);
    const nextTopics = new Set(tiles.map((t) => tileTopic(params.set, t)));
    const { toSubscribe, toUnsubscribe } = diffSubscriptions(this.tileSubs, nextTopics);
    if (toUnsubscribe.length) {
      this.mqtt?.unsubscribe(toUnsubscribe);
      for (const t of toUnsubscribe) this.tileSubs.delete(t);
      this.evictFeaturesNotInTopics(nextTopics, params.set, z);
    }
    if (toSubscribe.length) {
      this.mqtt?.subscribe(toSubscribe, { qos: 0 });
      for (const t of toSubscribe) this.tileSubs.add(t);
    }
    return tiles;
  }

  subscribeObject(obid: string): void {
    const topic = `objects/${obid}`;
    if (this.objectSubs.has(topic)) return;
    this.mqtt?.subscribe(topic, { qos: 0 });
    this.objectSubs.add(topic);
  }

  unsubscribeObject(obid: string): void {
    const topic = `objects/${obid}`;
    if (!this.objectSubs.has(topic)) return;
    this.mqtt?.unsubscribe(topic);
    this.objectSubs.delete(topic);
  }

  private emit(ev: GeomqttEvent): void {
    for (const l of this.listeners) {
      try {
        l(ev);
      } catch {
        // swallow — a broken listener must not poison the bus
      }
    }
  }

  private onMessage(topic: string, payload: Buffer | Uint8Array): void {
    let text: string;
    if (typeof (payload as Buffer).toString === "function") {
      text = (payload as Buffer).toString("utf8");
    } else {
      text = new TextDecoder().decode(payload as Uint8Array);
    }
    let parsed: unknown;
    try {
      parsed = JSON.parse(text);
    } catch (e) {
      this.emit({ type: "error", error: e instanceof Error ? e : new Error(String(e)) });
      return;
    }
    if (topic.startsWith("geo/")) {
      this.handleTile(parsed as TilePayload);
    } else if (topic.startsWith("objects/")) {
      this.handleObject(topic.slice("objects/".length), parsed as ObjectPayload);
    }
  }

  private handleTile(p: TilePayload): void {
    switch (p.op) {
      case "snapshot":
      case "add": {
        const feat: Feature = {
          id: p.id,
          lat: p.lat,
          lng: p.lng,
          properties: { ...(this.features.get(p.id)?.properties ?? {}), ...(p.attrs ?? {}) },
        };
        this.features.set(p.id, feat);
        this.emit({ type: "feature-upsert", feature: feat, op: p.op });
        break;
      }
      case "move": {
        const prev = this.features.get(p.id);
        const feat: Feature = {
          id: p.id,
          lat: p.lat,
          lng: p.lng,
          properties: prev?.properties ?? {},
        };
        this.features.set(p.id, feat);
        this.emit({ type: "feature-upsert", feature: feat, op: "move" });
        break;
      }
      case "remove": {
        if (this.features.delete(p.id)) this.emit({ type: "feature-remove", id: p.id });
        break;
      }
      case "attr": {
        const prev = this.features.get(p.id);
        if (!prev) break;
        const feat: Feature = {
          ...prev,
          properties: { ...prev.properties, ...p.attrs },
        };
        this.features.set(p.id, feat);
        this.emit({ type: "feature-upsert", feature: feat, op: "attr" });
        break;
      }
    }
  }

  private handleObject(id: string, p: ObjectPayload): void {
    this.emit({ type: "object", id, payload: p });
    // Also merge attrs into any tracked feature so UI-bound listeners see them.
    if (p.op === "snapshot" || p.op === "attr") {
      const prev = this.features.get(id);
      if (prev) {
        const feat: Feature = {
          ...prev,
          properties: { ...prev.properties, ...p.attrs },
        };
        this.features.set(id, feat);
        this.emit({ type: "feature-upsert", feature: feat, op: "attr" });
      }
    }
  }

  /**
   * When the viewport shrinks, drop features whose last-known tile is no
   * longer subscribed. Without this the state map grows unboundedly on pans.
   */
  private evictFeaturesNotInTopics(nextTopics: Set<string>, set: string, z: number): void {
    for (const [id, feat] of this.features) {
      const { x, y } = tileForCoordLocal(z, feat.lat, feat.lng);
      const topic = `geo/${set}/${z}/${x}/${y}`;
      if (!nextTopics.has(topic) && !this.objectSubs.has(`objects/${id}`)) {
        this.features.delete(id);
        this.emit({ type: "feature-remove", id });
      }
    }
  }
}

// Local copy to avoid a circular import when called from the eviction path.
function tileForCoordLocal(z: number, lat: number, lon: number): { x: number; y: number } {
  const n = 2 ** z;
  const clampedLat = Math.max(-85.05112878, Math.min(85.05112878, lat));
  const latRad = (clampedLat * Math.PI) / 180;
  const x = Math.floor(((lon + 180) / 360) * n);
  const y = Math.floor(((1 - Math.log(Math.tan(latRad) + 1 / Math.cos(latRad)) / Math.PI) / 2) * n);
  const max = n - 1;
  return { x: Math.max(0, Math.min(max, x)), y: Math.max(0, Math.min(max, y)) };
}
