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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn parse(bytes: &[u8]) -> Value {
        serde_json::from_slice(bytes).unwrap()
    }

    #[test]
    fn tile_payload_shapes() {
        let p = TilePayload::snapshot("x".into(), 1.0, 2.0, Map::new());
        let v = parse(&p.to_bytes());
        assert_eq!(v["op"], "snapshot");
        assert_eq!(v["id"], "x");
        assert_eq!(v["lat"], json!(1.0));
        assert_eq!(v["lng"], json!(2.0));
        assert!(v.get("attrs").is_none(), "empty attrs must be skipped");

        let p = TilePayload::move_("x".into(), 1.0, 2.0);
        let v = parse(&p.to_bytes());
        assert_eq!(v["op"], "move");
        assert!(v.get("attrs").is_none());

        let p = TilePayload::remove("x".into());
        let v = parse(&p.to_bytes());
        assert_eq!(v["op"], "remove");
        assert_eq!(v["id"], "x");
        assert!(v.get("lat").is_none());
    }

    #[test]
    fn tile_payload_includes_attrs_when_present() {
        let mut attrs = Map::new();
        attrs.insert("icon".into(), json!("truck"));
        let p = TilePayload::add("x".into(), 1.0, 2.0, attrs);
        let v = parse(&p.to_bytes());
        assert_eq!(v["attrs"]["icon"], "truck");
    }

    #[test]
    fn object_payload_shapes() {
        let p = ObjectPayload::delete("x".into());
        let v = parse(&p.to_bytes());
        assert_eq!(v["op"], "delete");
        assert_eq!(v["id"], "x");
    }

    #[test]
    fn topic_helpers() {
        assert_eq!(
            tile_topic("vehicles", 10, 544, 370),
            "geo/vehicles/10/544/370"
        );
        assert_eq!(object_topic("veh-42"), "objects/veh-42");
        assert_eq!(
            redis_tile_channel("vehicles", 10, 544, 370),
            "gmq:tile:vehicles:10:544:370"
        );
        assert_eq!(redis_object_channel("veh-42"), "gmq:obj:veh-42");
    }
}
