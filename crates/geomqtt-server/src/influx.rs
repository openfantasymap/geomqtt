//! Optional InfluxDB 2.x sink. When `GEOMQTT_INFLUX_URL` is set, the RESP
//! proxy hands every GEOADD position and HSET attribute write to a bounded
//! mpsc; a background task batches them into line-protocol POSTs against
//! `/api/v2/write`. The hot path is `try_send` only — full queue or HTTP
//! failure never blocks fanout, it just bumps a counter.

use crate::config::InfluxSettings;
use crate::metrics::Metrics;
use serde_json::{Map, Value};
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc;
use tracing::{info, warn};

const QUEUE_CAPACITY: usize = 4096;
const MAX_BATCH: usize = 500;
const FLUSH_INTERVAL_MS: u64 = 200;
const HTTP_TIMEOUT_SECS: u64 = 5;

pub struct InfluxClient {
    tx: mpsc::Sender<String>,
    metrics: Arc<Metrics>,
}

impl InfluxClient {
    pub fn spawn(settings: InfluxSettings, metrics: Arc<Metrics>) -> Arc<Self> {
        let (tx, rx) = mpsc::channel::<String>(QUEUE_CAPACITY);
        let m = metrics.clone();
        tokio::spawn(async move { writer_task(settings, rx, m).await });
        Arc::new(Self { tx, metrics })
    }

    /// Position point — `geomqtt_position,set=<set>,obid=<member> lat=<lat>,lon=<lon>`.
    pub fn position(&self, set: &str, member: &str, lon: f64, lat: f64) {
        let mut line = String::with_capacity(96);
        line.push_str("geomqtt_position,set=");
        escape_tag(&mut line, set);
        line.push_str(",obid=");
        escape_tag(&mut line, member);
        line.push_str(" lat=");
        push_float(&mut line, lat);
        line.push_str(",lon=");
        push_float(&mut line, lon);
        self.send(line);
    }

    /// Attribute write — `geomqtt_attr,obid=<obid> <k>="<v>"[,<k>="<v>"…]`.
    /// Skips entries whose value can't be reduced to a string. No-op if the
    /// resulting fieldset is empty (line protocol requires ≥1 field).
    pub fn attr(&self, obid: &str, attrs: &Map<String, Value>) {
        if attrs.is_empty() {
            return;
        }
        let mut line = String::with_capacity(64 + attrs.len() * 24);
        line.push_str("geomqtt_attr,obid=");
        escape_tag(&mut line, obid);
        line.push(' ');
        let mut first = true;
        for (k, v) in attrs {
            let s = match v {
                Value::String(s) => s.clone(),
                Value::Number(n) => n.to_string(),
                Value::Bool(b) => b.to_string(),
                Value::Null => continue,
                other => other.to_string(),
            };
            if !first {
                line.push(',');
            }
            first = false;
            escape_field_key(&mut line, k);
            line.push('=');
            push_string_field(&mut line, &s);
        }
        if first {
            return; // no fields produced
        }
        self.send(line);
    }

    fn send(&self, line: String) {
        match self.tx.try_send(line) {
            Ok(()) => {
                self.metrics
                    .influx_writes_enqueued
                    .fetch_add(1, Ordering::Relaxed);
            }
            Err(_) => {
                self.metrics
                    .influx_writes_dropped
                    .fetch_add(1, Ordering::Relaxed);
            }
        }
    }
}

async fn writer_task(
    settings: InfluxSettings,
    mut rx: mpsc::Receiver<String>,
    metrics: Arc<Metrics>,
) {
    let client = match reqwest::Client::builder()
        .timeout(Duration::from_secs(HTTP_TIMEOUT_SECS))
        .build()
    {
        Ok(c) => c,
        Err(e) => {
            warn!(error = %e, "influx: failed to build reqwest client; sink disabled");
            // Drain forever so senders' try_send doesn't accumulate.
            while rx.recv().await.is_some() {}
            return;
        }
    };
    let url = format!(
        "{}/api/v2/write?org={}&bucket={}&precision=ns",
        settings.url,
        urlencode(&settings.org),
        urlencode(&settings.bucket),
    );
    let auth = format!("Token {}", settings.token);
    info!(url = %settings.url, bucket = %settings.bucket, "influx sink enabled");

    let mut batch: Vec<String> = Vec::with_capacity(MAX_BATCH);
    while let Some(first) = rx.recv().await {
        batch.push(first);
        let deadline = tokio::time::sleep(Duration::from_millis(FLUSH_INTERVAL_MS));
        tokio::pin!(deadline);
        loop {
            tokio::select! {
                maybe = rx.recv() => match maybe {
                    Some(l) => {
                        batch.push(l);
                        if batch.len() >= MAX_BATCH { break; }
                    }
                    None => break,
                },
                _ = &mut deadline => break,
            }
        }
        let body = batch.join("\n");
        let res = client
            .post(&url)
            .header("Authorization", &auth)
            .header("Content-Type", "text/plain; charset=utf-8")
            .body(body)
            .send()
            .await;
        match res {
            Ok(r) if r.status().is_success() => {
                metrics.influx_batches_sent.fetch_add(1, Ordering::Relaxed);
            }
            Ok(r) => {
                metrics.influx_batch_errors.fetch_add(1, Ordering::Relaxed);
                warn!(status = %r.status(), "influx write rejected");
            }
            Err(e) => {
                metrics.influx_batch_errors.fetch_add(1, Ordering::Relaxed);
                warn!(error = %e, "influx write failed");
            }
        }
        batch.clear();
    }
}

fn escape_tag(out: &mut String, s: &str) {
    for c in s.chars() {
        match c {
            ',' | '=' | ' ' => {
                out.push('\\');
                out.push(c);
            }
            _ => out.push(c),
        }
    }
}

fn escape_field_key(out: &mut String, s: &str) {
    escape_tag(out, s); // same escape rules
}

fn push_string_field(out: &mut String, s: &str) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' | '\\' => {
                out.push('\\');
                out.push(c);
            }
            _ => out.push(c),
        }
    }
    out.push('"');
}

fn push_float(out: &mut String, v: f64) {
    if v.is_finite() {
        use std::fmt::Write as _;
        let _ = write!(out, "{v}");
    } else {
        out.push('0');
    }
}

fn urlencode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char);
            }
            _ => {
                use std::fmt::Write as _;
                let _ = write!(out, "%{:02X}", b);
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tag_escape() {
        let mut s = String::new();
        escape_tag(&mut s, "a b,c=d");
        assert_eq!(s, r"a\ b\,c\=d");
    }

    #[test]
    fn field_string_escape() {
        let mut s = String::new();
        push_string_field(&mut s, r#"he said "hi" \o/"#);
        assert_eq!(s, r#""he said \"hi\" \\o/""#);
    }

    #[test]
    fn urlencodes_special() {
        assert_eq!(urlencode("my org"), "my%20org");
        assert_eq!(urlencode("ascii.-_~"), "ascii.-_~");
    }
}
