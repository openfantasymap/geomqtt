//! Minimal MQTT v3.1.1 broker: QoS 0, clean session only.
//!
//! Both raw TCP (port from GEOMQTT_MQTT_ADDR) and WebSocket (port from
//! GEOMQTT_MQTT_WS_ADDR) listeners feed the same session state machine.

use crate::broker::{encode_publish, Broker, SessionId};
use crate::config::Config;
use crate::fanout::Fanout;
use crate::payload::TilePayload;
use crate::redis::RedisHandle;
use anyhow::{anyhow, Result};
use bytes::{Bytes, BytesMut};
use fred::interfaces::{GeoInterface, HashesInterface};
use fred::types::geo::GeoUnit;
use fred::types::SortOrder;
use fred::types::Value as FredValue;
use futures_util::{SinkExt, StreamExt};
use mqttbytes::v4::{
    ConnAck, ConnectReturnCode, Packet, Publish, SubAck, Subscribe, SubscribeReasonCode, UnsubAck,
    Unsubscribe,
};
use mqttbytes::QoS;
use serde_json::{Map, Value};
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::Message;
use tracing::{debug, info, warn};

const MAX_PACKET: usize = 1024 * 1024;

#[derive(Clone)]
pub struct MqttContext {
    pub broker: Arc<Broker>,
    pub redis: RedisHandle,
    pub cfg: Arc<Config>,
}

pub async fn serve(tcp_addr: SocketAddr, ws_addr: SocketAddr, ctx: MqttContext) -> Result<()> {
    let tcp = TcpListener::bind(tcp_addr).await?;
    let ws = TcpListener::bind(ws_addr).await?;
    info!(%tcp_addr, %ws_addr, "MQTT listeners bound");

    let tcp_ctx = ctx.clone();
    let ws_ctx = ctx.clone();

    let tcp_loop = async move {
        loop {
            let (sock, peer) = tcp.accept().await?;
            let ctx = tcp_ctx.clone();
            tokio::spawn(async move {
                if let Err(e) = handle_tcp(sock, ctx).await {
                    debug!(%peer, error = %e, "tcp session ended");
                }
            });
        }
        #[allow(unreachable_code)]
        Ok::<(), anyhow::Error>(())
    };
    let ws_loop = async move {
        loop {
            let (sock, peer) = ws.accept().await?;
            let ctx = ws_ctx.clone();
            tokio::spawn(async move {
                if let Err(e) = handle_ws(sock, ctx).await {
                    debug!(%peer, error = %e, "ws session ended");
                }
            });
        }
        #[allow(unreachable_code)]
        Ok::<(), anyhow::Error>(())
    };

    tokio::try_join!(tcp_loop, ws_loop)?;
    Ok(())
}

async fn handle_tcp(mut sock: TcpStream, ctx: MqttContext) -> Result<()> {
    let (outbound_tx, mut outbound_rx) = mpsc::unbounded_channel::<Bytes>();
    let session_id = ctx.broker.register(outbound_tx.clone());
    let ctx_inbound = ctx.clone();

    let mut buf = BytesMut::with_capacity(4096);
    let mut connect_seen = false;

    // Read loop inlined to avoid splitting; use select on outbound.
    let broker_for_cleanup = ctx.broker.clone();
    let result = async {
        loop {
            tokio::select! {
                r = sock.read_buf(&mut buf) => {
                    let n = r?;
                    if n == 0 { return Ok::<(), anyhow::Error>(()); }
                    loop {
                        match mqttbytes::v4::read(&mut buf, MAX_PACKET) {
                            Ok(pkt) => {
                                handle_packet(pkt, session_id, &outbound_tx, &ctx_inbound, &mut connect_seen).await?;
                            }
                            Err(mqttbytes::Error::InsufficientBytes(_)) => break,
                            Err(e) => return Err(anyhow!("mqtt parse: {:?}", e)),
                        }
                    }
                }
                maybe = outbound_rx.recv() => {
                    let Some(bytes) = maybe else { return Ok(()); };
                    sock.write_all(&bytes).await?;
                }
            }
        }
    }
    .await;
    broker_for_cleanup.deregister(session_id);
    result
}

