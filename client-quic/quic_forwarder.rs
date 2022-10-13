use crate::grpc_client::pb::Transaction;
use crate::metrics::Metrics;
use base64::decode;
use log::{error, info};
use solana_client::{
    connection_cache::{ConnectionCache, DEFAULT_TPU_CONNECTION_POOL_SIZE},
    tpu_connection::TpuConnection,
};
use solana_sdk::signature::Keypair;
use std::{
    net::{IpAddr, SocketAddr},
    sync::{atomic::Ordering, Arc},
};
use tokio::sync::mpsc::UnboundedSender;

pub fn forward_transaction(
    connection_cache: Arc<ConnectionCache>,
    signature: String,
    transaction_data_b64: String,
    tpu: SocketAddr,
    metrics: Arc<Metrics>,
) {
    let wire_transaction = decode(transaction_data_b64).unwrap();
    let conn = connection_cache.get_connection(&tpu);
    if let Err(err) = conn.send_wire_transaction(wire_transaction) {
        error!("Failed to send the transaction: {}", err);
        metrics.tx_forward_failed.fetch_add(1, Ordering::Relaxed);
    } else {
        metrics.tx_forward_succeeded.fetch_add(1, Ordering::Relaxed);
    }
    info!("Tx {} -> {}", signature, &tpu);
}

pub fn spawn_quic_forwarded(
    identity: Option<Keypair>,
    tpu_addr: Option<IpAddr>,
    metrics: Arc<Metrics>,
) -> UnboundedSender<Transaction> {
    let mut connection_cache = ConnectionCache::new(DEFAULT_TPU_CONNECTION_POOL_SIZE);
    if let (Some(identity), Some(tpu_addr)) = (identity, tpu_addr) {
        if let Err(err) = connection_cache.update_client_certificate(&identity, tpu_addr) {
            error!("Failed to update client certificate: {}", err);
        }
        info!(
            "Updated QUIC certificate with identity {:?} at {}",
            &identity, tpu_addr
        );
    }

    let (tx_transactions, mut rx_transactions) =
        tokio::sync::mpsc::unbounded_channel::<crate::grpc_client::pb::Transaction>();
    {
        tokio::spawn(async move {
            let connection_cache = Arc::new(connection_cache);
            while let Some(transaction) = rx_transactions.recv().await {
                metrics.tx_received.fetch_add(1, Ordering::Relaxed);
                info!("Forwarding tx {:?}", &transaction.signature);
                for tpu in transaction.tpu {
                    let tx_data = transaction.data.clone();
                    let tx_signature = transaction.signature.clone();
                    let connection_cache = connection_cache.clone();
                    let metrics = metrics.clone();
                    tokio::task::spawn_blocking(move || {
                        let tpu = tpu.parse().unwrap();
                        crate::quic_forwarder::forward_transaction(
                            connection_cache,
                            tx_signature,
                            tx_data,
                            tpu,
                            metrics,
                        );
                    });
                }
            }
        });
    }

    tx_transactions
}
