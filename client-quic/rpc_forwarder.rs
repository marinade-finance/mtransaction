use crate::forwarder::Forwarder;
use crate::grpc_client::pb::Transaction;
use crate::metrics::Metrics;
use log::{error, info};
use serde_json::json;
use solana_client::{
    client_error::ClientErrorKind,
    nonblocking::rpc_client::RpcClient,
    rpc_config::RpcSendTransactionConfig,
    rpc_request::{RpcError, RpcRequest},
};
use solana_sdk::{commitment_config::CommitmentLevel, signature::Signature};
use solana_transaction_status::UiTransactionEncoding;
use std::sync::{atomic::Ordering, Arc};
use tokio::sync::Semaphore;

pub struct RpcForwarder {
    metrics: Arc<Metrics>,
    throttle_parallel: Arc<Semaphore>,
    rpc_client: Arc<RpcClient>,
}

impl RpcForwarder {
    pub fn new(rpc_url: String, metrics: Arc<Metrics>, throttle_parallel: usize) -> Self {
        Self {
            rpc_client: Arc::new(RpcClient::new(rpc_url)),
            metrics,
            throttle_parallel: Arc::new(Semaphore::new(throttle_parallel)),
        }
    }
}

impl Forwarder for RpcForwarder {
    fn process(&self, transaction: Transaction) {
        self.metrics.tx_received.fetch_add(1, Ordering::Relaxed);
        let rpc_client = self.rpc_client.clone();
        let throttle_parallel = self.throttle_parallel.clone();
        let metrics = self.metrics.clone();
        tokio::spawn(async move {
            let throttle_permit = throttle_parallel.acquire_owned().await.unwrap();

            info!("Tx {} -> {}", transaction.signature, &rpc_client.url());
            let _ = match rpc_client
                .send::<Signature>(
                    RpcRequest::SendTransaction,
                    json!([
                        transaction.data,
                        RpcSendTransactionConfig {
                            encoding: Some(UiTransactionEncoding::Base64),
                            preflight_commitment: Some(CommitmentLevel::Processed),
                            max_retries: None,
                            skip_preflight: true,
                            min_context_slot: None,
                        }
                    ]),
                )
                .await
            {
                Ok(_) => {
                    metrics.tx_forward_succeeded.fetch_add(1, Ordering::Relaxed);
                }
                Err(err) => {
                    if let ClientErrorKind::RpcError(RpcError::RpcResponseError {
                        code,
                        message,
                        data,
                    }) = &err.kind
                    {
                        error!(
                            "Failed to send the transaction, RPC error: {} {} {}",
                            code, message, data
                        );
                    } else {
                        error!("Failed to send the transaction: {}", err);
                    }
                    metrics.tx_forward_failed.fetch_add(1, Ordering::Relaxed);
                }
            };
            drop(throttle_permit);
        });
    }
}
