//! HTTP endpoints for GeoJSON visualization.

use crate::coord::bbox_for_tile;
use crate::mqtt::{geosearch_box, hgetall, GeoHit, MqttContext};
use anyhow::Result;
use axum::{
    extract::{Path, Query, State},
    http::{header, StatusCode},
    response::IntoResponse,
    routing::get,
    Router,
};
use serde::Deserialize;
use serde_json::{json, Map, Value};
use std::net::SocketAddr;
use tracing::info;

#[derive(Clone)]
pub struct HttpState {
    pub ctx: MqttContext,
}

pub async fn serve(addr: SocketAddr, state: HttpState) -> Result<()> {
    let app = Router::new()
        .route("/healthz", get(healthz))
        .route("/config", get(config_json))
        .route("/tiles/{set}/{z}/{x}/{y}", get(tile_geojson))
        .route("/viewport/{set}", get(viewport_geojson))
        .route("/objects/{obid}", get(object_geojson))
        .with_state(state);

    let listener = tokio::net::TcpListener::bind(addr).await?;
    info!(%addr, "HTTP listener bound");
    axum::serve(listener, app).await?;
    Ok(())
}

async fn healthz() -> &'static str {
    "ok"
}

async fn config_json(State(state): State<HttpState>) -> impl IntoResponse {
    let cfg = &state.ctx.cfg;
    let body = json!({
        "tileSize": cfg.tile_size,
        "zooms": cfg.enrich_zooms,
        "rawZooms": cfg.raw_zooms,
        "enrichAttrs": cfg.enrich_attrs,
        "objectKeyPrefix": cfg.object_key_prefix,
    });
    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "application/json")],
        body.to_string(),
    )
}

fn geojson_ct() -> [(header::HeaderName, &'static str); 1] {
    [(header::CONTENT_TYPE, "application/geo+json")]
}

async fn tile_geojson(
    State(state): State<HttpState>,
    Path((set, z, x, y)): Path<(String, u8, u32, u32)>,
) -> impl IntoResponse {
    let (w, s, e, n) = bbox_for_tile(z, x, y);
    match feature_collection_for_bbox(&state.ctx, &set, w, s, e, n).await {
        Ok(fc) => (StatusCode::OK, geojson_ct(), fc.to_string()).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, format!("error: {e}")).into_response(),
    }
}

#[derive(Deserialize)]
struct ViewportQuery {
    bbox: String,
}

async fn viewport_geojson(
    State(state): State<HttpState>,
    Path(set): Path<String>,
    Query(q): Query<ViewportQuery>,
) -> impl IntoResponse {
    let parts: Vec<&str> = q.bbox.split(',').collect();
    if parts.len() != 4 {
        return (StatusCode::BAD_REQUEST, "bbox must be w,s,e,n".to_string()).into_response();
    }
    let Ok(w) = parts[0].parse::<f64>() else {
        return (StatusCode::BAD_REQUEST, "bad w".to_string()).into_response();
    };
    let Ok(s) = parts[1].parse::<f64>() else {
        return (StatusCode::BAD_REQUEST, "bad s".to_string()).into_response();
    };
    let Ok(e) = parts[2].parse::<f64>() else {
        return (StatusCode::BAD_REQUEST, "bad e".to_string()).into_response();
    };
    let Ok(n) = parts[3].parse::<f64>() else {
        return (StatusCode::BAD_REQUEST, "bad n".to_string()).into_response();
    };
    match feature_collection_for_bbox(&state.ctx, &set, w, s, e, n).await {
        Ok(fc) => (StatusCode::OK, geojson_ct(), fc.to_string()).into_response(),
        Err(err) => (StatusCode::INTERNAL_SERVER_ERROR, format!("error: {err}")).into_response(),
    }
}

async fn object_geojson(
    State(state): State<HttpState>,
    Path(obid): Path<String>,
) -> impl IntoResponse {
    match feature_for_object(&state.ctx, &obid).await {
        Ok(Some(feat)) => (StatusCode::OK, geojson_ct(), feat.to_string()).into_response(),
        Ok(None) => (StatusCode::NOT_FOUND, "not found".to_string()).into_response(),
        Err(err) => (StatusCode::INTERNAL_SERVER_ERROR, format!("error: {err}")).into_response(),
    }
}

async fn feature_collection_for_bbox(
    ctx: &MqttContext,
    set: &str,
    w: f64,
    s: f64,
    e: f64,
    n: f64,
) -> Result<Value> {
    let (cx, cy) = ((w + e) / 2.0, (s + n) / 2.0);
    let width_m = haversine_m(cy, w, cy, e);
    let height_m = haversine_m(s, cx, n, cx);
    let hits: Vec<GeoHit> = geosearch_box(ctx, set, cx, cy, width_m, height_m).await?;
    let mut features = Vec::with_capacity(hits.len());
    for hit in hits {
        let attrs = hgetall(ctx, &hit.member).await.unwrap_or_default();
        features.push(feature(&hit.member, hit.lat, hit.lng, attrs));
    }
    Ok(json!({ "type": "FeatureCollection", "features": features }))
}

async fn feature_for_object(ctx: &MqttContext, obid: &str) -> Result<Option<Value>> {
    // Search every geo set? We don't know which set. Require the hash to exist,
    // and report position if any geo-ish field is recorded via a companion key
    // `obj:<obid>:pos` storing "lon,lat". Fall back to just the hash as a non-
    // geo Feature.
    let attrs = hgetall(ctx, obid).await?;
    if attrs.is_empty() {
        return Ok(None);
    }
    let pos_key = format!("{}{}:pos", ctx.cfg.object_key_prefix, obid);
    let pos: Option<String> =
        fred::interfaces::KeysInterface::get(ctx.redis.client.as_ref(), &pos_key)
            .await
            .ok()
            .flatten();
    let (lon, lat) = match pos.and_then(parse_lonlat) {
        Some(p) => p,
        None => {
            // Try any known set: not possible without index. Return NoGeom feature.
            return Ok(Some(json!({
                "type": "Feature",
                "geometry": null,
                "properties": attrs,
                "id": obid,
            })));
        }
    };
    Ok(Some(feature(obid, lat, lon, attrs)))
}

fn parse_lonlat(s: String) -> Option<(f64, f64)> {
    let (a, b) = s.split_once(',')?;
    Some((a.parse().ok()?, b.parse().ok()?))
}

fn feature(id: &str, lat: f64, lng: f64, props: Map<String, Value>) -> Value {
    json!({
        "type": "Feature",
        "id": id,
        "geometry": { "type": "Point", "coordinates": [lng, lat] },
        "properties": props,
    })
}

fn haversine_m(lat1: f64, lon1: f64, lat2: f64, lon2: f64) -> f64 {
    const R: f64 = 6_371_000.0;
    let (phi1, phi2) = (lat1.to_radians(), lat2.to_radians());
    let dphi = (lat2 - lat1).to_radians();
    let dlambda = (lon2 - lon1).to_radians();
    let a = (dphi / 2.0).sin().powi(2) + phi1.cos() * phi2.cos() * (dlambda / 2.0).sin().powi(2);
    2.0 * R * a.sqrt().asin()
}
