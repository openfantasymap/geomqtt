//! RESP (Redis) protocol listener — an intercepting proxy in front of upstream Redis.
//!
//! Incoming RESP2 commands are parsed, the commands we care about (GEOADD, ZREM,
//! HSET, HDEL, DEL on obj:* keys) trigger MQTT fanout, and every command is
//! forwarded to the upstream Redis via fred's `custom_raw` path. Responses are
//! re-encoded as RESP2 and sent back to the client verbatim.

use crate::broker::Broker;
use crate::config::Config;
use crate::fanout::Fanout;
use crate::payload::ObjectPayload;
use crate::redis::RedisHandle;
use anyhow::{anyhow, Result};
use bytes::{Bytes, BytesMut};
use fred::interfaces::{ClientLike, GeoInterface};
use fred::types::{ClusterHash, CustomCommand, Value as FredValue};
use redis_protocol::resp2::decode::decode_bytes_mut;
use redis_protocol::resp2::encode::encode_bytes;
use redis_protocol::resp2::types::BytesFrame as Resp2Frame;
use redis_protocol::resp3::types::BytesFrame as Resp3Frame;
use serde_json::{Map, Value};
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tracing::{debug, info, warn};

#[derive(Clone)]
pub struct RespContext {
    pub broker: Arc<Broker>,
    pub redis: RedisHandle,
    pub cfg: Arc<Config>,
}

pub async fn serve(addr: SocketAddr, ctx: RespContext) -> Result<()> {
    let listener = TcpListener::bind(addr).await?;
    info!(%addr, "RESP listener bound");
    loop {
        let (sock, peer) = listener.accept().await?;
        let ctx = ctx.clone();
        tokio::spawn(async move {
            if let Err(e) = handle_client(sock, ctx).await {
                debug!(%peer, error = %e, "RESP session closed");
            }
        });
    }
}

async fn handle_client(mut sock: TcpStream, ctx: RespContext) -> Result<()> {
    let mut buf = BytesMut::with_capacity(4096);
    loop {
        let n = sock.read_buf(&mut buf).await?;
        if n == 0 {
            return Ok(());
        }
        loop {
            match decode_bytes_mut(&mut buf) {
                Ok(Some((frame, _consumed, _bytes))) => {
                    let response = handle_command(frame, &ctx).await;
                    let out = encode_resp2(&response);
                    sock.write_all(&out).await?;
                }
                Ok(None) => break, // need more bytes
                Err(e) => return Err(anyhow!("resp2 decode: {:?}", e)),
            }
        }
    }
}

