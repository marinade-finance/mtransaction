use crate::grpc_client::pb::Transaction;
use crate::metrics::Metrics;
use crate::quic_forwarder::QuicForwarder;
use crate::rpc_forwarder::RpcForwarder;
use enum_dispatch::enum_dispatch;
use log::{info, warn};
use solana_sdk::signature::Keypair;
use std::{net::IpAddr, sync::Arc};
use tokio::sync::mpsc::UnboundedSender;

#[enum_dispatch]
enum TransactionForwarder {
    Quic(QuicForwarder),
    Rpc(RpcForwarder),
    Blackhole(BlackholeForwarder),
}

#[enum_dispatch(TransactionForwarder)]
pub trait Forwarder {
    fn process(&self, transaction: Transaction) -> ();
}

struct BlackholeForwarder {}
impl Forwarder for BlackholeForwarder {
    fn process(&self, transaction: Transaction) {
        info!("Tx {} -> blackhole", transaction.signature);
    }
}

pub fn spawn_forwarder(
    identity: Option<Keypair>,
    tpu_addr: Option<IpAddr>,
    rpc_url: Option<String>,
    metrics: Arc<Metrics>,
    blackhole: bool,
    throttle_parallel: usize,
) -> UnboundedSender<Transaction> {
    let forwarder = if blackhole {
        warn!("Blackholing all transactions!");
        TransactionForwarder::Blackhole(BlackholeForwarder {})
    } else if let Some(rpc_url) = rpc_url {
        if identity.is_some() || tpu_addr.is_some() {
            panic!("Cannot use parameters identity and tpu-addr when rpc-url is specified!");
        }
        TransactionForwarder::Rpc(RpcForwarder::new(rpc_url, metrics, throttle_parallel))
    } else {
        TransactionForwarder::Quic(QuicForwarder::new(
            identity,
            tpu_addr,
            metrics,
            throttle_parallel,
        ))
    };

    let (tx_transactions, mut rx_transactions) =
        tokio::sync::mpsc::unbounded_channel::<Transaction>();

    tokio::spawn(async move {
        while let Some(transaction) = rx_transactions.recv().await {
            forwarder.process(transaction);
        }
    });

    tx_transactions
}
