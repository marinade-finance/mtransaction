use crate::bundler::Bundler;
use crate::grpc_client::pb::Transaction;
use crate::metrics::Metrics;
use base64::decode;
use log::{error, info};
use solana_client::{
    connection_cache::{ConnectionCache, DEFAULT_TPU_CONNECTION_POOL_SIZE},
    nonblocking::tpu_connection::TpuConnection,
};
use solana_sdk::signature::Keypair;
use std::{
    iter::repeat,
    net::{IpAddr, SocketAddr},
    sync::{atomic::Ordering, Arc},
};
use tokio::{
    sync::{mpsc::UnboundedSender, Semaphore},
    time::Duration,
};
use tokio_stream::{wrappers::UnboundedReceiverStream, StreamExt};

const MAX_QUEUE_LENGTH: usize = 100;

struct QuicForwarder {
    metrics: Arc<Metrics>,
    blackhole: bool,
    throttle_parallel: Arc<Semaphore>,
    connection_cache: Arc<ConnectionCache>,
    bundler: Bundler,
}

impl QuicForwarder {
    pub fn new(
        identity: Option<Keypair>,
        tpu_addr: Option<IpAddr>,
        metrics: Arc<Metrics>,
        blackhole: bool,
        throttle_parallel: usize,
    ) -> Self {
        let mut connection_cache = ConnectionCache::new(DEFAULT_TPU_CONNECTION_POOL_SIZE);
        if let (Some(identity), Some(tpu_addr)) = (identity, tpu_addr) {
            if let Err(err) = connection_cache.update_client_certificate(&identity, tpu_addr) {
                error!("Failed to update client certificate: {}", err);
            }
            info!("Updated QUIC certificate");
        }

        let quic_forwarder = Self {
            metrics,
            blackhole,
            throttle_parallel: Arc::new(Semaphore::new(throttle_parallel)),
            connection_cache: Arc::new(connection_cache),
            bundler: Bundler::new(),
        };

        quic_forwarder
    }

    pub fn process(&mut self, transaction: Transaction) {
        self.metrics.tx_received.fetch_add(1, Ordering::Relaxed);
        for tpu in transaction.tpu.iter() {
            let record = (transaction.signature.clone(), transaction.data.clone());
            let queue_length = self.bundler.enqueue(&tpu, record);
            if queue_length >= MAX_QUEUE_LENGTH {
                self.spawn_bundle_sender(&tpu);
            }
        }
    }

    fn spawn_bundle_sender(&mut self, tpu: &String) {
        if let Some(bundle) = self.bundler.drain(tpu) {
            let tpu = tpu.clone();
            let throttle_parallel = self.throttle_parallel.clone();
            let connection_cache = self.connection_cache.clone();
            let metrics = self.metrics.clone();
            let blackhole = self.blackhole;

            tokio::spawn(async move {
                let permit = throttle_parallel.acquire_owned().await.unwrap();
                let tpu = tpu.parse().unwrap();
                crate::quic_forwarder::forward_transaction_bundle(
                    connection_cache,
                    bundle
                        .iter()
                        .map(|(_, data)| decode(data).unwrap())
                        .collect(),
                    tpu,
                    metrics,
                    blackhole,
                )
                .await;
                drop(permit);
            });
        }
    }

    fn bundle_timeout(&mut self) {
        for tpu in self.bundler.get_tpus() {
            self.spawn_bundle_sender(&tpu);
        }
    }
}

pub async fn forward_transaction_bundle(
    connection_cache: Arc<ConnectionCache>,
    wire_transactions: Vec<Vec<u8>>,
    tpu: SocketAddr,
    metrics: Arc<Metrics>,
    blackhole: bool,
) {
    let len = wire_transactions.len() as u64;
    info!("Tx bundle({}) -> {}", len, &tpu);
    if blackhole {
        return;
    }
    let conn = connection_cache.get_nonblocking_connection(&tpu);
    if let Err(err) = conn.send_wire_transaction_batch(&wire_transactions).await {
        error!("Failed to send the transaction: {}", err);
        metrics.tx_forward_failed.fetch_add(len, Ordering::Relaxed);
    } else {
        metrics
            .tx_forward_succeeded
            .fetch_add(len, Ordering::Relaxed);
    }
}

pub fn spawn_quic_forwarder(
    identity: Option<Keypair>,
    tpu_addr: Option<IpAddr>,
    metrics: Arc<Metrics>,
    blackhole: bool,
    throttle_parallel: usize,
) -> UnboundedSender<Transaction> {
    let mut quic_forwarder =
        QuicForwarder::new(identity, tpu_addr, metrics, blackhole, throttle_parallel);

    let (tx_transactions, rx_transactions) = tokio::sync::mpsc::unbounded_channel::<Transaction>();

    let mut rx_transactions = UnboundedReceiverStream::new(rx_transactions);

    let mut bundle_timeout_signal =
        Box::pin(tokio_stream::iter(repeat(())).throttle(Duration::from_millis(10)));

    tokio::spawn(async move {
        loop {
            tokio::select! {
                _ = bundle_timeout_signal.next() => {
                    quic_forwarder.bundle_timeout();
                },

                Some(transaction) = rx_transactions.next() => {
                    quic_forwarder.process(transaction);
                },

                else => break
            }
        }
    });

    tx_transactions
}
