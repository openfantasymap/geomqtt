//! End-to-end integration test.
//!
//! Spawns the `geomqtt-server` binary pointed at a real Redis (requires
//! `GEOMQTT_TEST_REDIS_URL` env var, e.g. `redis://127.0.0.1:6379`). If the
//! env var is missing, the test logs a skip and returns — local `cargo test`
//! without Redis still passes. CI sets it via a Redis service container.
//!
//! Covers the three user-facing surfaces:
//!   * HTTP: /healthz + /config sanity
//!   * RESP: GEOADD forwarded to upstream, /tiles returns the FeatureCollection
//!   * MQTT: subscribe to a tile, receive the per-subscriber snapshot burst

use std::io::{Read, Write};
use std::net::TcpStream;
use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

// Tests run in parallel (cargo test uses a thread pool + tokio tests run
// concurrently), so each test reserves its own port block rather than sharing
// fixed ports.
#[derive(Clone, Copy)]
struct Ports {
    resp: u16,
    mqtt: u16,
    ws: u16,
    http: u16,
}

struct ServerGuard(Child);

impl Drop for ServerGuard {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn maybe_redis() -> Option<String> {
    std::env::var("GEOMQTT_TEST_REDIS_URL").ok()
}

fn spawn_server(redis_url: &str, ports: Ports) -> ServerGuard {
    let bin = env!("CARGO_BIN_EXE_geomqtt-server");
    let child = Command::new(bin)
        .env("GEOMQTT_REDIS_URL", redis_url)
        .env("GEOMQTT_RESP_ADDR", format!("127.0.0.1:{}", ports.resp))
        .env("GEOMQTT_MQTT_ADDR", format!("127.0.0.1:{}", ports.mqtt))
        .env("GEOMQTT_MQTT_WS_ADDR", format!("127.0.0.1:{}", ports.ws))
        .env("GEOMQTT_HTTP_ADDR", format!("127.0.0.1:{}", ports.http))
        .env("GEOMQTT_ENRICH_ATTRS", "icon,color")
        .env("GEOMQTT_ENRICH_ZOOMS", "8-10")
        .env("RUST_LOG", "error")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn geomqtt-server binary");
    ServerGuard(child)
}

fn wait_for_http(port: u16, timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if ureq::get(&format!("http://127.0.0.1:{port}/healthz"))
            .timeout(Duration::from_millis(500))
            .call()
            .is_ok()
        {
            return true;
        }
        thread::sleep(Duration::from_millis(100));
    }
    false
}

fn resp_command(stream: &mut TcpStream, args: &[&str]) -> Vec<u8> {
    let mut cmd = format!("*{}\r\n", args.len());
    for a in args {
        cmd.push_str(&format!("${}\r\n{a}\r\n", a.len()));
    }
    stream.write_all(cmd.as_bytes()).unwrap();
    let mut buf = vec![0u8; 4096];
    let n = stream.read(&mut buf).unwrap();
    buf.truncate(n);
    buf
}

