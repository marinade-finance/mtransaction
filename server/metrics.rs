use log::info;
use prometheus::{
    register_histogram_vec, register_int_counter, register_int_gauge_vec, Encoder, HistogramVec,
    IntCounter, IntGaugeVec, TextEncoder,
};
use std::{collections::HashMap, sync::Arc};
use tokio::sync::{
    mpsc::{unbounded_channel, UnboundedSender},
    RwLock,
};
use warp::{http::StatusCode, Filter, Rejection, Reply};

pub const NOT_AVAILABLE: f64 = -1.0;
pub struct MetricsStore {
    client_tx_received: IntGaugeVec,
    client_tx_forward_succeeded: IntGaugeVec,
    client_tx_forward_failed: IntGaugeVec,
    client_latency: HistogramVec,
    chain_tx_finalized: IntCounter,
    chain_tx_timeout: IntCounter,
    server_rpc_tx_accepted: IntCounter,
    server_rpc_tx_bytes_in: IntCounter,
    server_total_connected_stake: IntGaugeVec,
    tx_slots: RwLock<HashMap<u64, usize>>,
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
            client_tx_forward_succeeded: register_int_gauge_vec!(
                "mtx_client_tx_forward_succeeded",
                "How many transactions were successfully forwarded",
                &["identity"]
            )
            .unwrap(),
            client_tx_forward_failed: register_int_gauge_vec!(
                "mtx_client_tx_forward_failed",
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
            chain_tx_finalized: register_int_counter!(
                "mtx_chain_tx_finalized",
                "How many transactions were finalized on chain"
            )
            .unwrap(),
            chain_tx_timeout: register_int_counter!(
                "mtx_chain_tx_timeout",
                "How many transactions we were unable to confirm as finalized"
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
            server_total_connected_stake: register_int_gauge_vec!(
                "mtx_server_total_connected_stake",
                "Total amount of stake connected to MTX server",
                &["identity"]
            )
            .unwrap(),
            tx_slots: Default::default(),
        }
    }

    pub async fn process_metric(&self, metric: Metric) {
        match metric {
            Metric::ClientTxReceived { identity, count } => self
                .client_tx_received
                .with_label_values(&[&identity])
                .set(count as i64),
            Metric::ClientTxForwardSucceeded { identity, count } => self
                .client_tx_forward_succeeded
                .with_label_values(&[&identity])
                .set(count as i64),
            Metric::ClientTxForwardFailed { identity, count } => self
                .client_tx_forward_failed
                .with_label_values(&[&identity])
                .set(count as i64),
            Metric::ClientLatency { identity, latency } => {
                if latency == NOT_AVAILABLE {
                    self.client_latency
                        .remove_label_values(&[&identity])
                        .expect("Couldn't remove latency metric");
                } else {
                    self.client_latency
                        .with_label_values(&[&identity])
                        .observe(latency);
                }
            }
            Metric::ChainTxFinalized => self.chain_tx_finalized.inc(),
            Metric::ChainTxTimeout => self.chain_tx_timeout.inc(),
            Metric::ChainTxSlot { slot } => self.tx_slot_inc(slot).await,
            Metric::ServerRpcTxAccepted => self.server_rpc_tx_accepted.inc(),
            Metric::ServerRpcTxBytesIn { bytes } => self.server_rpc_tx_bytes_in.inc_by(bytes),
            Metric::ServerTotalConnectedStake { identity, stake } => {
                if stake == NOT_AVAILABLE {
                    self.server_total_connected_stake
                        .remove_label_values(&[&identity])
                        .expect("Couldn't remove stake metric");
                } else {
                    self.server_total_connected_stake
                        .with_label_values(&[&identity])
                        .set(stake as i64);
                }
            }
        }
    }

    async fn tx_slot_inc(&self, slot: u64) {
        self.tx_slots
            .write()
            .await
            .entry(slot)
            .and_modify(|count| *count += 1)
            .or_insert(1);
    }

    pub async fn process_metrics(&self, metrics: Vec<Metric>) {
        for metric in metrics {
            self.process_metric(metric).await;
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
    ServerTotalConnectedStake { identity: String, stake: f64 },
}

pub fn spawn(metrics_addr: std::net::SocketAddr) -> UnboundedSender<Vec<Metric>> {
    let (tx, mut rx) = unbounded_channel();
    let metrics_store = Arc::new(MetricsStore::new());

    {
        let metrics_store = metrics_store.clone();
        tokio::spawn(async move {
            info!("Waiting for metrics");
            while let Some(metrics) = rx.recv().await {
                metrics_store.process_metrics(metrics).await;
            }
        });
    }

    {
        tokio::spawn(async move {
            let metrics_route = {
                let metrics_store = metrics_store.clone();
                warp::path!("metrics")
                    .and(warp::get())
                    .and(warp::any().map(move || metrics_store.clone()))
                    .and_then(metrics_handler)
            };
            let tx_slots_route = {
                let metrics_store = metrics_store.clone();
                warp::path!("slots")
                    .and(warp::get())
                    .and(warp::any().map(move || metrics_store.clone()))
                    .and_then(tx_slots_handler)
            };
            info!("Spawning metrics server");
            warp::serve(metrics_route.or(tx_slots_route))
                .run(metrics_addr)
                .await;
        });
    }

    tx
}

pub async fn metrics_handler(
    metrics_store: Arc<MetricsStore>,
) -> std::result::Result<impl Reply, Rejection> {
    info!("Metrics requested");
    Ok(warp::reply::with_status(
        metrics_store.gather(),
        StatusCode::OK,
    ))
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