async fn handle_command(frame: Resp2Frame, ctx: &RespContext) -> Resp2Frame {
    let args = match frame_to_args(&frame) {
        Some(a) if !a.is_empty() => a,
        _ => return Resp2Frame::Error("ERR empty or malformed command".into()),
    };
    let cmd = args[0].to_ascii_uppercase();

    // Pre-interception: capture old position for GEOADD/ZREM.
    let mut pre_positions: Vec<Option<(f64, f64)>> = Vec::new();
    let mut pre_set: Option<String> = None;
    let mut pre_members: Vec<String> = Vec::new();

    // GEOADD key [NX|XX] [CH] lon lat member [lon lat member ...]
    if cmd == "GEOADD" && args.len() >= 5 {
        let set = args[1].clone();
        let (members, _) = extract_geoadd_members(&args[2..]);
        if !members.is_empty() {
            match geopos_batch(&ctx.redis, &set, &members).await {
                Ok(pos) => {
                    pre_positions = pos;
                    pre_set = Some(set);
                    pre_members = members;
                }
                Err(e) => warn!(error = %e, "GEOPOS prefetch failed"),
            }
        }
    } else if cmd == "ZREM" && args.len() >= 3 {
        let set = args[1].clone();
        let members: Vec<String> = args[2..].to_vec();
        match geopos_batch(&ctx.redis, &set, &members).await {
            Ok(pos) => {
                pre_positions = pos;
                pre_set = Some(set);
                pre_members = members;
            }
            Err(e) => warn!(error = %e, "GEOPOS prefetch (zrem) failed"),
        }
    }

    // Forward every command to the upstream Redis.
    let response = forward(&ctx.redis, &args).await;

    // Post-interception: fan out on success.
    if matches!(&response, Resp2Frame::Error(_)) {
        return response;
    }
    let fanout = Fanout {
        broker: ctx.broker.clone(),
        redis: ctx.redis.clone(),
        enrich_zooms: ctx.cfg.enrich_zooms.clone(),
    };
    match cmd.as_str() {
        "GEOADD" => {
            if let Some(set) = pre_set {
                let (members, new_positions) = extract_geoadd_members(&args[2..]);
                for (i, member) in members.iter().enumerate() {
                    let new = new_positions[i];
                    let old = pre_positions.get(i).copied().flatten();
                    let attrs = crate::mqtt::hmget_enrich(
                        &crate::mqtt::MqttContext {
                            broker: ctx.broker.clone(),
                            redis: ctx.redis.clone(),
                            cfg: ctx.cfg.clone(),
                        },
                        member,
                    )
                    .await
                    .unwrap_or_default();
                    fanout.on_geo_write(&set, member, old, new, attrs).await;
                }
            }
        }
        "ZREM" => {
            if let Some(set) = pre_set {
                for (i, member) in pre_members.iter().enumerate() {
                    if let Some(Some(pos)) = pre_positions.get(i) {
                        fanout.on_geo_remove(&set, member, *pos).await;
                    }
                }
            }
        }
        "HSET" | "HMSET" => handle_hash_write(ctx, &fanout, &args, false).await,
        "HDEL" => handle_hash_write(ctx, &fanout, &args, true).await,
        "DEL" => handle_del(ctx, &fanout, &args).await,
        _ => {}
    }

    response
}

fn extract_geoadd_members(tail: &[String]) -> (Vec<String>, Vec<(f64, f64)>) {
    // Skip leading flags: NX, XX, CH
    let mut i = 0;
    while i < tail.len() {
        let u = tail[i].to_ascii_uppercase();
        if matches!(u.as_str(), "NX" | "XX" | "CH") {
            i += 1;
        } else {
            break;
        }
    }
    let rest = &tail[i..];
    let mut members = Vec::new();
    let mut positions = Vec::new();
    let mut j = 0;
    while j + 2 < rest.len() {
        let lon: f64 = rest[j].parse().unwrap_or(0.0);
        let lat: f64 = rest[j + 1].parse().unwrap_or(0.0);
        let member = rest[j + 2].clone();
        members.push(member);
        positions.push((lon, lat));
        j += 3;
    }
    (members, positions)
}

async fn geopos_batch(
    redis: &RedisHandle,
    set: &str,
    members: &[String],
) -> Result<Vec<Option<(f64, f64)>>> {
    let positions: Vec<Option<(f64, f64)>> = redis
        .client
        .geopos(set, members.to_vec())
        .await
        .map_err(|e| anyhow!("geopos: {:?}", e))?;
    Ok(positions)
}

async fn handle_hash_write(ctx: &RespContext, fanout: &Fanout, args: &[String], is_del: bool) {
    if args.len() < 2 {
        return;
    }
    let key = &args[1];
    let Some(obid) = key.strip_prefix(&*ctx.cfg.object_key_prefix) else {
        return;
    };
    // Build the attrs delta payload.
    let mut attrs = Map::new();
    if is_del {
        // HDEL obj:foo field [field ...] — we don't have the new value; omit attrs.
    } else {
        let mut i = 2;
        while i + 1 < args.len() {
            attrs.insert(args[i].clone(), Value::String(args[i + 1].clone()));
            i += 2;
        }
    }
    let payload = ObjectPayload::attr(obid.to_string(), attrs);
    fanout.publish_object(obid, &payload).await;
}

