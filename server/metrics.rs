use lazy_static::lazy_static;
use log::info;
use prometheus::{
    register_histogram_vec, register_int_counter, register_int_gauge_vec, Encoder, HistogramVec,
    IntCounter, IntGaugeVec, TextEncoder,
};
use warp::Filter;

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
        "mtx_client_ping_rtt",
        "Latency to the client based on ping times",
        &["identity"],
        vec![0.002, 0.004, 0.008, 0.016, 0.032, 0.064, 0.128]
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

pub fn reset_client_ping_rtt(identity: &String) {
    CLIENT_PING_RTT
        .remove_label_values(&[identity])
        .expect("Couldn't remove latency metric");
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