#[test]
fn http_and_resp_end_to_end() {
    let Some(redis_url) = maybe_redis() else {
        eprintln!("skip http_and_resp_end_to_end: GEOMQTT_TEST_REDIS_URL not set");
        return;
    };
    let ports = Ports {
        resp: 26380,
        mqtt: 21883,
        ws: 28083,
        http: 28080,
    };
    let _guard = spawn_server(&redis_url, ports);
    assert!(
        wait_for_http(ports.http, Duration::from_secs(10)),
        "server never bound HTTP"
    );

    // /healthz
    let body: String = ureq::get(&format!("http://127.0.0.1:{}/healthz", ports.http))
        .call()
        .unwrap()
        .into_string()
        .unwrap();
    assert_eq!(body.trim(), "ok");

    // /config
    let cfg: serde_json::Value = ureq::get(&format!("http://127.0.0.1:{}/config", ports.http))
        .call()
        .unwrap()
        .into_json()
        .unwrap();
    assert_eq!(cfg["tileSize"], 256);
    assert_eq!(cfg["zooms"], serde_json::json!([8, 9, 10]));

    // RESP: a proxied GEOADD must succeed and the written point must be
    // visible via /tiles.
    let mut s = TcpStream::connect(format!("127.0.0.1:{}", ports.resp)).unwrap();
    let set = format!("test-{}", std::process::id());
    let member = format!("veh-{}", std::process::id());

    let resp = resp_command(&mut s, &["GEOADD", &set, "11.34", "44.49", &member]);
    let text = String::from_utf8_lossy(&resp);
    assert!(
        text.starts_with(":1") || text.starts_with(":0"),
        "GEOADD returned unexpected: {text:?}"
    );

    let resp = resp_command(
        &mut s,
        &[
            "HSET",
            &format!("obj:{member}"),
            "icon",
            "truck",
            "color",
            "red",
        ],
    );
    assert!(String::from_utf8_lossy(&resp).starts_with(':'));

    // Tile at z=10 containing (11.34, 44.49) = (544, 370)
    let geo: serde_json::Value = ureq::get(&format!(
        "http://127.0.0.1:{}/tiles/{set}/10/544/370",
        ports.http
    ))
    .call()
    .unwrap()
    .into_json()
    .unwrap();
    let features = geo["features"].as_array().expect("FeatureCollection");
    assert!(
        features.iter().any(|f| f["id"] == member.as_str()),
        "point not in /tiles response: {geo}"
    );

    // Cleanup — remove the test set + hash.
    let _ = resp_command(&mut s, &["DEL", &set, &format!("obj:{member}")]);
}

#[tokio::test]
async fn mqtt_snapshot_burst() {
    let Some(redis_url) = maybe_redis() else {
        eprintln!("skip mqtt_snapshot_burst: GEOMQTT_TEST_REDIS_URL not set");
        return;
    };
    // Distinct port block so this test doesn't collide with http_and_resp_end_to_end.
    let ports = Ports {
        resp: 26381,
        mqtt: 21884,
        ws: 28084,
        http: 28081,
    };
    let _guard = spawn_server(&redis_url, ports);
    assert!(
        wait_for_http(ports.http, Duration::from_secs(10)),
        "server never bound HTTP"
    );

    // Seed a point via RESP before subscribing so the snapshot has something
    // to replay.
    let set = format!("smqt-{}", std::process::id());
    let member = format!("veh-{}", std::process::id());
    {
        let mut s = TcpStream::connect(format!("127.0.0.1:{}", ports.resp)).unwrap();
        resp_command(&mut s, &["GEOADD", &set, "11.34", "44.49", &member]);
        resp_command(&mut s, &["HSET", &format!("obj:{member}"), "icon", "truck"]);
    }

    // Subscribe to the containing tile (z=10 → 544/370).
    let mut opts = rumqttc::MqttOptions::new("snapshot-test", "127.0.0.1", ports.mqtt);
    opts.set_keep_alive(Duration::from_secs(5));
    let (client, mut eventloop) = rumqttc::AsyncClient::new(opts, 16);
    let topic = format!("geo/{set}/10/544/370");
    client
        .subscribe(&topic, rumqttc::QoS::AtMostOnce)
        .await
        .unwrap();

    let deadline = Instant::now() + Duration::from_secs(10);
    let mut got_snapshot = false;
    while Instant::now() < deadline {
        let poll = tokio::time::timeout(Duration::from_millis(500), eventloop.poll()).await;
        match poll {
            Ok(Ok(rumqttc::Event::Incoming(rumqttc::Packet::Publish(p)))) => {
                if let Ok(v) = serde_json::from_slice::<serde_json::Value>(&p.payload) {
                    if v["op"] == "snapshot" && v["id"] == member.as_str() {
                        assert_eq!(v["attrs"]["icon"], "truck");
                        got_snapshot = true;
                        break;
                    }
                }
            }
            Ok(_) => continue,
            Err(_) => continue,
        }
    }
    assert!(got_snapshot, "no snapshot received on {topic}");

    // Cleanup.
    let mut s = TcpStream::connect(format!("127.0.0.1:{}", ports.resp)).unwrap();
    resp_command(&mut s, &["DEL", &set, &format!("obj:{member}")]);
}
