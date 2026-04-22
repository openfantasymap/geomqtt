use anyhow::{anyhow, Context, Result};
use std::net::SocketAddr;

#[derive(Debug, Clone)]
pub struct Config {
    pub redis_url: String,
    pub resp_addr: SocketAddr,
    pub mqtt_addr: SocketAddr,
    pub mqtt_ws_addr: SocketAddr,
    pub http_addr: SocketAddr,
    pub enrich_attrs: Vec<String>,
    /// Raw zoom levels requested in GEOMQTT_ENRICH_ZOOMS (before tile-size shift).
    pub raw_zooms: Vec<u8>,
    /// Effective zoom levels actually used in topics and subscriptions — i.e.
    /// `raw_zooms + zoom_offset(tile_size)`. Tile pixel size of 128 is equivalent
    /// to raising every zoom by 1; 64 raises by 2; etc.
    pub enrich_zooms: Vec<u8>,
    /// Tile edge size in pixels. 256 = standard slippy cell; 128 = half-width;
    /// 1 = max granularity. Must be a power of two ≤ 256.
    pub tile_size: u16,
    pub object_key_prefix: String,
}

impl Config {
    pub fn from_env() -> Result<Self> {
        let tile_size =
            parse_tile_size(&env_or("GEOMQTT_TILE_SIZE", "256")).context("GEOMQTT_TILE_SIZE")?;
        let zoom_offset = zoom_offset_for_tile_size(tile_size);
        let raw_zooms = parse_zoom_list(&env_or("GEOMQTT_ENRICH_ZOOMS", "6-12"))
            .context("GEOMQTT_ENRICH_ZOOMS")?;
        let enrich_zooms: Vec<u8> = raw_zooms
            .iter()
            .map(|z| z.saturating_add(zoom_offset))
            .collect();

        Ok(Self {
            redis_url: env_or("GEOMQTT_REDIS_URL", "redis://127.0.0.1:6379"),
            resp_addr: parse_addr("GEOMQTT_RESP_ADDR", "0.0.0.0:6380")?,
            mqtt_addr: parse_addr("GEOMQTT_MQTT_ADDR", "0.0.0.0:1883")?,
            mqtt_ws_addr: parse_addr("GEOMQTT_MQTT_WS_ADDR", "0.0.0.0:8083")?,
            http_addr: parse_addr("GEOMQTT_HTTP_ADDR", "0.0.0.0:8080")?,
            enrich_attrs: csv(&env_or("GEOMQTT_ENRICH_ATTRS", "")),
            raw_zooms,
            enrich_zooms,
            tile_size,
            object_key_prefix: env_or("GEOMQTT_OBJECT_KEY_PREFIX", "obj:"),
        })
    }
}

fn env_or(key: &str, default: &str) -> String {
    std::env::var(key).unwrap_or_else(|_| default.to_string())
}

fn parse_addr(key: &str, default: &str) -> Result<SocketAddr> {
    env_or(key, default)
        .parse()
        .with_context(|| key.to_string())
}

fn csv(s: &str) -> Vec<String> {
    s.split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect()
}

fn parse_tile_size(s: &str) -> Result<u16> {
    let v: u16 = s
        .trim()
        .parse()
        .map_err(|_| anyhow!("not a number: {s:?}"))?;
    if v == 0 || v > 256 || !v.is_power_of_two() {
        return Err(anyhow!(
            "tile_size must be a power of 2 in 1..=256 (got {v})"
        ));
    }
    Ok(v)
}

fn zoom_offset_for_tile_size(tile_size: u16) -> u8 {
    // tile_size 256 → 0, 128 → 1, 64 → 2, … 1 → 8.
    (256u16 / tile_size).trailing_zeros() as u8
}

/// Parses a comma-separated list of zooms with optional inclusive ranges:
///
/// * `"6-12"`         → `[6,7,8,9,10,11,12]`
/// * `"4,6-10,14"`    → `[4,6,7,8,9,10,14]`
/// * `"8"`            → `[8]`
/// * `""`             → `[]`
///
/// Output is deduplicated and sorted ascending.
pub fn parse_zoom_list(s: &str) -> Result<Vec<u8>> {
    let mut out: Vec<u8> = Vec::new();
    for token in s.split(',').map(str::trim).filter(|t| !t.is_empty()) {
        if let Some((lo, hi)) = token.split_once('-') {
            let lo: u8 = lo
                .trim()
                .parse()
                .map_err(|_| anyhow!("bad range {token:?}"))?;
            let hi: u8 = hi
                .trim()
                .parse()
                .map_err(|_| anyhow!("bad range {token:?}"))?;
            if hi < lo {
                return Err(anyhow!("reversed range {token:?}"));
            }
            for z in lo..=hi {
                out.push(z);
            }
        } else {
            out.push(token.parse().map_err(|_| anyhow!("bad zoom {token:?}"))?);
        }
    }
    out.sort_unstable();
    out.dedup();
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ranges() {
        assert_eq!(
            parse_zoom_list("6-12").unwrap(),
            (6u8..=12).collect::<Vec<_>>()
        );
        assert_eq!(parse_zoom_list("4,6-8,10").unwrap(), vec![4, 6, 7, 8, 10]);
        assert_eq!(parse_zoom_list("").unwrap(), Vec::<u8>::new());
        assert_eq!(parse_zoom_list("8,8,8").unwrap(), vec![8]);
        assert!(parse_zoom_list("12-6").is_err());
    }

    #[test]
    fn tile_sizes() {
        assert_eq!(zoom_offset_for_tile_size(256), 0);
        assert_eq!(zoom_offset_for_tile_size(128), 1);
        assert_eq!(zoom_offset_for_tile_size(64), 2);
        assert_eq!(zoom_offset_for_tile_size(1), 8);
        assert!(parse_tile_size("384").is_err());
        assert!(parse_tile_size("200").is_err());
        assert!(parse_tile_size("0").is_err());
        assert_eq!(parse_tile_size("128").unwrap(), 128);
    }
}
