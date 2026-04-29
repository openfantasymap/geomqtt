//! Core fanout logic: given a geo event, publish to MQTT topics.
//!
//! Shared by the RESP proxy (on GEOADD / ZREM) and any other writer that
//! discovers position changes. Each call emits BOTH:
//!   * A local broker.publish_local (for sessions on this node).
//!   * A Redis PUBLISH on the matching `gmq:tile:*` channel wrapped in the
//!     node-id envelope (for other nodes' pub/sub bridges).

use crate::broker::Broker;
use crate::coord;
use crate::influx::InfluxClient;
use crate::payload::{
    redis_object_channel, redis_tile_channel, tile_topic, ObjectPayload, TilePayload,
};
use crate::redis::{build_envelope, RedisHandle};
use fred::interfaces::PubsubInterface;
use serde_json::{Map, Value};
use std::sync::Arc;
use tracing::warn;

pub struct Fanout {
    pub broker: Arc<Broker>,
    pub redis: RedisHandle,
    pub enrich_zooms: Vec<u8>,
    pub metrics: Arc<crate::metrics::Metrics>,
    pub influx: Option<Arc<InfluxClient>>,
}

impl Fanout {
    pub async fn publish_tile(&self, set: &str, z: u8, x: u32, y: u32, payload: &TilePayload) {
        let topic = tile_topic(set, z, x, y);
        let body = payload.to_bytes();
        self.broker.publish_local(&topic, body.clone().into());
        self.metrics
            .tile_fanouts
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let channel = redis_tile_channel(set, z, x, y);
        let envelope = build_envelope(&self.redis.node_id, &body);
        if let Err(e) = self
            .redis
            .client
            .publish::<i64, _, _>(channel, envelope)
            .await
        {
            warn!(error = %e, "redis publish failed");
        }
    }

    pub async fn publish_object(&self, obid: &str, payload: &ObjectPayload) {
        let topic = format!("objects/{obid}");
        let body = payload.to_bytes();
        self.broker.publish_local(&topic, body.clone().into());
        self.metrics
            .object_fanouts
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        if let (Some(influx), ObjectPayload::Attr { attrs, .. }) = (self.influx.as_ref(), payload) {
            influx.attr(obid, attrs);
        }
        let channel = redis_object_channel(obid);
        let envelope = build_envelope(&self.redis.node_id, &body);
        if let Err(e) = self
            .redis
            .client
            .publish::<i64, _, _>(channel, envelope)
            .await
        {
            warn!(error = %e, "redis publish failed");
        }
    }

    /// After a GEOADD: emit appropriate tile events.
    /// `old_pos` is the (lon, lat) the member had before, if any.
    /// `new_pos` is the new (lon, lat).
    pub async fn on_geo_write(
        &self,
        set: &str,
        member: &str,
        old_pos: Option<(f64, f64)>,
        new_pos: (f64, f64),
        attrs: Map<String, Value>,
    ) {
        let (new_lon, new_lat) = new_pos;
        if let Some(influx) = self.influx.as_ref() {
            influx.position(set, member, new_lon, new_lat);
        }
        let new_tiles = coord::tiles_for_point(&self.enrich_zooms, new_lat, new_lon);
        let old_tiles: Vec<(u8, u32, u32)> = old_pos
            .map(|(lon, lat)| coord::tiles_for_point(&self.enrich_zooms, lat, lon))
            .unwrap_or_default();

        for &(z, nx, ny) in &new_tiles {
            let old_at_z = old_tiles
                .iter()
                .find(|(oz, _, _)| *oz == z)
                .map(|(_, ox, oy)| (*ox, *oy));
            match old_at_z {
                None => {
                    let p = TilePayload::add(member.to_string(), new_lat, new_lon, attrs.clone());
                    self.publish_tile(set, z, nx, ny, &p).await;
                }
                Some((ox, oy)) if (ox, oy) == (nx, ny) => {
                    let p = TilePayload::move_(member.to_string(), new_lat, new_lon);
                    self.publish_tile(set, z, nx, ny, &p).await;
                }
                Some((ox, oy)) => {
                    let rm = TilePayload::remove(member.to_string());
                    self.publish_tile(set, z, ox, oy, &rm).await;
                    let add = TilePayload::add(member.to_string(), new_lat, new_lon, attrs.clone());
                    self.publish_tile(set, z, nx, ny, &add).await;
                }
            }
        }
    }

    /// After a ZREM on a geo set.
    pub async fn on_geo_remove(&self, set: &str, member: &str, old_pos: (f64, f64)) {
        let (lon, lat) = old_pos;
        for (z, x, y) in coord::tiles_for_point(&self.enrich_zooms, lat, lon) {
            let p = TilePayload::remove(member.to_string());
            self.publish_tile(set, z, x, y, &p).await;
        }
    }
}
