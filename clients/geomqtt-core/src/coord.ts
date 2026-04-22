/**
 * Slippy-map XYZ tile math (Web Mercator). Mirrors the server-side `coord.rs`.
 */

import type { TileCoord } from "./types.js";

const MAX_LAT = 85.05112878;

export function tileForCoord(z: number, lat: number, lon: number): { x: number; y: number } {
  const n = 2 ** z;
  const clampedLat = Math.max(-MAX_LAT, Math.min(MAX_LAT, lat));
  const latRad = (clampedLat * Math.PI) / 180;
  const x = Math.floor(((lon + 180) / 360) * n);
  const y = Math.floor(
    ((1 - Math.log(Math.tan(latRad) + 1 / Math.cos(latRad)) / Math.PI) / 2) * n,
  );
  const max = n - 1;
  return {
    x: Math.max(0, Math.min(max, x)),
    y: Math.max(0, Math.min(max, y)),
  };
}

export function bboxForTile(
  z: number,
  x: number,
  y: number,
): { w: number; s: number; e: number; n: number } {
  const n = 2 ** z;
  const w = (x / n) * 360 - 180;
  const e = ((x + 1) / n) * 360 - 180;
  const north = (Math.atan(Math.sinh(Math.PI * (1 - (2 * y) / n))) * 180) / Math.PI;
  const south = (Math.atan(Math.sinh(Math.PI * (1 - (2 * (y + 1)) / n))) * 180) / Math.PI;
  return { w, s: south, e, n: north };
}

/**
 * All tiles at zoom `z` that intersect the bbox. Handles antimeridian by not
 * wrapping — callers can split a crossed viewport before calling.
 */
export function tilesCoveringBbox(
  z: number,
  w: number,
  s: number,
  e: number,
  n: number,
): TileCoord[] {
  const tl = tileForCoord(z, n, w);
  const br = tileForCoord(z, s, e);
  const xMin = Math.min(tl.x, br.x);
  const xMax = Math.max(tl.x, br.x);
  const yMin = Math.min(tl.y, br.y);
  const yMax = Math.max(tl.y, br.y);
  const out: TileCoord[] = [];
  for (let y = yMin; y <= yMax; y++) {
    for (let x = xMin; x <= xMax; x++) {
      out.push({ z, x, y });
    }
  }
  return out;
}

/**
 * Pick the largest published zoom ≤ `current`. Falls back to the lowest
 * published if `current` is below all of them. Assumes `published` is
 * non-empty and sorted ascending.
 */
export function closestPublishedZoom(current: number, published: number[]): number {
  if (published.length === 0) return Math.floor(current);
  const sorted = [...published].sort((a, b) => a - b);
  const first = sorted[0]!;
  const last = sorted[sorted.length - 1]!;
  if (current <= first) return first;
  if (current >= last) return last;
  let chosen = first;
  for (const z of sorted) {
    if (z <= current) chosen = z;
    else break;
  }
  return chosen;
}