async fn handle_ws(sock: TcpStream, ctx: MqttContext) -> Result<()> {
    #[allow(clippy::result_large_err)] // the ErrorResponse type is imposed by tungstenite
    let callback =
        |_req: &tokio_tungstenite::tungstenite::handshake::server::Request,
         mut resp: tokio_tungstenite::tungstenite::handshake::server::Response| {
            // Accept either "mqtt" or "mqttv3.1" subprotocols if the client requests one.
            resp.headers_mut()
                .insert("Sec-WebSocket-Protocol", "mqtt".parse().unwrap());
            Ok(resp)
        };
    let ws = tokio_tungstenite::accept_hdr_async(sock, callback).await?;
    let (mut ws_tx, mut ws_rx) = ws.split();
    let (outbound_tx, mut outbound_rx) = mpsc::unbounded_channel::<Bytes>();
    let session_id = ctx.broker.register(outbound_tx.clone());
    let broker_for_cleanup = ctx.broker.clone();
    let ctx_inbound = ctx.clone();

    let mut buf = BytesMut::with_capacity(4096);
    let mut connect_seen = false;

    let result = async {
        loop {
            tokio::select! {
                maybe_msg = ws_rx.next() => {
                    let Some(msg) = maybe_msg else { return Ok::<(), anyhow::Error>(()); };
                    let msg = msg?;
                    match msg {
                        Message::Binary(data) => {
                            buf.extend_from_slice(&data);
                            loop {
                                match mqttbytes::v4::read(&mut buf, MAX_PACKET) {
                                    Ok(pkt) => handle_packet(pkt, session_id, &outbound_tx, &ctx_inbound, &mut connect_seen).await?,
                                    Err(mqttbytes::Error::InsufficientBytes(_)) => break,
                                    Err(e) => return Err(anyhow!("mqtt parse: {:?}", e)),
                                }
                            }
                        }
                        Message::Close(_) => return Ok(()),
                        Message::Ping(p) => ws_tx.send(Message::Pong(p)).await?,
                        _ => {}
                    }
                }
                maybe = outbound_rx.recv() => {
                    let Some(bytes) = maybe else { return Ok(()); };
                    ws_tx.send(Message::Binary(bytes.to_vec().into())).await?;
                }
            }
        }
    }
    .await;
    broker_for_cleanup.deregister(session_id);
    result
}

async fn handle_packet(
    pkt: Packet,
    session_id: SessionId,
    outbound: &mpsc::UnboundedSender<Bytes>,
    ctx: &MqttContext,
    connect_seen: &mut bool,
) -> Result<()> {
    match pkt {
        Packet::Connect(_) => {
            *connect_seen = true;
            let mut buf = BytesMut::new();
            ConnAck::new(ConnectReturnCode::Success, false)
                .write(&mut buf)
                .map_err(|e| anyhow!("connack write: {:?}", e))?;
            let _ = outbound.send(buf.freeze());
        }
        Packet::Subscribe(Subscribe { pkid, filters }) => {
            if !*connect_seen {
                return Err(anyhow!("SUBSCRIBE before CONNECT"));
            }
            let mut return_codes = Vec::with_capacity(filters.len());
            let mut snapshot_targets = Vec::new();
            for f in &filters {
                ctx.broker.subscribe(session_id, f.path.clone());
                return_codes.push(SubscribeReasonCode::Success(QoS::AtMostOnce));
                snapshot_targets.push(f.path.clone());
            }
            let mut buf = BytesMut::new();
            SubAck::new(pkid, return_codes)
                .write(&mut buf)
                .map_err(|e| anyhow!("suback write: {:?}", e))?;
            let _ = outbound.send(buf.freeze());

            // Snapshot burst. Spawn so a slow GEOSEARCH doesn't block the read loop.
            for filter in snapshot_targets {
                let ctx = ctx.clone();
                let outbound = outbound.clone();
                tokio::spawn(async move {
                    if let Err(e) = snapshot_for_filter(&filter, session_id, &outbound, &ctx).await
                    {
                        warn!(filter, error = %e, "snapshot burst failed");
                    }
                });
            }
        }
        Packet::Unsubscribe(Unsubscribe { pkid, topics, .. }) => {
            for f in &topics {
                ctx.broker.unsubscribe(session_id, f);
            }
            let mut buf = BytesMut::new();
            UnsubAck::new(pkid)
                .write(&mut buf)
                .map_err(|e| anyhow!("unsuback write: {:?}", e))?;
            let _ = outbound.send(buf.freeze());
        }
        Packet::Publish(Publish { topic, payload, .. }) => {
            // A client publishing on MQTT — fan out to other local sessions and cross-node.
            let fanout = Fanout {
                broker: ctx.broker.clone(),
                redis: ctx.redis.clone(),
                enrich_zooms: ctx.cfg.enrich_zooms.clone(),
            };
            ctx.broker.publish_local(&topic, payload.clone());
            let channel = format!("gmq:user:{topic}");
            let envelope = crate::redis::build_envelope(&ctx.redis.node_id, &payload);
            let _ = fred::interfaces::PubsubInterface::publish::<i64, _, _>(
                fanout.redis.client.as_ref(),
                channel,
                envelope,
            )
            .await;
        }
        Packet::PingReq => {
            let mut buf = BytesMut::new();
            buf.extend_from_slice(&[0xD0, 0x00]);
            let _ = outbound.send(buf.freeze());
        }
        Packet::Disconnect => return Err(anyhow!("client disconnect")),
        other => {
            debug!(?other, "ignoring unsupported packet");
        }
    }
    Ok(())
}

