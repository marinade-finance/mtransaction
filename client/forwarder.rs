use crate::grpc_client::pb::Transaction;
use crate::metrics;
use crate::quic_forwarder::QuicForwarder;
use crate::rpc_forwarder::RpcForwarder;
use enum_dispatch::enum_dispatch;
use log::{info, warn};
use solana_sdk::signature::Keypair;
use std::net::IpAddr;
use tokio::sync::mpsc::UnboundedSender;

#[enum_dispatch]
enum TransactionForwarder {
    Quic(QuicForwarder),
    Rpc(RpcForwarder),
    Blackhole(BlackholeForwarder),
}

pub struct ForwardedTransaction {
    pub source: String,
    pub transaction: Transaction,
}

#[enum_dispatch(TransactionForwarder)]
pub trait Forwarder {
    fn process(&self, source: String, transaction: Transaction) -> ();
}

struct BlackholeForwarder {}
impl Forwarder for BlackholeForwarder {
    fn process(&self, source: String, transaction: Transaction) {
        metrics::TX_RECEIVED_COUNT
            .with_label_values(&[source.as_str()])
            .inc();
        metrics::TX_FORWARD_SUCCEEDED_COUNT
            .with_label_values(&[source.as_str()])
            .inc();
        info!(
            "Tx {} -> blackhole ({:?})",
            transaction.signature, transaction.tpu
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
    let forwarder = if blackhole {
        warn!("Blackholing all transactions!");
        TransactionForwarder::Blackhole(BlackholeForwarder {})
    } else if let Some(rpc_url) = rpc_url {
        if identity.is_some() || tpu_addr.is_some() {
            panic!("Cannot use parameters identity and tpu-addr when rpc-url is specified!");
        }
        TransactionForwarder::Rpc(RpcForwarder::new(rpc_url, throttle_parallel))
    } else {
        TransactionForwarder::Quic(QuicForwarder::new(identity, tpu_addr, throttle_parallel))
    };

    let (tx_transactions, mut rx_transactions) =
        tokio::sync::mpsc::unbounded_channel::<ForwardedTransaction>();

    tokio::spawn(async move {
        while let Some(transaction) = rx_transactions.recv().await {
            forwarder.process(transaction.source, transaction.transaction);
        }
    });

    tx_transactions
}