async fn handle_del(ctx: &RespContext, fanout: &Fanout, args: &[String]) {
    for key in &args[1..] {
        if let Some(obid) = key.strip_prefix(&*ctx.cfg.object_key_prefix) {
            let payload = ObjectPayload::delete(obid.to_string());
            fanout.publish_object(obid, &payload).await;
        }
    }
}

fn frame_to_args(frame: &Resp2Frame) -> Option<Vec<String>> {
    let Resp2Frame::Array(items) = frame else {
        return None;
    };
    let mut out = Vec::with_capacity(items.len());
    for it in items {
        match it {
            Resp2Frame::BulkString(b) => out.push(String::from_utf8_lossy(b).to_string()),
            Resp2Frame::SimpleString(b) => out.push(String::from_utf8_lossy(b).to_string()),
            _ => return None,
        }
    }
    Some(out)
}

async fn forward(redis: &RedisHandle, args: &[String]) -> Resp2Frame {
    let cmd_name = args[0].clone();
    let tail: Vec<FredValue> = args[1..]
        .iter()
        .map(|s| FredValue::String(s.clone().into()))
        .collect();
    let cc = CustomCommand::new(cmd_name, ClusterHash::FirstKey, false);
    match redis.client.custom_raw(cc, tail).await {
        Ok(frame) => resp3_to_resp2(frame),
        Err(e) => Resp2Frame::Error(format!("ERR upstream: {e}").into()),
    }
}

fn resp3_to_resp2(frame: Resp3Frame) -> Resp2Frame {
    match frame {
        Resp3Frame::SimpleString { data, .. } => Resp2Frame::SimpleString(data),
        Resp3Frame::BlobString { data, .. } => Resp2Frame::BulkString(data),
        Resp3Frame::SimpleError { data, .. } => Resp2Frame::Error(data),
        Resp3Frame::BlobError { data, .. } => {
            Resp2Frame::Error(String::from_utf8_lossy(&data).to_string().into())
        }
        Resp3Frame::Number { data, .. } => Resp2Frame::Integer(data),
        Resp3Frame::Double { data, .. } => {
            Resp2Frame::BulkString(Bytes::from(data.to_string().into_bytes()))
        }
        Resp3Frame::Boolean { data, .. } => Resp2Frame::Integer(if data { 1 } else { 0 }),
        Resp3Frame::Null => Resp2Frame::Null,
        Resp3Frame::BigNumber { data, .. } => Resp2Frame::BulkString(data),
        Resp3Frame::Array { data, .. } | Resp3Frame::Push { data, .. } => {
            Resp2Frame::Array(data.into_iter().map(resp3_to_resp2).collect())
        }
        Resp3Frame::Set { data, .. } => {
            Resp2Frame::Array(data.into_iter().map(resp3_to_resp2).collect())
        }
        Resp3Frame::Map { data, .. } => {
            let mut out = Vec::with_capacity(data.len() * 2);
            for (k, v) in data {
                out.push(resp3_to_resp2(k));
                out.push(resp3_to_resp2(v));
            }
            Resp2Frame::Array(out)
        }
        Resp3Frame::VerbatimString { data, .. } => Resp2Frame::BulkString(data),
        Resp3Frame::ChunkedString(data) => Resp2Frame::BulkString(data),
        Resp3Frame::Hello { .. } => Resp2Frame::SimpleString(Bytes::from_static(b"OK")),
    }
}

fn encode_resp2(frame: &Resp2Frame) -> Vec<u8> {
    let mut buf = vec![0u8; 1024];
    loop {
        match encode_bytes(&mut buf, frame, false) {
            Ok(n) => {
                buf.truncate(n);
                return buf;
            }
            Err(e) => {
                if let redis_protocol::error::RedisProtocolErrorKind::BufferTooSmall(need) =
                    e.kind()
                {
                    let extra = *need;
                    buf.resize(buf.len() + extra + 64, 0);
                } else {
                    return Vec::new();
                }
            }
        }
    }
}
