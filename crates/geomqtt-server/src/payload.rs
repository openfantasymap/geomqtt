//! JSON payloads for MQTT tile and object topics (see PROTOCOL.md §4).

use serde::Serialize;
use serde_json::{Map, Value};

pub fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

fn is_empty(m: &Map<String, Value>) -> bool {
    m.is_empty()
}

#[derive(Serialize)]
#[serde(tag = "op", rename_all = "lowercase")]
#[allow(dead_code)] // `Attr` is reserved for tile-side attribute fanout (v0.2)
pub enum TilePayload {
    Snapshot {
        id: String,
        lat: f64,
        lng: f64,
        #[serde(skip_serializing_if = "is_empty")]
        attrs: Map<String, Value>,
        ts: u64,
    },
    Add {
        id: String,
        lat: f64,
        lng: f64,
        #[serde(skip_serializing_if = "is_empty")]
        attrs: Map<String, Value>,
        ts: u64,
    },
    Move {
        id: String,
        lat: f64,
        lng: f64,
        ts: u64,
    },
    Remove {
        id: String,
        ts: u64,
    },
    Attr {
        id: String,
        attrs: Map<String, Value>,
        ts: u64,
    },
}

impl TilePayload {
    pub fn snapshot(id: String, lat: f64, lng: f64, attrs: Map<String, Value>) -> Self {
        Self::Snapshot {
            id,
            lat,
            lng,
            attrs,
            ts: now_ms(),
        }
    }
    pub fn add(id: String, lat: f64, lng: f64, attrs: Map<String, Value>) -> Self {
        Self::Add {
            id,
            lat,
            lng,
            attrs,
            ts: now_ms(),
        }
    }
    pub fn move_(id: String, lat: f64, lng: f64) -> Self {
        Self::Move {
            id,
            lat,
            lng,
            ts: now_ms(),
        }
    }
    pub fn remove(id: String) -> Self {
        Self::Remove { id, ts: now_ms() }
    }
    #[allow(dead_code)]
    pub fn attr(id: String, attrs: Map<String, Value>) -> Self {
        Self::Attr {
            id,
            attrs,
            ts: now_ms(),
        }
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        serde_json::to_vec(self).expect("TilePayload serializes")
    }
}

#[derive(Serialize)]
#[serde(tag = "op", rename_all = "lowercase")]
pub enum ObjectPayload {
    Snapshot {
        id: String,
        attrs: Map<String, Value>,
        ts: u64,
    },
    Attr {
        id: String,
        attrs: Map<String, Value>,
        ts: u64,
    },
    Delete {
        id: String,
        ts: u64,
    },
}

impl ObjectPayload {
    pub fn snapshot(id: String, attrs: Map<String, Value>) -> Self {
        Self::Snapshot {
            id,
            attrs,
            ts: now_ms(),
        }
    }
    pub fn attr(id: String, attrs: Map<String, Value>) -> Self {
        Self::Attr {
            id,
            attrs,
            ts: now_ms(),
        }
    }
    pub fn delete(id: String) -> Self {
        Self::Delete { id, ts: now_ms() }
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        serde_json::to_vec(self).expect("ObjectPayload serializes")
    }
}

/// Topic helpers.
pub fn tile_topic(set: &str, z: u8, x: u32, y: u32) -> String {
    format!("geo/{set}/{z}/{x}/{y}")
}

#[allow(dead_code)]
pub fn object_topic(obid: &str) -> String {
    format!("objects/{obid}")
}

/// Channel name for cross-node Redis pub/sub (mirrors the MQTT topic 1:1).
pub fn redis_tile_channel(set: &str, z: u8, x: u32, y: u32) -> String {
    format!("gmq:tile:{set}:{z}:{x}:{y}")
}

pub fn redis_object_channel(obid: &str) -> String {
    format!("gmq:obj:{obid}")
}
