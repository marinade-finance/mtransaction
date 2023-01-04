use crate::grpc_client::pb::{self, RequestMessageEnvelope};
use lazy_static::lazy_static;
use log::error;
use prometheus::{register_int_counter, Encoder, IntCounter, TextEncoder};
use std::time::Duration;
use tokio::time::sleep;

use warp::Filter;

const METRICS_SYNC_TIME_IN_S: u64 = 10;
const METRICS_BUFFER_SIZE: usize = 1;

lazy_static! {
    pub static ref TX_RECEIVED_COUNT: IntCounter = register_int_counter!(
        "tx_received",
        "How many transactions were received by the client"
    )
    .unwrap();
    pub static ref TX_FORWARD_SUCCEEDED_COUNT: IntCounter = register_int_counter!(
        "tx_forward_succeeded",
        "How many transactions were successfully forwarded"
    )
    .unwrap();
    pub static ref TX_FORWARD_FAILED_COUNT: IntCounter = register_int_counter!(
        "tx_forward_failed",
        "How many transactions failed on the client side"
    )
    .unwrap();
}

fn spawn_feeder() -> tokio::sync::mpsc::Receiver<RequestMessageEnvelope> {
    let (metrics_sender, metrics_receiver) =
        tokio::sync::mpsc::channel::<RequestMessageEnvelope>(METRICS_BUFFER_SIZE);
    tokio::spawn(async move {
        loop {
            if let Err(err) = metrics_sender
                .send(pb::RequestMessageEnvelope {
                    metrics: Some(pb::Metrics {
                        tx_received: TX_RECEIVED_COUNT.get(),
                        tx_forward_succeeded: TX_FORWARD_SUCCEEDED_COUNT.get(),
                        tx_forward_failed: TX_FORWARD_FAILED_COUNT.get(),
                        version: crate::VERSION.to_string(),
                    }),
                    ..Default::default()
                })
                .await
            {
                error!("Failed to feed client metrics: {}", err);
            }
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

pub fn spawn_metrics(
    metrics_addr: std::net::SocketAddr,
) -> tokio::sync::mpsc::Receiver<RequestMessageEnvelope> {
    tokio::spawn(async move {
        let route_metrics = warp::path!("metrics")
            .and(warp::path::end())
            .and(warp::get())
            .map(|| collect_metrics());
        warp::serve(route_metrics).run(metrics_addr).await;
        error!("Metrics server is not up.");
        std::process::exit(1);
    });
    spawn_feeder()
}
