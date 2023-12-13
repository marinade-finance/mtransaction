use crate::grpc_client::pb::{self, RequestMessageEnvelope};
use lazy_static::lazy_static;
use log::error;
use memory_stats::memory_stats;
use prometheus::{
    register_int_counter_vec, register_int_gauge, Encoder, IntCounterVec, IntGauge, TextEncoder,
};
use std::time::Duration;
use tokio::time::sleep;

use warp::Filter;

const METRICS_SYNC_TIME_IN_S: u64 = 10;
const METRICS_BUFFER_SIZE: usize = 1;

lazy_static! {
    pub static ref TX_RECEIVED_COUNT: IntCounterVec = register_int_counter_vec!(
        "tx_received",
        "How many transactions were received by the client",
        &["source"],
    )
    .unwrap();
    pub static ref TX_FORWARD_SUCCEEDED_COUNT: IntCounterVec = register_int_counter_vec!(
        "tx_forward_succeeded",
        "How many transactions were successfully forwarded",
        &["source"],
    )
    .unwrap();
    pub static ref TX_FORWARD_FAILED_COUNT: IntCounterVec = register_int_counter_vec!(
        "tx_forward_failed",
        "How many transactions failed on the client side",
        &["source"],
    )
    .unwrap();
    pub static ref QUIC_FORWARDER_PERMITS_USED_MAX: IntGauge = register_int_gauge!(
        "quic_forwarder_available_permits_max",
        "QUIC concurrent tasks created at any single moment since the last metric feed to the server"
    )
    .unwrap();
}

pub fn observe_quic_forwarded_available_permits(permits_used: usize) {
    QUIC_FORWARDER_PERMITS_USED_MAX.set(
        QUIC_FORWARDER_PERMITS_USED_MAX
            .get()
            .max(permits_used as i64),
    )
}

pub fn spawn_feeder(source: String) -> tokio::sync::mpsc::Receiver<RequestMessageEnvelope> {
    let (metrics_sender, metrics_receiver) =
        tokio::sync::mpsc::channel::<RequestMessageEnvelope>(METRICS_BUFFER_SIZE);
    tokio::spawn(async move {
        loop {
            let memory_physical = memory_stats().map_or(0, |usage| usage.physical_mem as u64);
            if let Err(err) = metrics_sender
                .send(pb::RequestMessageEnvelope {
                    metrics: Some(pb::Metrics {
                        tx_received: TX_RECEIVED_COUNT
                            .with_label_values(&[source.as_str()])
                            .get(),
                        tx_forward_succeeded: TX_FORWARD_SUCCEEDED_COUNT
                            .with_label_values(&[source.as_str()])
                            .get(),
                        tx_forward_failed: TX_FORWARD_FAILED_COUNT
                            .with_label_values(&[source.as_str()])
                            .get(),
                        version: crate::VERSION.to_string(),
                        quic_forwarder_permits_used_max: QUIC_FORWARDER_PERMITS_USED_MAX.get()
                            as u64,
                        memory_physical,
                    }),
                    ..Default::default()
                })
                .await
            {
                error!("Failed to feed client metrics: {}", err);
            }
            QUIC_FORWARDER_PERMITS_USED_MAX.set(0);
            sleep(Duration::from_secs(METRICS_SYNC_TIME_IN_S)).await;
        }
    });
    metrics_receiver
}

fn collect_metrics() -> String {
    let mut buffer = Vec::new();
    let encoder = TextEncoder::new();

    encoder.encode(&prometheus::gather(), &mut buffer).unwrap();
    String::from_utf8(buffer.clone()).unwrap()
}

pub fn spawn_metrics(metrics_addr: std::net::SocketAddr) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let route_metrics = warp::path!("metrics")
            .and(warp::path::end())
            .and(warp::get())
            .map(|| collect_metrics());
        warp::serve(route_metrics).run(metrics_addr).await;
        error!("Metrics server is not up.");
        std::process::exit(1);
    })
}
