import L from "leaflet";
import { GeomqttClient, type Feature, type GeomqttEvent } from "@geomqtt/core";

export interface GeomqttLeafletOptions {
  /** MQTT-over-WebSocket URL of the geomqtt server, e.g. `ws://localhost:8083`. */
  url: string;
  /** Redis GEO set name this layer renders, e.g. `"vehicles"`. */
  set: string;
  /** Zoom levels the server publishes at (must match GEOMQTT_ENRICH_ZOOMS). */
  publishedZooms?: number[];
  /** Hook to build a custom marker/layer per feature. Defaults to a plain CircleMarker. */
  markerFor?: (feature: Feature) => L.Layer;
  clientId?: string;
  username?: string;
  password?: string;
}

export class GeomqttLayer extends L.LayerGroup {
  private client: GeomqttClient;
  private readonly markers = new Map<string, L.Layer>();
  private map: L.Map | null = null;
  private readonly boundMoveEnd: () => void;
  private offEvents: (() => void) | null = null;

  constructor(private readonly opts: GeomqttLeafletOptions) {
    super();
    this.client = new GeomqttClient({
      url: opts.url,
      publishedZooms: opts.publishedZooms,
      clientId: opts.clientId,
      username: opts.username,
      password: opts.password,
    });
    this.boundMoveEnd = () => this.updateViewport();
  }

  override onAdd(map: L.Map): this {
    this.map = map;
    super.onAdd(map);
    this.offEvents = this.client.on((ev) => this.onEvent(ev));
    void this.client.connect().then(() => this.updateViewport());
    map.on("moveend", this.boundMoveEnd);
    map.on("zoomend", this.boundMoveEnd);
    return this;
  }

  override onRemove(map: L.Map): this {
    map.off("moveend", this.boundMoveEnd);
    map.off("zoomend", this.boundMoveEnd);
    this.offEvents?.();
    this.offEvents = null;
    this.client.disconnect();
    for (const layer of this.markers.values()) this.removeLayer(layer);
    this.markers.clear();
    super.onRemove(map);
    this.map = null;
    return this;
  }

  private updateViewport(): void {
    if (!this.map) return;
    const b = this.map.getBounds();
    this.client.setViewport({
      set: this.opts.set,
      zoom: this.map.getZoom(),
      bbox: { w: b.getWest(), s: b.getSouth(), e: b.getEast(), n: b.getNorth() },
    });
  }

  private onEvent(ev: GeomqttEvent): void {
    if (ev.type === "feature-upsert") {
      this.upsertMarker(ev.feature);
    } else if (ev.type === "feature-remove") {
      const m = this.markers.get(ev.id);
      if (m) {
        this.removeLayer(m);
        this.markers.delete(ev.id);
      }
    }
  }

  private upsertMarker(f: Feature): void {
    const existing = this.markers.get(f.id);
    if (existing) {
      if ("setLatLng" in existing && typeof existing.setLatLng === "function") {
        (existing as L.Marker | L.CircleMarker).setLatLng([f.lat, f.lng]);
      }
      if ("setTooltipContent" in existing && typeof (existing as L.Layer & { setTooltipContent?: (c: string) => void }).setTooltipContent === "function") {
        (existing as L.Layer & { setTooltipContent: (c: string) => void }).setTooltipContent(
          describe(f),
        );
      }
      return;
    }
    const layer = this.opts.markerFor
      ? this.opts.markerFor(f)
      : L.circleMarker([f.lat, f.lng], { radius: 6 }).bindTooltip(describe(f));
    this.markers.set(f.id, layer);
    this.addLayer(layer);
  }
}

function describe(f: Feature): string {
  const kv = Object.entries(f.properties)
    .map(([k, v]) => `${k}=${String(v)}`)
    .join(" ");
  return `${f.id}${kv ? ` — ${kv}` : ""}`;
}

/** Convenience: build + add to map in one call. */
export function addGeomqttLayer(map: L.Map, opts: GeomqttLeafletOptions): GeomqttLayer {
  const layer = new GeomqttLayer(opts);
  layer.addTo(map);
  return layer;
}
