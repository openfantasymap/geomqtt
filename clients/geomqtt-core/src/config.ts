import type { ServerConfig } from "./types.js";

/**
 * Fetch `GET /config` from a geomqtt HTTP endpoint. Use the returned
 * `zooms` to seed `GeomqttClient({ publishedZooms: cfg.zooms })` so the
 * client subscribes to exactly the zoom levels the server is publishing.
 *
 * @param httpUrl e.g. `"http://localhost:8080"` (no trailing slash required)
 */
export async function fetchServerConfig(httpUrl: string): Promise<ServerConfig> {
  const base = httpUrl.replace(/\/+$/, "");
  const r = await fetch(`${base}/config`);
  if (!r.ok) throw new Error(`geomqtt /config: HTTP ${r.status}`);
  return (await r.json()) as ServerConfig;
}
