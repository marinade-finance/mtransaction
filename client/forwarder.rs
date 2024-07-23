use crate::grpc_client::pb::Transaction;
use crate::metrics;
use crate::quic_forwarder::QuicForwarder;
use crate::rpc_forwarder::RpcForwarder;
use log::{info, warn};
use solana_sdk::signature::Keypair;
use std::net::IpAddr;
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};

#[derive(Debug)]
pub struct ForwardedTransaction {
    pub ctime: u64,
    pub source: String,
    pub transaction: Transaction,
}

pub trait Forwarder {
    fn process(&self, tx: ForwardedTransaction);
}

struct BlackholeForwarder {}
impl Forwarder for BlackholeForwarder {
    fn process(&self, tx: ForwardedTransaction) {
        let transaction = tx.transaction;
        let source = tx.source;
        metrics::TX_RECEIVED_COUNT
            .with_label_values(&[source.as_str()])
            .inc();
        metrics::TX_FORWARD_SUCCEEDED_COUNT
            .with_label_values(&[source.as_str()])
            .inc();
        info!(
            "Tx {} -> blackhole ({:?})",
            transaction.signature, transaction
        );
    }
}

pub fn spawn_forwarder(
    identity: Option<Keypair>,
    tpu_addr: Option<IpAddr>,
    rpc_url: Option<String>,
    blackhole: bool,
    throttle_parallel: usize,
) -> UnboundedSender<ForwardedTransaction> {
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel::<ForwardedTransaction>();
    if blackhole {
        warn!("Blackholing all transactions!");
        tokio::spawn(forwarder_loop(rx, BlackholeForwarder {}));
    } else if let Some(rpc_url) = rpc_url {
        if identity.is_some() || tpu_addr.is_some() {
            panic!("Cannot use parameters identity and tpu-addr when rpc-url is specified!");
        }
        tokio::spawn(forwarder_loop(
            rx,
            RpcForwarder::new(rpc_url, throttle_parallel),
        ));
    } else {
        tokio::spawn(forwarder_loop(
            rx,
            QuicForwarder::new(identity, tpu_addr, throttle_parallel),
        ));
    };

    tx
}

pub async fn forwarder_loop<T: Forwarder>(
    mut rx_transactions: UnboundedReceiver<ForwardedTransaction>,
    forwarder: T,
) {
    while let Some(tx) = rx_transactions.recv().await {
        forwarder.process(tx);
    }
}