/// On SUB to `geo/<set>/<z>/<x>/<y>` or `objects/<obid>`, replay current state.
async fn snapshot_for_filter(
    filter: &str,
    session_id: SessionId,
    outbound: &mpsc::UnboundedSender<Bytes>,
    ctx: &MqttContext,
) -> Result<()> {
    if let Some(parts) = parse_tile_filter(filter) {
        let (set, z, x, y) = parts;
        snapshot_tile(set, z, x, y, session_id, outbound, ctx).await?;
    } else if let Some(obid) = parse_object_filter(filter) {
        snapshot_object(obid, session_id, outbound, ctx).await?;
    }
    Ok(())
}

fn parse_tile_filter(filter: &str) -> Option<(&str, u8, u32, u32)> {
    let parts: Vec<&str> = filter.splitn(5, '/').collect();
    if parts.len() != 5 {
        return None;
    }
    if parts[0] != "geo" {
        return None;
    }
    let z: u8 = parts[2].parse().ok()?;
    let x: u32 = parts[3].parse().ok()?;
    let y: u32 = parts[4].parse().ok()?;
    Some((parts[1], z, x, y))
}

fn parse_object_filter(filter: &str) -> Option<&str> {
    filter.strip_prefix("objects/")
}

async fn snapshot_tile(
    set: &str,
    z: u8,
    x: u32,
    y: u32,
    session_id: SessionId,
    _outbound: &mpsc::UnboundedSender<Bytes>,
    ctx: &MqttContext,
) -> Result<()> {
    let (w, s, e, n) = crate::coord::bbox_for_tile(z, x, y);
    let (cx, cy) = ((w + e) / 2.0, (s + n) / 2.0);
    let width_m = haversine_m(cy, w, cy, e);
    let height_m = haversine_m(s, cx, n, cx);
    let members = geosearch_box(ctx, set, cx, cy, width_m, height_m).await?;

    let topic = format!("geo/{set}/{z}/{x}/{y}");
    for m in members {
        let attrs = hmget_enrich(ctx, &m.member).await.unwrap_or_default();
        let payload = TilePayload::snapshot(m.member, m.lat, m.lng, attrs);
        let body = payload.to_bytes();
        let bytes = encode_publish(&topic, &body);
        ctx.broker.send_direct(session_id, bytes);
    }
    Ok(())
}

async fn snapshot_object(
    obid: &str,
    session_id: SessionId,
    _outbound: &mpsc::UnboundedSender<Bytes>,
    ctx: &MqttContext,
) -> Result<()> {
    let topic = format!("objects/{obid}");
    let attrs = hgetall(ctx, obid).await.unwrap_or_default();
    if attrs.is_empty() {
        return Ok(());
    }
    let payload = crate::payload::ObjectPayload::snapshot(obid.to_string(), attrs);
    let body = payload.to_bytes();
    let bytes = encode_publish(&topic, &body);
    ctx.broker.send_direct(session_id, bytes);
    Ok(())
}

