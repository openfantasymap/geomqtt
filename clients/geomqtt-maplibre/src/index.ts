import type {
  GeoJSONSource,
  LayerSpecification,
  Map as MapLibreMap,
} from "maplibre-gl";
import { GeomqttClient, type Feature, type GeomqttEvent } from "@geomqtt/core";

export interface GeomqttMaplibreOptions {
  /** The MapLibre / Mapbox GL map instance. */
  map: MapLibreMap;
  /** MQTT-over-WebSocket URL, e.g. `ws://localhost:8083`. */
  url: string;
  /** Redis GEO set name this layer renders. */
  set: string;
  /** Zoom levels the server publishes at (must match GEOMQTT_ENRICH_ZOOMS). */
  publishedZooms?: number[];
  /** Id used for the GeoJSON source added to the map. Default: `geomqtt-<set>`. */
  sourceId?: string;
  /** Optional layers to add on top of the source. If omitted, a default circle layer is added. */
  layers?: LayerSpecification[];
  /** ms to throttle setData calls. Default: 33 (~30fps). */
  updateThrottleMs?: number;
  clientId?: string;
  username?: string;
  password?: string;
}

/**
 * Manages a GeoJSON source whose features are the current state of a geomqtt
 * set. Pan/zoom events on the map drive the client's viewport subscriptions.
 */
export class GeomqttSource {
  private readonly client: GeomqttClient;
  private readonly sourceId: string;
  private readonly layerIds: string[] = [];
  private attached = false;
  private offEvents: (() => void) | null = null;
  private readonly boundMoveEnd: () => void;
  private updatePending = false;
  private lastUpdate = 0;
  private readonly throttleMs: number;

  constructor(private readonly opts: GeomqttMaplibreOptions) {
    this.client = new GeomqttClient({
      url: opts.url,
      publishedZooms: opts.publishedZooms,
      clientId: opts.clientId,
      username: opts.username,
      password: opts.password,
    });
    this.sourceId = opts.sourceId ?? `geomqtt-${opts.set}`;
    this.throttleMs = opts.updateThrottleMs ?? 33;
    this.boundMoveEnd = () => this.updateViewport();
  }

  async attach(): Promise<void> {
    if (this.attached) return;
    const map = this.opts.map;
    map.addSource(this.sourceId, {
      type: "geojson",
      data: { type: "FeatureCollection", features: [] },
    });
    const layers =
      this.opts.layers ??
      ([
        {
          id: `${this.sourceId}-circle`,
          type: "circle",
          source: this.sourceId,
          paint: {
            "circle-radius": 5,
            "circle-color": "#ff5722",
            "circle-stroke-color": "#222",
            "circle-stroke-width": 1,
          },
        },
      ] satisfies LayerSpecification[]);
    for (const spec of layers) {
      map.addLayer({ ...spec, source: this.sourceId } as LayerSpecification);
      this.layerIds.push(spec.id);
    }
    this.offEvents = this.client.on((ev) => this.onEvent(ev));
    map.on("moveend", this.boundMoveEnd);
    map.on("zoomend", this.boundMoveEnd);
    await this.client.connect();
    this.updateViewport();
    this.attached = true;
  }

  detach(): void {
    if (!this.attached) return;
    const map = this.opts.map;
    map.off("moveend", this.boundMoveEnd);
    map.off("zoomend", this.boundMoveEnd);
    for (const id of this.layerIds) {
      if (map.getLayer(id)) map.removeLayer(id);
    }
    this.layerIds.length = 0;
    if (map.getSource(this.sourceId)) map.removeSource(this.sourceId);
    this.offEvents?.();
    this.offEvents = null;
    this.client.disconnect();
    this.attached = false;
  }

  private updateViewport(): void {
    const map = this.opts.map;
    const b = map.getBounds();
    this.client.setViewport({
      set: this.opts.set,
      zoom: map.getZoom(),
      bbox: { w: b.getWest(), s: b.getSouth(), e: b.getEast(), n: b.getNorth() },
    });
  }

  private onEvent(ev: GeomqttEvent): void {
    if (ev.type === "feature-upsert" || ev.type === "feature-remove") {
      this.scheduleDataPush();
    }
  }

  private scheduleDataPush(): void {
    if (this.updatePending) return;
    const now = Date.now();
    const wait = Math.max(0, this.throttleMs - (now - this.lastUpdate));
    this.updatePending = true;
    setTimeout(() => {
      this.updatePending = false;
      this.lastUpdate = Date.now();
      this.pushData();
    }, wait);
  }

  private pushData(): void {
    const map = this.opts.map;
    const src = map.getSource(this.sourceId) as GeoJSONSource | undefined;
    if (!src) return;
    const features = this.client.snapshot().map((f: Feature) => ({
      type: "Feature" as const,
      id: f.id,
      geometry: { type: "Point" as const, coordinates: [f.lng, f.lat] },
      properties: f.properties,
    }));
    src.setData({ type: "FeatureCollection", features });
  }
}

/** Convenience: instantiate and attach in one call. */
export async function addGeomqttSource(opts: GeomqttMaplibreOptions): Promise<GeomqttSource> {
  const src = new GeomqttSource(opts);
  await src.attach();
  return src;
}
