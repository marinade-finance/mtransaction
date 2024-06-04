use lazy_static::lazy_static;
use log::{info, warn};
use prometheus::{
    register_histogram_vec, register_int_counter, register_int_gauge, register_int_gauge_vec,
    Encoder, HistogramVec, IntCounter, IntGauge, IntGaugeVec, TextEncoder,
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
    pub static ref CLIENT_QUIC_FORWARDER_PERMITS_USED_MAX: IntGaugeVec = register_int_gauge_vec!(
        "mtx_client_quic_forwarder_available_permits_max",
        "QUIC concurrent tasks created on the client side at any single moment since the last metric feed to the server",
        &["identity"]
    )
    .unwrap();
    pub static ref CLIENT_MEMORY_PHYSICAL: IntGaugeVec = register_int_gauge_vec!(
        "mtx_client_memory_physical",
        "Memory used by the client",
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
    pub static ref CHAIN_TX_FINALIZED: IntGaugeVec = register_int_gauge_vec!(
        "mtx_chain_tx_finalized",
        "How many transactions were finalized on chain",
        &["partner", "mode"]
    )
    .unwrap();
    pub static ref CHAIN_TX_FINALIZED_BY_CONSUMER: IntGaugeVec = register_int_gauge_vec!(
        "mtx_chain_tx_finalized_by_consumer",
        "How many transactions finalized submitted through consumer",
        &["consumer"]
    )
    .unwrap();
    pub static ref CHAIN_TX_FINALIZED_BY_TPU_IP: IntGaugeVec = register_int_gauge_vec!(
        "mtx_chain_tx_finalized_by_tpu_ip",
        "How many transactions finalized submitted through a specific tpu ip",
        &["tpu_ip"]
    )
    .unwrap();
    pub static ref CHAIN_TX_TIMEOUT: IntGaugeVec = register_int_gauge_vec!(
        "mtx_chain_tx_timeout",
        "How many transactions we were unable to confirm as finalized",
        &["partner", "mode"]
    )
    .unwrap();
    pub static ref CHAIN_TX_TIMEOUT_BY_CONSUMER: IntGaugeVec = register_int_gauge_vec!(
        "mtx_chain_tx_timeout_by_consumer",
        "How many transactions timed out submitted through consumer",
        &["consumer"]
    )
    .unwrap();
    pub static ref CHAIN_TX_TIMEOUT_BY_TPU_IP: IntGaugeVec = register_int_gauge_vec!(
        "mtx_chain_tx_timeout_by_tpu_ip",
        "How many transactions timed out submitted through a specific tpu ip",
        &["tpu_ip"]
    )
    .unwrap();
    pub static ref CHAIN_TX_EXECUTION_SUCCESS: IntGaugeVec = register_int_gauge_vec!(
        "mtx_chain_tx_execution_success",
        "How many transactions ended on chain without errors",
        &["partner", "mode"]
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
        &["partner", "mode"]
    )
    .unwrap();
    pub static ref SERVER_RPC_TX_BYTES_IN: IntCounter = register_int_counter!(
        "mtx_server_rpc_tx_bytes_in",
        "How many bytes were ingested by the RPC server"
    )
    .unwrap();
    pub static ref SERVER_TOTAL_CONNECTED_STAKE: IntGaugeVec = register_int_gauge_vec!(
        "mtx_server_total_connected_stake",
        "Total amount of stake connected to MTX server",
        &["identity"]
    )
    .unwrap();
    pub static ref SERVER_TOTAL_CONNECTED_TX_CONSUMERS: IntGauge = register_int_gauge!(
        "mtx_server_total_connected_tx_consumers",
        "Total amount of TX consumers to MTX server"
    )
    .unwrap();

}

pub fn reset_client_metrics(identity: &String) {
    if let Err(err) = CLIENT_PING_RTT.remove_label_values(&[identity]) {
        warn!(
            "Couldn't discard latency metrics for {}. Error: {:?}",
            identity, err
        );
    }
    if let Err(err) = SERVER_TOTAL_CONNECTED_STAKE.remove_label_values(&[identity]) {
        warn!(
            "Couldn't discard connected stake metrics for {}. Error: {:?}",
            identity, err
        );
    }
}

pub fn spawn(metrics_addr: std::net::SocketAddr) {
    tokio::spawn(async move {
        init_metrics();
        let metrics_route = warp::path!("metrics")
            .and(warp::get())
            .map(|| metrics_handler());
        info!("Spawning metrics server");
        warp::serve(metrics_route).run(metrics_addr).await;
    });
}

fn init_metrics() {
    // Set the TX consumers to 0 so that we can get some metrics when the node doesn't have any connected stake
    SERVER_TOTAL_CONNECTED_TX_CONSUMERS.set(0);
}

fn metrics_handler() -> String {
    let mut buffer = Vec::new();
    let encoder = TextEncoder::new();

    encoder.encode(&prometheus::gather(), &mut buffer).unwrap();
    String::from_utf8(buffer.clone()).unwrap()
}
