use lazy_static::lazy_static;
use log::info;
use prometheus::{
    register_histogram_vec, register_int_counter, register_int_gauge_vec, Encoder, HistogramVec,
    IntCounter, IntGaugeVec, TextEncoder,
};
use std::{collections::HashMap, sync::Arc};
use tokio::sync::RwLock;
use warp::{http::StatusCode, Filter, Rejection, Reply};

lazy_static! {
    pub static ref CLIENT_TX_RECEIVED: IntGaugeVec = register_int_gauge_vec!(
        "mtx_client_tx_received",
        "How many transactions were received by the client",
        &["identity"]
    )
    .unwrap();
    pub static ref CLIENT_TX_FORWARD_SUCCEEDED: IntGaugeVec = register_int_gauge_vec!(
        "mtx_client_tx_forward_succeeded",
        "How many transactions were successfully forwarded",
        &["identity"]
    )
    .unwrap();
    pub static ref CLIENT_TX_FORWARD_FAILED: IntGaugeVec = register_int_gauge_vec!(
        "mtx_client_tx_forward_failed",
        "How many transactions failed on the client side",
        &["identity"]
    )
    .unwrap();
    pub static ref CLIENT_PING_RTT: HistogramVec = register_histogram_vec!(
        "mtx_CLIENT_PING_RTT",
        "Latency to the client based on ping times",
        &["identity"],
        vec![0.005, 0.01, 0.02, 0.04, 0.08, 0.16, 0.32, 0.64, 1.28, 2.56]
    )
    .unwrap();
    pub static ref CHAIN_TX_FINALIZED: IntCounter = register_int_counter!(
        "mtx_chain_tx_finalized",
        "How many transactions were finalized on chain"
    )
    .unwrap();
    pub static ref CHAIN_TX_TIMEOUT: IntCounter = register_int_counter!(
        "mtx_chain_tx_timeout",
        "How many transactions we were unable to confirm as finalized"
    )
    .unwrap();
    pub static ref CHAIN_TX_EXECUTION_SUCCESS: IntCounter = register_int_counter!(
        "mtx_chain_tx_execution_success",
        "How many transactions ended on chain without errors"
    )
    .unwrap();
    pub static ref CHAIN_TX_EXECUTION_ERROR: IntCounter = register_int_counter!(
        "mtx_chain_tx_execution_error",
        "How many transactions ended on chain with errors"
    )
    .unwrap();
    pub static ref SERVER_RPC_TX_ACCEPTED: IntGaugeVec = register_int_gauge_vec!(
        "mtx_server_rpc_tx_accepted",
        "How many transactions were accepted by the server",
        &["partner"]
    )
    .unwrap();
    pub static ref SERVER_RPC_TX_BYTES_IN: IntCounter = register_int_counter!(
        "mtx_server_rpc_tx_bytes_in",
        "How many bytes were ingested by the RPC server"
    )
    .unwrap();
}

pub struct MetricsStore {
    tx_slots: RwLock<HashMap<u64, usize>>,
}

#[derive(Debug, Clone)]
pub enum Metric {
    ClientTxReceived { identity: String, count: u64 },
    ClientTxForwardSucceeded { identity: String, count: u64 },
    ClientTxForwardFailed { identity: String, count: u64 },
    ClientLatency { identity: String, latency: f64 },
    ChainTxFinalized,
    ChainTxTimeout,
    ChainTxSlot { slot: u64 },
    ServerRpcTxAccepted,
    ServerRpcTxBytesIn { bytes: u64 },
}

pub fn spawn(metrics_addr: std::net::SocketAddr) {
    tokio::spawn(async move {
        let metrics_route = warp::path!("metrics")
            .and(warp::get())
            .map(|| metrics_handler());
        info!("Spawning metrics server");
        warp::serve(metrics_route).run(metrics_addr).await;
    });
}

fn metrics_handler() -> String {
    let mut buffer = Vec::new();
    let encoder = TextEncoder::new();

    encoder.encode(&prometheus::gather(), &mut buffer).unwrap();
    String::from_utf8(buffer.clone()).unwrap()
}

pub async fn tx_slots_handler(
    metrics_store: Arc<MetricsStore>,
) -> std::result::Result<impl Reply, Rejection> {
    info!("Slots info requested");

    let tx_slots = metrics_store
        .tx_slots
        .read()
        .await
        .iter()
        .map(|(slot, count)| std::iter::repeat(slot.to_string()).take(*count))
        .flatten()
        .fold(String::new(), |a, b| a + &b + "\n");

    Ok(warp::reply::with_status(tx_slots, StatusCode::OK))
}
