use log::info;
use prometheus::{
    register_histogram_vec, register_int_counter, register_int_gauge_vec, Encoder, HistogramVec,
    IntCounter, IntGaugeVec, TextEncoder,
};
use std::sync::Arc;
use tokio::sync::{
    mpsc::{unbounded_channel, UnboundedSender},
    RwLock,
};
use warp::{http::StatusCode, Filter, Rejection, Reply};

pub struct MetricsStore {
    client_tx_received: IntGaugeVec,
    client_tx_succeeded: IntGaugeVec,
    client_tx_failed: IntGaugeVec,
    client_latency: HistogramVec,
    server_rpc_tx_accepted: IntCounter,
    server_rpc_tx_bytes_in: IntCounter,
}

impl MetricsStore {
    pub fn new() -> Self {
        Self {
            client_tx_received: register_int_gauge_vec!(
                "mtx_client_tx_received",
                "How many transactions were received by the client",
                &["identity"]
            )
            .unwrap(),
            client_tx_succeeded: register_int_gauge_vec!(
                "mtx_client_tx_succeeded",
                "How many transactions were successfully forwarded",
                &["identity"]
            )
            .unwrap(),
            client_tx_failed: register_int_gauge_vec!(
                "mtx_client_tx_failed",
                "How many transactions failed on the client side",
                &["identity"]
            )
            .unwrap(),
            client_latency: register_histogram_vec!(
                "mtx_client_latency",
                "Latency to the client based on ping times",
                &["identity"],
                vec![0.005, 0.01, 0.02, 0.04, 0.08, 0.16, 0.32, 0.64, 1.28, 2.56]
            )
            .unwrap(),
            server_rpc_tx_accepted: register_int_counter!(
                "mtx_server_rpc_tx_accepted",
                "How many transactions were accepted by the server"
            )
            .unwrap(),
            server_rpc_tx_bytes_in: register_int_counter!(
                "mtx_server_rpc_tx_bytes_in",
                "How many bytes were ingested by the RPC server"
            )
            .unwrap(),
        }
    }

    pub fn process_metric(&mut self, metric: Metric) {
        match metric {
            Metric::ClientTxReceived { identity, count } => self
                .client_tx_received
                .with_label_values(&[&identity])
                .set(count as i64),
            Metric::ClientTxSucceeded { identity, count } => self
                .client_tx_succeeded
                .with_label_values(&[&identity])
                .set(count as i64),
            Metric::ClientTxFailed { identity, count } => self
                .client_tx_failed
                .with_label_values(&[&identity])
                .set(count as i64),
            Metric::ClientLatency { identity, latency } => self
                .client_latency
                .with_label_values(&[&identity])
                .observe(latency),
            Metric::ServerRpcTxAccepted => self.server_rpc_tx_accepted.inc(),
            Metric::ServerRpcTxBytesIn { bytes } => self.server_rpc_tx_bytes_in.inc_by(bytes),
        }
    }

    pub fn process_metrics(&mut self, metrics: Vec<Metric>) {
        for metric in metrics {
            self.process_metric(metric);
        }
    }

    pub fn gather(&self) -> String {
        let mut buffer = Vec::new();
        let encoder = TextEncoder::new();
        let metrics = prometheus::gather();
        encoder.encode(&metrics, &mut buffer).unwrap();

        String::from_utf8(buffer.clone()).unwrap()
    }
}

#[derive(Debug)]
pub enum Metric {
    ClientTxReceived { identity: String, count: u64 },
    ClientTxSucceeded { identity: String, count: u64 },
    ClientTxFailed { identity: String, count: u64 },
    ClientLatency { identity: String, latency: f64 },
    ServerRpcTxAccepted,
    ServerRpcTxBytesIn { bytes: u64 },
}

pub fn spawn(metrics_addr: std::net::SocketAddr) -> UnboundedSender<Vec<Metric>> {
    let (tx, mut rx) = unbounded_channel();
    let metrics_store = Arc::new(RwLock::new(MetricsStore::new()));

    {
        let metrics_store = metrics_store.clone();
        tokio::spawn(async move {
            info!("Waiting for metrics");
            while let Some(metrics) = rx.recv().await {
                metrics_store.write().await.process_metrics(metrics);
            }
        });
    }

    {
        tokio::spawn(async move {
            let metrics_route = warp::path!("metrics")
                .and(warp::get())
                .and(warp::any().map(move || metrics_store.clone()))
                .and_then(metrics_handler);
            info!("Spawning metrics server");
            warp::serve(metrics_route).run(metrics_addr).await;
        });
    }

    tx
}

pub async fn metrics_handler(
    metrics_store: Arc<RwLock<MetricsStore>>,
) -> std::result::Result<impl Reply, Rejection> {
    info!("Metrics requested");
    Ok(warp::reply::with_status(
        metrics_store.read().await.gather(),
        StatusCode::OK,
    ))
}
