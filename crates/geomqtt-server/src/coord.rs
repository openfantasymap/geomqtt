//! Slippy-map XYZ tile math (Web Mercator).

use std::f64::consts::PI;

/// Tile (x, y) at zoom `z` containing `(lat, lon)` in degrees.
pub fn tile_for_coord(z: u8, lat: f64, lon: f64) -> (u32, u32) {
    let n = 2f64.powi(z as i32);
    let x = ((lon + 180.0) / 360.0 * n).floor().clamp(0.0, n - 1.0) as u32;
    let lat_rad = lat.clamp(-85.05112878, 85.05112878).to_radians();
    let y = ((1.0 - (lat_rad.tan() + 1.0 / lat_rad.cos()).ln() / PI) / 2.0 * n)
        .floor()
        .clamp(0.0, n - 1.0) as u32;
    (x, y)
}

/// (west, south, east, north) bbox of tile (z, x, y), degrees.
pub fn bbox_for_tile(z: u8, x: u32, y: u32) -> (f64, f64, f64, f64) {
    let n = 2f64.powi(z as i32);
    let w = x as f64 / n * 360.0 - 180.0;
    let e = (x + 1) as f64 / n * 360.0 - 180.0;
    let north = (PI * (1.0 - 2.0 * y as f64 / n)).sinh().atan().to_degrees();
    let south = (PI * (1.0 - 2.0 * (y + 1) as f64 / n))
        .sinh()
        .atan()
        .to_degrees();
    (w, south, e, north)
}

/// Containing tiles at each of the configured zoom levels for a point.
pub fn tiles_for_point(zooms: &[u8], lat: f64, lon: f64) -> Vec<(u8, u32, u32)> {
    zooms
        .iter()
        .map(|&z| {
            let (x, y) = tile_for_coord(z, lat, lon);
            (z, x, y)
        })
        .collect()
}

/// All tile (x, y) pairs at zoom `z` that intersect the bbox (w, s, e, n).
///
/// A bbox edge that sits *exactly* on a tile boundary is treated as belonging
/// to the inside of the bbox, not the neighbouring tile — otherwise floating
/// point ambiguity at the boundary would over-include a row/column on each
/// side. The corners are nudged inward by a tiny fraction of a tile span.
#[allow(dead_code)]
pub fn tiles_covering_bbox(z: u8, w: f64, s: f64, e: f64, n: f64) -> Vec<(u32, u32)> {
    let tile_span = 360.0 / 2f64.powi(z as i32);
    let eps = tile_span * 1e-9;
    let (x0, y0) = tile_for_coord(z, n - eps, w + eps);
    let (x1, y1) = tile_for_coord(z, s + eps, e - eps);
    let (x_min, x_max) = (x0.min(x1), x0.max(x1));
    let (y_min, y_max) = (y0.min(y1), y0.max(y1));
    let mut out = Vec::with_capacity(((x_max - x_min + 1) * (y_max - y_min + 1)) as usize);
    for y in y_min..=y_max {
        for x in x_min..=x_max {
            out.push((x, y));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zero_zero_at_z0() {
        assert_eq!(tile_for_coord(0, 0.0, 0.0), (0, 0));
    }

    #[test]
    fn roundtrip_bbox_contains_center() {
        let (z, x, y) = (10, 523, 391);
        let (w, s, e, n) = bbox_for_tile(z, x, y);
        let (cx, cy) = ((w + e) / 2.0, (s + n) / 2.0);
        assert_eq!(tile_for_coord(z, cy, cx), (x, y));
    }

    #[test]
    fn tiles_for_point_one_per_zoom() {
        let tiles = tiles_for_point(&[4, 8, 12], 44.49, 11.34);
        assert_eq!(tiles.len(), 3);
        assert_eq!(tiles[0].0, 4);
    }

    #[test]
    fn tiles_covering_bbox_exact_boundary_is_inclusive_not_overinclusive() {
        // bbox built from the *exact* edges of tiles (544, 370) and (545, 371)
        // should resolve to a 2×2 block, not the 4×3 block the boundary
        // floating-point ambiguity would otherwise produce.
        let a = bbox_for_tile(10, 544, 370);
        let b = bbox_for_tile(10, 545, 371);
        let (w, s, e, n) = (a.0, b.1, b.2, a.3);
        let tiles = tiles_covering_bbox(10, w, s, e, n);
        assert_eq!(tiles.len(), 4, "got {tiles:?}");
        assert!(tiles.contains(&(544, 370)));
        assert!(tiles.contains(&(545, 370)));
        assert!(tiles.contains(&(544, 371)));
        assert!(tiles.contains(&(545, 371)));
    }

    #[test]
    fn tiles_covering_bbox_single_interior_bbox_returns_one_tile() {
        let (w, s, e, n) = bbox_for_tile(10, 544, 370);
        // Shrink so the bbox is strictly interior.
        let eps = 1e-4;
        let tiles = tiles_covering_bbox(10, w + eps, s + eps, e - eps, n - eps);
        assert_eq!(tiles, vec![(544, 370)]);
    }
}
