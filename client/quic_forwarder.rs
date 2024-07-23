use crate::forwarder::{ForwardedTransaction, Forwarder};
use crate::grpc_client::pb::Transaction;
use crate::{metrics, perf_counter_ns, time_ms};
use base64::decode;
use log::{error, info};
use solana_client::{
    connection_cache::ConnectionCache, nonblocking::tpu_connection::TpuConnection,
};
use solana_sdk::{signature::Keypair, transport::TransportError};
use std::{net::IpAddr, sync::Arc};
use tokio::sync::Semaphore;

pub struct QuicForwarder {
    max_permits: usize,
    throttle_parallel: Arc<Semaphore>,
    connection_cache: Arc<ConnectionCache>,
}

impl QuicForwarder {
    pub fn new(identity: Option<Keypair>, tpu_addr: Option<IpAddr>, max_permits: usize) -> Self {
        let cert_info = if let (Some(identity), Some(tpu_addr)) = (&identity, tpu_addr) {
            Some((identity, tpu_addr))
        } else {
            None
        };
        let connection_cache = ConnectionCache::new_with_client_options(
            "default connection cache",
            2,
            None,
            cert_info,
            None,
        );

        Self {
            max_permits,
            throttle_parallel: Arc::new(Semaphore::new(max_permits)),
            connection_cache: Arc::new(connection_cache),
        }
    }

    fn spawn_transaction_forwarder(&self, source: String, transaction: Transaction, tpu: &String) {
        let tpu = tpu.clone();
        let throttle_parallel = self.throttle_parallel.clone();
        let connection_cache = self.connection_cache.clone();
        let max_permits = self.max_permits;

        tokio::spawn(async move {
            metrics::observe_quic_forwarded_available_permits(
                max_permits - throttle_parallel.available_permits(),
            );

            let tpu = tpu.parse().unwrap();
            let wire_transaction = decode(transaction.data).unwrap();

            let throttle_permit = throttle_parallel.acquire_owned().await.unwrap();

            info!("Tx {} -> {}", transaction.signature, &tpu);
            let conn = connection_cache.get_nonblocking_connection(&tpu);
            let start = perf_counter_ns();
            let request_result = conn.send_data(&wire_transaction).await;
            let took = start - perf_counter_ns();
            metrics::TX_FORWARD_LATENCY
                .with_label_values(&[&tpu.ip().to_string()])
                .add(took.try_into().unwrap_or(0));
            Self::handle_send_result(source, request_result);

            drop(throttle_permit);
        });
    }

    fn handle_send_result(source: String, result: Result<(), TransportError>) {
        if let Err(err) = result {
            error!("Failed to send the transaction: {}", err);
            metrics::TX_FORWARD_FAILED_COUNT
                .with_label_values(&[source.as_str()])
                .inc();
        } else {
            metrics::TX_FORWARD_SUCCEEDED_COUNT
                .with_label_values(&[source.as_str()])
                .inc();
        }
    }
}

impl Forwarder for QuicForwarder {
    fn process(&self, tx: ForwardedTransaction) {
        let source = tx.source;
        let transaction = tx.transaction;
        metrics::TX_RECEIVED_COUNT
            .with_label_values(&[source.as_str()])
            .inc();
        metrics::TX_RECEIVED_LATENCY
            .with_label_values(&[source.as_str()])
            .add((time_ms().max(tx.ctime) - tx.ctime).try_into().unwrap_or(0));
        for tpu in transaction.tpu.iter() {
            self.spawn_transaction_forwarder(source.clone(), transaction.clone(), tpu);
        }
    }
}
