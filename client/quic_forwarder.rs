use crate::forwarder::Forwarder;
use crate::grpc_client::pb::Transaction;
use crate::metrics;
use base64::decode;
use log::{error, info};
use solana_client::{
    connection_cache::{ConnectionCache, DEFAULT_TPU_CONNECTION_POOL_SIZE},
    nonblocking::tpu_connection::TpuConnection,
};
use solana_sdk::{signature::Keypair, transport::TransportError};
use std::{net::IpAddr, sync::Arc};
use tokio::{
    sync::Semaphore,
    time::{timeout, Duration},
};

const SEND_TRANSACTION_TIMEOUT_MS: u64 = 10000;

pub struct QuicForwarder {
    throttle_parallel: Arc<Semaphore>,
    connection_cache: Arc<ConnectionCache>,
}

impl QuicForwarder {
    pub fn new(
        identity: Option<Keypair>,
        tpu_addr: Option<IpAddr>,
        throttle_parallel: usize,
    ) -> Self {
        let mut connection_cache = ConnectionCache::new(DEFAULT_TPU_CONNECTION_POOL_SIZE);
        if let (Some(identity), Some(tpu_addr)) = (identity, tpu_addr) {
            if let Err(err) = connection_cache.update_client_certificate(&identity, tpu_addr) {
                error!("Failed to update client certificate: {}", err);
            }
            info!("Updated QUIC certificate");
        }

        Self {
            throttle_parallel: Arc::new(Semaphore::new(throttle_parallel)),
            connection_cache: Arc::new(connection_cache),
        }
    }

    fn spawn_transaction_forwarder(&self, transaction: Transaction, tpu: &String) {
        let tpu = tpu.clone();
        let throttle_parallel = self.throttle_parallel.clone();
        let connection_cache = self.connection_cache.clone();

        tokio::spawn(async move {
            let tpu = tpu.parse().unwrap();
            let wire_transaction = decode(transaction.data).unwrap();

            let throttle_permit = throttle_parallel.acquire_owned().await.unwrap();

            info!("Tx {} -> {}", transaction.signature, &tpu);
            let conn = connection_cache.get_nonblocking_connection(&tpu);

            let result = timeout(
                Duration::from_millis(SEND_TRANSACTION_TIMEOUT_MS),
                conn.send_wire_transaction(&wire_transaction),
            )
            .await;

            Self::handle_send_result(result);
            drop(throttle_permit);
        });
    }

    fn handle_send_result(result: Result<Result<(), TransportError>, tokio::time::error::Elapsed>) {
        match result {
            Ok(result) => {
                if let Err(err) = result {
                    error!("Failed to send the transaction: {}", err);
                    metrics::TX_FORWARD_FAILED_COUNT.inc();
                } else {
                    metrics::TX_FORWARD_SUCCEEDED_COUNT.inc();
                }
            }
            Err(err) => {
                error!("Timed out sending transaction {:?}", err);
            }
        }
    }
}

impl Forwarder for QuicForwarder {
    fn process(&self, transaction: Transaction) {
        metrics::TX_RECEIVED_COUNT.inc();
        for tpu in transaction.tpu.iter() {
            self.spawn_transaction_forwarder(transaction.clone(), tpu);
        }
    }
}
