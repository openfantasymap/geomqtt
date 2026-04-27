//! Cheap counters wired into hot paths and rendered as Prometheus text on
//! demand. Each increment is a single `fetch_add(Relaxed)`; rendering walks
//! atomics and asks the broker for two `len()`s under one lock. Nothing
//! here makes a Redis round-trip or holds locks longer than needed.

use crate::broker::Broker;
use std::fmt::Write as _;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Instant;

pub struct Metrics {
    pub start: Instant,
    pub mqtt_connections_tcp: AtomicU64,
    pub mqtt_connections_ws: AtomicU64,
    pub mqtt_packets_in: AtomicU64,
    pub mqtt_publish_local: AtomicU64,
    pub resp_commands: AtomicU64,
    pub resp_geo_writes: AtomicU64,
    pub tile_fanouts: AtomicU64,
    pub object_fanouts: AtomicU64,
    pub redis_bridge_messages: AtomicU64,
    pub http_requests: AtomicU64,
}

impl Metrics {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            start: Instant::now(),
            mqtt_connections_tcp: AtomicU64::new(0),
            mqtt_connections_ws: AtomicU64::new(0),
            mqtt_packets_in: AtomicU64::new(0),
            mqtt_publish_local: AtomicU64::new(0),
            resp_commands: AtomicU64::new(0),
            resp_geo_writes: AtomicU64::new(0),
            tile_fanouts: AtomicU64::new(0),
            object_fanouts: AtomicU64::new(0),
            redis_bridge_messages: AtomicU64::new(0),
            http_requests: AtomicU64::new(0),
        })
    }

    pub fn render(&self, broker: &Broker, node_id: &str) -> String {
        let (sessions, subs) = broker.size();
        let uptime = self.start.elapsed().as_secs_f64();
        let load = |c: &AtomicU64| c.load(Ordering::Relaxed);

        let mut s = String::with_capacity(2048);
        let _ = writeln!(s, "# HELP geomqtt_build_info Build info (gauge always 1)");
        let _ = writeln!(s, "# TYPE geomqtt_build_info gauge");
        let _ = writeln!(
            s,
            "geomqtt_build_info{{version=\"{}\",node_id=\"{}\"}} 1",
            env!("CARGO_PKG_VERSION"),
            node_id
        );

        let _ = writeln!(s, "# HELP geomqtt_uptime_seconds Process uptime in seconds");
        let _ = writeln!(s, "# TYPE geomqtt_uptime_seconds gauge");
        let _ = writeln!(s, "geomqtt_uptime_seconds {uptime}");

        let _ = writeln!(
            s,
            "# HELP geomqtt_mqtt_sessions Active MQTT sessions on this node"
        );
        let _ = writeln!(s, "# TYPE geomqtt_mqtt_sessions gauge");
        let _ = writeln!(s, "geomqtt_mqtt_sessions {sessions}");

        let _ = writeln!(s, "# HELP geomqtt_mqtt_subscriptions Active MQTT topic-filter subscriptions across all sessions");
        let _ = writeln!(s, "# TYPE geomqtt_mqtt_subscriptions gauge");
        let _ = writeln!(s, "geomqtt_mqtt_subscriptions {subs}");

        let _ = writeln!(
            s,
            "# HELP geomqtt_mqtt_connections_total Cumulative MQTT connections accepted"
        );
        let _ = writeln!(s, "# TYPE geomqtt_mqtt_connections_total counter");
        let _ = writeln!(
            s,
            "geomqtt_mqtt_connections_total{{transport=\"tcp\"}} {}",
            load(&self.mqtt_connections_tcp)
        );
        let _ = writeln!(
            s,
            "geomqtt_mqtt_connections_total{{transport=\"ws\"}} {}",
            load(&self.mqtt_connections_ws)
        );

        let _ = writeln!(s, "# HELP geomqtt_mqtt_packets_received_total Cumulative MQTT packets parsed from clients");
        let _ = writeln!(s, "# TYPE geomqtt_mqtt_packets_received_total counter");
        let _ = writeln!(
            s,
            "geomqtt_mqtt_packets_received_total {}",
            load(&self.mqtt_packets_in)
        );

        let _ = writeln!(s, "# HELP geomqtt_mqtt_publish_local_total Cumulative PUBLISH broadcasts to local subscribers");
        let _ = writeln!(s, "# TYPE geomqtt_mqtt_publish_local_total counter");
        let _ = writeln!(
            s,
            "geomqtt_mqtt_publish_local_total {}",
            load(&self.mqtt_publish_local)
        );

        let _ = writeln!(
            s,
            "# HELP geomqtt_resp_commands_total Cumulative RESP commands proxied"
        );
        let _ = writeln!(s, "# TYPE geomqtt_resp_commands_total counter");
        let _ = writeln!(
            s,
            "geomqtt_resp_commands_total {}",
            load(&self.resp_commands)
        );

        let _ = writeln!(s, "# HELP geomqtt_resp_geo_writes_total Cumulative GEOADD/ZREM intercepts that triggered fanout");
        let _ = writeln!(s, "# TYPE geomqtt_resp_geo_writes_total counter");
        let _ = writeln!(
            s,
            "geomqtt_resp_geo_writes_total {}",
            load(&self.resp_geo_writes)
        );

        let _ = writeln!(s, "# HELP geomqtt_tile_fanouts_total Cumulative tile-topic publishes (geo/<set>/<z>/<x>/<y>)");
        let _ = writeln!(s, "# TYPE geomqtt_tile_fanouts_total counter");
        let _ = writeln!(s, "geomqtt_tile_fanouts_total {}", load(&self.tile_fanouts));

        let _ = writeln!(s, "# HELP geomqtt_object_fanouts_total Cumulative object-topic publishes (objects/<obid>)");
        let _ = writeln!(s, "# TYPE geomqtt_object_fanouts_total counter");
        let _ = writeln!(
            s,
            "geomqtt_object_fanouts_total {}",
            load(&self.object_fanouts)
        );

        let _ = writeln!(s, "# HELP geomqtt_redis_bridge_messages_total Cumulative cross-node pub/sub envelopes received and applied locally");
        let _ = writeln!(s, "# TYPE geomqtt_redis_bridge_messages_total counter");
        let _ = writeln!(
            s,
            "geomqtt_redis_bridge_messages_total {}",
            load(&self.redis_bridge_messages)
        );

        let _ = writeln!(
            s,
            "# HELP geomqtt_http_requests_total Cumulative HTTP requests served"
        );
        let _ = writeln!(s, "# TYPE geomqtt_http_requests_total counter");
        let _ = writeln!(
            s,
            "geomqtt_http_requests_total {}",
            load(&self.http_requests)
        );

        s
    }
}