pub struct GeoHit {
    pub member: String,
    pub lat: f64,
    pub lng: f64,
}

pub async fn geosearch_box(
    ctx: &MqttContext,
    set: &str,
    center_lon: f64,
    center_lat: f64,
    width_m: f64,
    height_m: f64,
) -> Result<Vec<GeoHit>> {
    use fred::types::geo::GeoPosition;
    let pos = GeoPosition {
        longitude: center_lon,
        latitude: center_lat,
    };
    let raw: FredValue = ctx
        .redis
        .client
        .geosearch(
            set,
            None,
            Some(pos),
            None,
            Some((width_m, height_m, GeoUnit::Meters)),
            Some(SortOrder::Asc),
            None,
            true,
            false,
            false,
        )
        .await
        .map_err(|e| anyhow!("geosearch: {:?}", e))?;
    parse_geosearch(raw)
}

fn parse_geosearch(v: FredValue) -> Result<Vec<GeoHit>> {
    let items = match v {
        FredValue::Array(a) => a,
        FredValue::Null => return Ok(vec![]),
        other => return Err(anyhow!("geosearch unexpected: {:?}", other)),
    };
    let mut out = Vec::with_capacity(items.len());
    for item in items {
        let arr = match item {
            FredValue::Array(a) => a,
            _ => continue,
        };
        if arr.len() < 2 {
            continue;
        }
        let member = value_to_string(&arr[0]).unwrap_or_default();
        let pos = match &arr[1] {
            FredValue::Array(p) if p.len() == 2 => p,
            _ => continue,
        };
        let lon = value_to_f64(&pos[0]).unwrap_or_default();
        let lat = value_to_f64(&pos[1]).unwrap_or_default();
        out.push(GeoHit {
            member,
            lat,
            lng: lon,
        });
    }
    Ok(out)
}

fn value_to_string(v: &FredValue) -> Option<String> {
    match v {
        FredValue::String(s) => Some(s.to_string()),
        FredValue::Bytes(b) => Some(String::from_utf8_lossy(b).to_string()),
        _ => None,
    }
}

fn value_to_f64(v: &FredValue) -> Option<f64> {
    match v {
        FredValue::Double(d) => Some(*d),
        FredValue::Integer(i) => Some(*i as f64),
        FredValue::String(s) => s.parse().ok(),
        FredValue::Bytes(b) => std::str::from_utf8(b).ok()?.parse().ok(),
        _ => None,
    }
}

pub async fn hgetall(ctx: &MqttContext, obid: &str) -> Result<Map<String, Value>> {
    let key = format!("{}{}", ctx.cfg.object_key_prefix, obid);
    let map: HashMap<String, String> = ctx
        .redis
        .client
        .hgetall(&key)
        .await
        .map_err(|e| anyhow!("hgetall: {:?}", e))?;
    Ok(map
        .into_iter()
        .map(|(k, v)| (k, Value::String(v)))
        .collect())
}

pub async fn hmget_enrich(ctx: &MqttContext, obid: &str) -> Result<Map<String, Value>> {
    if ctx.cfg.enrich_attrs.is_empty() {
        return Ok(Map::new());
    }
    let key = format!("{}{}", ctx.cfg.object_key_prefix, obid);
    let fields: Vec<String> = ctx.cfg.enrich_attrs.clone();
    let vals: Vec<Option<String>> = ctx
        .redis
        .client
        .hmget(&key, fields.clone())
        .await
        .map_err(|e| anyhow!("hmget: {:?}", e))?;
    let mut out = Map::new();
    for (k, v) in fields.into_iter().zip(vals) {
        if let Some(s) = v {
            out.insert(k, Value::String(s));
        }
    }
    Ok(out)
}

fn haversine_m(lat1: f64, lon1: f64, lat2: f64, lon2: f64) -> f64 {
    const R: f64 = 6_371_000.0;
    let (phi1, phi2) = (lat1.to_radians(), lat2.to_radians());
    let dphi = (lat2 - lat1).to_radians();
    let dlambda = (lon2 - lon1).to_radians();
    let a = (dphi / 2.0).sin().powi(2) + phi1.cos() * phi2.cos() * (dlambda / 2.0).sin().powi(2);
    2.0 * R * a.sqrt().asin()
}
