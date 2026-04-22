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
#[allow(dead_code)]
pub fn tiles_covering_bbox(z: u8, w: f64, s: f64, e: f64, n: f64) -> Vec<(u32, u32)> {
    let (x0, y0) = tile_for_coord(z, n, w);
    let (x1, y1) = tile_for_coord(z, s, e);
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
}
