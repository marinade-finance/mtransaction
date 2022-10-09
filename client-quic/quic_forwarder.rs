use crate::grpc_client::pb::Tx;
use base64::decode;
use log::{error, info};
use solana_client::{
    connection_cache::{ConnectionCache, DEFAULT_TPU_CONNECTION_POOL_SIZE},
    tpu_connection::TpuConnection,
};
use solana_sdk::signature::Keypair;
use std::{
    net::{IpAddr, SocketAddr},
    sync::Arc,
};
use tokio::sync::mpsc::UnboundedSender;

pub fn forward_tx(connection_cache: Arc<ConnectionCache>, tx: String) {
    info!("Sending tx....");
    let tx = decode(tx).unwrap();
    let tpu_addr = "141.98.217.56".parse().unwrap();
    let tpu_port = 8003;
    let tpu = SocketAddr::new(tpu_addr, tpu_port);
    let conn = connection_cache.get_connection(&tpu);
    if let Err(err) = conn.send_wire_transaction(tx) {
        error!("Failed to send the tx: {}", err);
    }
    info!("Tx sent!");
}

pub fn spawn_quic_forwarded(
    identity: Option<Keypair>,
    tpu_addr: Option<IpAddr>,
) -> UnboundedSender<Tx> {
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

    let (tx_queue, mut rx) = tokio::sync::mpsc::unbounded_channel::<crate::grpc_client::pb::Tx>();
    {
        tokio::spawn(async move {
            let connection_cache = Arc::new(connection_cache);
            while let Some(tx) = rx.recv().await {
                info!("Forwarding tx {:?}", &tx);

                {
                    let connection_cache = connection_cache.clone();
                    tokio::task::spawn_blocking(move || {
                        crate::quic_forwarder::forward_tx(connection_cache, tx.data);
                    });
                }
            }
        });
    }

    tx_queue
}