import { describe, expect, test } from "vitest";
import {
  bboxForTile,
  closestPublishedZoom,
  tileForCoord,
  tilesCoveringBbox,
} from "../src/coord.js";

describe("tileForCoord", () => {
  test("(0,0) at z=0 is (0,0)", () => {
    expect(tileForCoord(0, 0, 0)).toEqual({ x: 0, y: 0 });
  });

  test("matches known slippy coordinate (Bologna @ z=10)", () => {
    // Standard OSM wiki example: 44.49, 11.34 at z=10 → 544, 370.
    expect(tileForCoord(10, 44.49, 11.34)).toEqual({ x: 544, y: 370 });
  });

  test("clamps latitude outside Web Mercator range", () => {
    const north = tileForCoord(4, 90, 0);
    const south = tileForCoord(4, -90, 0);
    // Web Mercator caps at ~±85.05, so extreme lats land on the edge tiles.
    expect(north.y).toBe(0);
    expect(south.y).toBe(15);
  });
});

describe("bboxForTile + tileForCoord round-trip", () => {
  test("a tile's center re-resolves to the same tile", () => {
    const cases: Array<[number, number, number]> = [
      [0, 0, 0],
      [4, 8, 5],
      [10, 544, 370],
      [12, 2177, 1481],
    ];
    for (const [z, x, y] of cases) {
      const bb = bboxForTile(z, x, y);
      const cx = (bb.w + bb.e) / 2;
      const cy = (bb.s + bb.n) / 2;
      expect(tileForCoord(z, cy, cx)).toEqual({ x, y });
    }
  });
});

describe("tilesCoveringBbox", () => {
  test("single tile bbox returns just that tile", () => {
    const bb = bboxForTile(10, 544, 370);
    // Shrink the bbox slightly so it stays inside the tile.
    const eps = 0.0001;
    const tiles = tilesCoveringBbox(10, bb.w + eps, bb.s + eps, bb.e - eps, bb.n - eps);
    expect(tiles).toEqual([{ z: 10, x: 544, y: 370 }]);
  });

  test("covers a 2x2 block when spanning four neighboring tiles", () => {
    const a = bboxForTile(10, 544, 370);
    const b = bboxForTile(10, 545, 371);
    const tiles = tilesCoveringBbox(10, a.w, b.s, b.e, a.n);
    // 2x2 block: (544,370), (545,370), (544,371), (545,371)
    expect(tiles).toHaveLength(4);
    expect(tiles).toContainEqual({ z: 10, x: 544, y: 370 });
    expect(tiles).toContainEqual({ z: 10, x: 545, y: 371 });
  });

  test("bbox at exact tile boundaries doesn't over-include neighbours", () => {
    // Regression: floating-point ambiguity at boundaries used to make a
    // single-tile bbox resolve to a 3x3 (or 2x2) block of neighbours.
    const a = bboxForTile(10, 544, 370);
    const tiles = tilesCoveringBbox(10, a.w, a.s, a.e, a.n);
    expect(tiles).toEqual([{ z: 10, x: 544, y: 370 }]);
  });
});

describe("closestPublishedZoom", () => {
  const published = [6, 7, 8, 9, 10, 11, 12];

  test("picks largest published ≤ current", () => {
    expect(closestPublishedZoom(11.7, published)).toBe(11);
    expect(closestPublishedZoom(10.0, published)).toBe(10);
  });

  test("clamps below the lowest", () => {
    expect(closestPublishedZoom(3, published)).toBe(6);
  });

  test("clamps at the highest", () => {
    expect(closestPublishedZoom(99, published)).toBe(12);
  });

  test("handles empty published list by flooring current", () => {
    expect(closestPublishedZoom(11.7, [])).toBe(11);
  });
});
