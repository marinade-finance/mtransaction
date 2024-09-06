use crate::forwarder::{ForwardedTransaction, Forwarder};
use crate::metrics::{self};
use crate::time_ms;
use log::{error, info};
use serde_json::json;
use solana_client::{
    client_error::ClientErrorKind,
    nonblocking::rpc_client::RpcClient,
    rpc_config::RpcSendTransactionConfig,
    rpc_request::{RpcError, RpcRequest},
};
use solana_sdk::commitment_config::CommitmentLevel;
use solana_transaction_status::UiTransactionEncoding;
use std::sync::Arc;
use tokio::sync::Semaphore;

pub struct RpcForwarder {
    throttle_parallel: Arc<Semaphore>,
    rpc_client: Arc<RpcClient>,
}

impl RpcForwarder {
    pub fn new(rpc_url: String, throttle_parallel: usize) -> Self {
        Self {
            rpc_client: Arc::new(RpcClient::new(rpc_url)),
            throttle_parallel: Arc::new(Semaphore::new(throttle_parallel)),
        }
    }
}

impl Forwarder for RpcForwarder {
    fn process(&self, tx: ForwardedTransaction) {
        let source = tx.source;
        let transaction = tx.transaction;
        metrics::TX_RECEIVED_COUNT
            .with_label_values(&[source.as_str()])
            .inc();
        metrics::TX_RECEIVED_LATENCY
            .with_label_values(&[source.as_str()])
            .add((time_ms().max(tx.ctime) - tx.ctime).try_into().unwrap_or(0));
        let rpc_client = self.rpc_client.clone();
        let throttle_parallel = self.throttle_parallel.clone();
        tokio::spawn(async move {
            let throttle_permit = throttle_parallel.acquire_owned().await.unwrap();

            info!("Tx {} -> {}", transaction.signature, &rpc_client.url());
            let config = RpcSendTransactionConfig {
                encoding: Some(UiTransactionEncoding::Base64),
                preflight_commitment: Some(CommitmentLevel::Processed),
                max_retries: None,
                skip_preflight: true,
                min_context_slot: None,
            };
            match rpc_client
                .send::<String>(
                    RpcRequest::SendTransaction,
                    json!([transaction.data, config]),
                )
                .await
            {
                Ok(_) => {
                    metrics::TX_FORWARD_SUCCEEDED_COUNT
                        .with_label_values(&[source.as_str()])
                        .inc();
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
                    metrics::TX_FORWARD_FAILED_COUNT
                        .with_label_values(&[source.as_str()])
                        .inc();
                }
            };
            drop(throttle_permit);
        });
    }
}
