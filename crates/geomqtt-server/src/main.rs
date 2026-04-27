mod broker;
mod config;
mod coord;
mod fanout;
mod http;
mod metrics;
mod mqtt;
mod payload;
mod redis;
mod resp;

use anyhow::Result;
use std::sync::Arc;
use tracing::info;

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info,geomqtt_server=debug".into()),
        )
        .init();

    let cfg = Arc::new(config::Config::from_env()?);
    info!(?cfg, "starting geomqtt-server");

    let redis = redis::connect(&cfg).await?;
    let broker = broker::Broker::new();
    let metrics = metrics::Metrics::new();

    let mqtt_ctx = mqtt::MqttContext {
        broker: broker.clone(),
        redis: redis.clone(),
        cfg: cfg.clone(),
        metrics: metrics.clone(),
    };
    let resp_ctx = resp::RespContext {
        broker: broker.clone(),
        redis: redis.clone(),
        cfg: cfg.clone(),
        metrics: metrics.clone(),
    };
    let http_state = http::HttpState {
        ctx: mqtt_ctx.clone(),
    };

    let resp_task = tokio::spawn(resp::serve(cfg.resp_addr, resp_ctx));
    let mqtt_task = tokio::spawn(mqtt::serve(cfg.mqtt_addr, cfg.mqtt_ws_addr, mqtt_ctx));
    let http_task = tokio::spawn(http::serve(cfg.http_addr, http_state));
    let bridge_task = tokio::spawn(redis::run_pubsub_bridge(
        redis.clone(),
        broker.clone(),
        metrics.clone(),
    ));

    tokio::select! {
        r = resp_task   => { r??; }
        r = mqtt_task   => { r??; }
        r = http_task   => { r??; }
        r = bridge_task => { r??; }
        _ = tokio::signal::ctrl_c() => {
            info!("shutdown requested");
        }
    }
    Ok(())
}
