//! Upstream Redis client + sharded pub/sub coordination.

use crate::config::Config;
use anyhow::{Context, Result};
use fred::clients::{Client, SubscriberClient};
use fred::interfaces::*;
use fred::types::config::Config as FredConfig;
use std::sync::Arc;
use tracing::{info, warn};

#[derive(Clone)]
pub struct RedisHandle {
    pub client: Arc<Client>,
    pub subscriber: Arc<SubscriberClient>,
    pub node_id: Arc<str>,
}

pub async fn connect(cfg: &Config) -> Result<RedisHandle> {
    let fred_config = FredConfig::from_url(&cfg.redis_url).context("parsing GEOMQTT_REDIS_URL")?;
    let client = Client::new(fred_config.clone(), None, None, None);
    client
        .init()
        .await
        .context("connect upstream Redis (command client)")?;
    let pong: String = client.ping(None).await.context("PING upstream")?;
    info!(%pong, "upstream Redis reachable");

    let subscriber = SubscriberClient::new(fred_config, None, None, None);
    subscriber
        .init()
        .await
        .context("connect upstream Redis (subscriber client)")?;
    subscriber.manage_subscriptions();

    let node_id: Arc<str> = Arc::from(make_node_id().as_str());
    info!(%node_id, "geomqtt node id");

    Ok(RedisHandle {
        client: Arc::new(client),
        subscriber: Arc::new(subscriber),
        node_id,
    })
}

/// Background task: listen for cross-node tile/object publishes on Redis and
/// re-publish them on the LOCAL broker only. Messages originating from this
/// same node are filtered by `node_id` prefix.
pub async fn run_pubsub_bridge(
    redis: RedisHandle,
    broker: Arc<crate::broker::Broker>,
) -> Result<()> {
    // Subscribe to all tile and object channels via patterns.
    redis
        .subscriber
        .psubscribe(vec!["gmq:tile:*", "gmq:obj:*"])
        .await
        .context("psubscribe cross-node channels")?;
    let mut rx = redis.subscriber.message_rx();
    info!("cross-node pub/sub bridge up");
    let my_id = redis.node_id.clone();
    loop {
        match rx.recv().await {
            Ok(msg) => {
                let payload_bytes: Vec<u8> = match msg.value.into_bytes() {
                    Some(b) => b.to_vec(),
                    None => continue,
                };
                let Some((src, body)) = split_envelope(&payload_bytes) else {
                    continue;
                };
                if src == my_id.as_ref() {
                    continue; // local echo — we already published on the originating node
                }
                let channel = msg.channel.to_string();
                let Some(topic) = redis_channel_to_mqtt_topic(&channel) else {
                    continue;
                };
                broker.publish_local(&topic, body.to_vec().into());
            }
            Err(e) => {
                warn!(error = %e, "pub/sub bridge recv error; retrying");
                tokio::time::sleep(std::time::Duration::from_millis(200)).await;
            }
        }
    }
}

fn split_envelope(buf: &[u8]) -> Option<(&str, &[u8])> {
    let pos = buf.iter().position(|b| *b == b'|')?;
    let src = std::str::from_utf8(&buf[..pos]).ok()?;
    Some((src, &buf[pos + 1..]))
}

pub fn build_envelope(node_id: &str, payload: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(node_id.len() + 1 + payload.len());
    out.extend_from_slice(node_id.as_bytes());
    out.push(b'|');
    out.extend_from_slice(payload);
    out
}

fn make_node_id() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let pid = std::process::id();
    format!("{pid:x}-{:x}", nanos as u64)
}

fn redis_channel_to_mqtt_topic(channel: &str) -> Option<String> {
    if let Some(rest) = channel.strip_prefix("gmq:tile:") {
        // set:z:x:y -> geo/set/z/x/y
        let parts: Vec<&str> = rest.splitn(4, ':').collect();
        if parts.len() != 4 {
            return None;
        }
        Some(format!(
            "geo/{}/{}/{}/{}",
            parts[0], parts[1], parts[2], parts[3]
        ))
    } else {
        channel
            .strip_prefix("gmq:obj:")
            .map(|rest| format!("objects/{rest}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn envelope_roundtrip() {
        let bytes = build_envelope("node-a", b"{\"op\":\"add\"}");
        let (src, body) = split_envelope(&bytes).unwrap();
        assert_eq!(src, "node-a");
        assert_eq!(body, b"{\"op\":\"add\"}");
    }

    #[test]
    fn envelope_missing_separator() {
        assert!(split_envelope(b"no-pipe-here").is_none());
    }

    #[test]
    fn envelope_handles_payload_with_pipes() {
        // Separator is the FIRST '|', so payloads with embedded '|' survive.
        let bytes = build_envelope("n", b"{\"k\":\"a|b|c\"}");
        let (src, body) = split_envelope(&bytes).unwrap();
        assert_eq!(src, "n");
        assert_eq!(body, b"{\"k\":\"a|b|c\"}");
    }

    #[test]
    fn channel_to_topic_tile() {
        assert_eq!(
            redis_channel_to_mqtt_topic("gmq:tile:vehicles:10:544:370").as_deref(),
            Some("geo/vehicles/10/544/370")
        );
    }

    #[test]
    fn channel_to_topic_object() {
        assert_eq!(
            redis_channel_to_mqtt_topic("gmq:obj:veh-42").as_deref(),
            Some("objects/veh-42")
        );
    }

    #[test]
    fn channel_to_topic_unknown_returns_none() {
        assert!(redis_channel_to_mqtt_topic("some:other").is_none());
    }
}
