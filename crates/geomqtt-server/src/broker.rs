//! In-memory MQTT broker state: sessions, subscriptions, local delivery.

use bytes::{Bytes, BytesMut};
use mqttbytes::v4::Publish;
use mqttbytes::QoS;
use parking_lot::Mutex;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::mpsc;
use tracing::debug;

pub type SessionId = u64;

pub struct Broker {
    inner: Mutex<Inner>,
}

struct Inner {
    next_id: SessionId,
    sessions: HashMap<SessionId, Session>,
}

struct Session {
    outbound: mpsc::UnboundedSender<Bytes>,
    subs: Vec<String>, // topic filters
}

impl Broker {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            inner: Mutex::new(Inner {
                next_id: 1,
                sessions: HashMap::new(),
            }),
        })
    }

    pub fn register(&self, outbound: mpsc::UnboundedSender<Bytes>) -> SessionId {
        let mut inner = self.inner.lock();
        let id = inner.next_id;
        inner.next_id += 1;
        inner.sessions.insert(
            id,
            Session {
                outbound,
                subs: Vec::new(),
            },
        );
        debug!(session = id, "session registered");
        id
    }

    pub fn deregister(&self, id: SessionId) {
        let mut inner = self.inner.lock();
        inner.sessions.remove(&id);
        debug!(session = id, "session deregistered");
    }

    pub fn subscribe(&self, id: SessionId, filter: String) {
        let mut inner = self.inner.lock();
        if let Some(sess) = inner.sessions.get_mut(&id) {
            if !sess.subs.iter().any(|f| f == &filter) {
                sess.subs.push(filter);
            }
        }
    }

    pub fn unsubscribe(&self, id: SessionId, filter: &str) {
        let mut inner = self.inner.lock();
        if let Some(sess) = inner.sessions.get_mut(&id) {
            sess.subs.retain(|f| f != filter);
        }
    }

    /// List of topic filters currently subscribed across all sessions.
    /// Used to decide whether a given tile publish has any local audience.
    #[allow(dead_code)]
    pub fn has_local_subscriber_for(&self, topic: &str) -> bool {
        let inner = self.inner.lock();
        inner
            .sessions
            .values()
            .any(|s| s.subs.iter().any(|f| mqttbytes::matches(topic, f)))
    }

    /// Fan out one message to all locally-connected sessions with a matching filter.
    /// Returns the number of sessions that received the message.
    pub fn publish_local(&self, topic: &str, payload: Bytes) -> usize {
        let bytes = encode_publish(topic, &payload);
        let inner = self.inner.lock();
        let mut delivered = 0usize;
        for sess in inner.sessions.values() {
            if sess.subs.iter().any(|f| mqttbytes::matches(topic, f))
                && sess.outbound.send(bytes.clone()).is_ok()
            {
                delivered += 1;
            }
        }
        delivered
    }

    /// Send raw bytes (already an encoded PUBLISH) to ONE specific session.
    /// Used for per-subscriber snapshot bursts.
    pub fn send_direct(&self, id: SessionId, bytes: Bytes) {
        let inner = self.inner.lock();
        if let Some(sess) = inner.sessions.get(&id) {
            let _ = sess.outbound.send(bytes);
        }
    }
}

pub fn encode_publish(topic: &str, payload: &[u8]) -> Bytes {
    let publish = Publish::new(topic, QoS::AtMostOnce, payload.to_vec());
    let mut buf = BytesMut::with_capacity(2 + topic.len() + payload.len() + 8);
    publish.write(&mut buf).expect("Publish::write");
    buf.freeze()
}
