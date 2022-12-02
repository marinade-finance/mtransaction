pub mod pb {
    tonic::include_proto!("validator");
}
use crate::metrics::Metrics;
use log::{error, info, warn};
use pb::{
    m_transaction_client::MTransactionClient, Ping, Pong, RequestMessageEnvelope,
    ResponseMessageEnvelope, Transaction,
};
use std::sync::Arc;
use tokio::{
    sync::mpsc::UnboundedSender,
    time::{sleep, Duration},
};
use tokio_stream::StreamExt;
use tonic::{
    transport::{Certificate, Channel, ClientTlsConfig, Identity, Uri},
    Status,
};
use sysinfo::{ProcessExt, System, SystemExt};

pub const DEFAULT_VALIDATOR_WARMUP_MINUTES: u64 = 15;

fn validator_check() -> Result<String, String> {
    let sys = System::new_all();
    for process in sys.processes_by_name("solana-valid") {
        if process.run_time() / 60 > DEFAULT_VALIDATOR_WARMUP_MINUTES {
            return Ok("Validator is running.".to_string());
        } else
        {
            error!("Validator is not warmed up. Time left: {} min.", DEFAULT_VALIDATOR_WARMUP_MINUTES - (process.run_time() / 60));
            return Err("Validator is not warmed up.".to_string());
        }
    }
    error!("Validator is not running.");
    Err("Validator is not running.".to_string())
}

fn process_ping(ping: Ping, tx_upstream_transactions: UnboundedSender<RequestMessageEnvelope>) {
    info!("Sending pong: {}", ping.id);
    if let Err(err) = tx_upstream_transactions.send(RequestMessageEnvelope {
        pong: Some(Pong { id: ping.id }),
        ..Default::default()
    }) {
        error!("Failed to enqueue pong: {}", err);
    }
}

fn process_transaction(transaction: Transaction, tx_transactions: UnboundedSender<Transaction>) {
    if let Err(err) = tx_transactions.send(transaction) {
        error!("Failed to enqueue tx: {}", err);
    }
}

fn process_upstream_message(
    response: Result<ResponseMessageEnvelope, Status>,
    tx_upstream_transactions: UnboundedSender<RequestMessageEnvelope>,
    tx_transactions: UnboundedSender<Transaction>,
) {
    match response {
        Ok(response_message_envelope) => {
            if let Some(ping) = response_message_envelope.ping {
                process_ping(ping, tx_upstream_transactions);
            }
            if let Some(transaction) = response_message_envelope.transaction {
                process_transaction(transaction, tx_transactions);
            }
        }
        Err(err) => {
            error!("Received error from upstream: {}", err);
        }
    };
}

async fn mtx_stream(
    client: &mut MTransactionClient<Channel>,
    tx_transactions: tokio::sync::mpsc::UnboundedSender<Transaction>,
    metrics: Arc<Metrics>,
) -> Result<String,String> {
    let metrics_stream = async_stream::stream! {
        loop {
            yield metrics.as_ref().into();
            sleep(Duration::from_secs(10)).await;
        }
    };
    futures::pin_mut!(metrics_stream);

    let (tx_upstream_transactions, mut rx_upstream_transactions) =
        tokio::sync::mpsc::unbounded_channel::<RequestMessageEnvelope>();
    let request_stream = async_stream::stream! {
        while let Some(item) = rx_upstream_transactions.recv().await {
            yield item;
        }
    };

    let response = client.tx_stream(request_stream).await.unwrap();
    let mut response_stream = response.into_inner();

    loop {
        let result = validator_check();
        if result.is_err() {
            return Err("Validator not connected.".to_string()) 
        }
        tokio::select! {
            metrics = metrics_stream.next() => {
                if let Some(metrics) = metrics {
                    info!("Sending metrics: {:?}", metrics);
                    if let Err(err) = tx_upstream_transactions.send(metrics) {
                        error!("Failed to enqueue metrics: {}", err);
                    }
                } else {
                    error!("Stream of metrics dropped!");
                    break
                }
            }

            response = response_stream.next() => {
                if let Some(response) = response {
                    process_upstream_message(response, tx_upstream_transactions.clone(), tx_transactions.clone());
                } else {
                    error!("Upstream closed!");
                    break
                }
            }
        }
    }
    Err("Stream closing.".to_string())
}

async fn get_tls_config(
    tls_ca_cert: Option<String>,
    tls_client_key: Option<String>,
    tls_client_cert: Option<String>,
) -> std::result::Result<Option<ClientTlsConfig>, Box<dyn std::error::Error>> {
    let tls = if let (Some(ca_cert), Some(client_key), Some(client_cert)) =
        (tls_ca_cert, tls_client_key, tls_client_cert)
    {
        info!("Loading CA from {}", ca_cert);
        let ca_cert = tokio::fs::read(ca_cert).await?;
        let ca_cert = Certificate::from_pem(ca_cert);

        info!("Loading client TLS from {} and {}", client_cert, client_key);
        let client_cert = tokio::fs::read(client_cert).await?;
        let client_key = tokio::fs::read(client_key).await?;
        let client_identity = Identity::from_pem(client_cert, client_key);

        Some(
            ClientTlsConfig::new()
                .ca_certificate(ca_cert)
                .domain_name("localhost")
                .identity(client_identity),
        )
    } else {
        warn!("TLS is disabled by (lack of) configuration!");
        None
    };

    Ok(tls)
}

pub async fn spawn_grpc_client(
    grpc_url: Uri,
    tls_grpc_ca_cert: Option<String>,
    tls_grpc_client_key: Option<String>,
    tls_grpc_client_cert: Option<String>,
    tx_transactions: tokio::sync::mpsc::UnboundedSender<Transaction>,
    metrics: Arc<Metrics>,
) -> std::result::Result<(), Box<dyn std::error::Error>> {
    info!("Loading TLS configuration.");
    let tls = get_tls_config(tls_grpc_ca_cert, tls_grpc_client_key, tls_grpc_client_cert).await?;
    let mut validator_online = true;

    loop {
        if validator_online {
            info!("Opening the gRPC channel.");
            let channel = match tls {
                Some(ref tls) => Channel::builder(grpc_url.clone()).tls_config(tls.clone())?,
                _ => Channel::builder(grpc_url.clone()),
            }
            .connect()
            .await?;
            info!("Streaming from gRPC server.");

            let mut client = MTransactionClient::new(channel.clone());
            let result = mtx_stream(&mut client, tx_transactions.clone(), metrics.clone()).await;

            if result.is_err() {
                if result.unwrap_err().contains("Validator") {
                    validator_online = false;
                }
            }
        }
        if !validator_online {
            warn!("Retrying streaming from gRPC server in 30s.");
            sleep(Duration::from_secs(30)).await;
            let result = validator_check();
            if result.is_ok() {
                validator_online = true;
            }
            continue;
        } else {
            break;
        }
    }
    Ok(())
}
