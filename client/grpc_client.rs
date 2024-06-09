pub mod pb {
    tonic::include_proto!("validator");
}
use crate::forwarder::ForwardedTransaction;
use crate::time_us;
use log::{error, info, warn};
use pb::{
    m_transaction_client::MTransactionClient, Pong, RequestMessageEnvelope,
    ResponseMessageEnvelope, Rtt, Transaction,
};
use solana_sdk::signature::Keypair;
use solana_client::connection_cache::ConnectionCache;
use solana_client::nonblocking::tpu_connection::TpuConnection;
use solana_client::quic_client::QuicTpuConnection;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::sync::mpsc::UnboundedSender;
use tokio_stream::StreamExt;
use tonic::{
    transport::{Certificate, Channel, ClientTlsConfig, Identity, Uri},
    Status,
};

async fn tpu_ping(outbox: UnboundedSender<RequestMessageEnvelope>, identity: Arc<Keypair>, tpu: String) {
    info!("tpu_ping spawn ping to {tpu}");
    // let cert_info = if let (Some(identity), Some(tpu_addr)) = (&identity, tpu_addr) {
    //         Some((identity, tpu_addr))
    // let ident = QuicClientCertificateKeypair::new()

    // let endpoint = QuicLazyInitializedEndpoint::new(ident, None);
    // QuicTpuConnection::new(None, tpu.parse().unwrap(), None);
    let tpu_addr: SocketAddr = match tpu.parse() {
        Ok(value) => value,
        Err(err) => {
            error!("tpu_ping failed to parse tpu into SocketAddr: {err}");
            return;
        }
    };
    let connection_cache = ConnectionCache::new_with_client_options(
        "default connection cache",
        2,
        None,
        Some((&identity, tpu_addr.ip())),
        None,
    );
    let conn = connection_cache.get_nonblocking_connection(&tpu_addr);
    let t0 = time_us();
    if let Err(err) = conn.send_data(&[0]).await {
        error!("tpu_ping failed to send data: {err}");
        return;
    };
    let took = time_us() - t0;

    let msg = RequestMessageEnvelope {
        rtt: Some(Rtt {
            ip: tpu,
            took,
            n: 1,
        }),
        ..Default::default()
    };
    if let Err(err) = outbox.send(msg) {
        error!("tpu_ping failed to send rtt stats: {err}");
    }
}

fn process_upstream_message(
    source: String,
    response: Result<ResponseMessageEnvelope, Status>,
    outbox: UnboundedSender<RequestMessageEnvelope>,
    tx_transactions: UnboundedSender<ForwardedTransaction>,
    identity: Arc<Keypair>,
) {
    match response {
        Ok(envelope) => {
            if let Some(ping) = envelope.ping {
                info!("Sending pong: {}", ping.id);
                let msg = RequestMessageEnvelope {
                    pong: Some(Pong { id: ping.id }),
                    ..Default::default()
                };
                if let Err(err) = outbox.send(msg) {
                    error!("Failed to enqueue pong: {}", err);
                }
            } else if let Some(transaction) = envelope.transaction {
                let tx = ForwardedTransaction {
                    ctime: 0,
                    source,
                    transaction,
                };
                if let Err(err) = tx_transactions.send(tx) {
                    error!("Failed to enqueue tx: {}", err);
                }
            } else if let Some(transaction) = envelope.timed_transaction {
                let tx = ForwardedTransaction {
                    ctime: transaction.ctime,
                    source,
                    transaction: Transaction {
                        signature: transaction.signature,
                        data: transaction.data,
                        tpu: transaction.tpu,
                    },
                };
                if let Err(err) = tx_transactions.send(tx) {
                    error!("Failed to enqueue tx: {}", err);
                }
            } else if let Some(tpus) = envelope.leader_tpus {
                for tpu in tpus.tpu {
                    tokio::spawn(tpu_ping(outbox.clone(), identity.clone(), tpu));
                }
            }
        }
        Err(err) => {
            error!("Received error from upstream: {}", err);
        }
    };
}

async fn mtx_stream(
    source: String,
    identity: Arc<Keypair>,
    client: &mut MTransactionClient<Channel>,
    tx_transactions: tokio::sync::mpsc::UnboundedSender<ForwardedTransaction>,
    mut metrics: tokio::sync::mpsc::Receiver<RequestMessageEnvelope>,
) -> std::result::Result<(), Box<dyn std::error::Error>> {
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
        tokio::select! {
            metrics = metrics.recv() => {
                if let Some(metrics) = metrics {
                    info!("Sending metrics: {:?}", metrics);
                    if let Err(err) = tx_upstream_transactions.send(metrics) {
                        error!("Failed to enqueue metrics, source: {:?} err {}", source, err);
                    }
                } else {
                    error!("Stream of metrics dropped: {:?}", source);
                    break
                }
            }

            response = response_stream.next() => {
                if let Some(response) = response {
                    process_upstream_message(source.clone(), response, tx_upstream_transactions.clone(), tx_transactions.clone(), identity.clone());
                } else {
                    metrics.close();
                    error!("Upstream closed: {:?}", source);
                    return Err("Upstream closed".into());
                }
            }
        }
    }
    Ok(())
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
    identity: Arc<Keypair>,
    grpc_url: Uri,
    tls_grpc_ca_cert: Option<String>,
    tls_grpc_client_key: Option<String>,
    tls_grpc_client_cert: Option<String>,
    tx_transactions: tokio::sync::mpsc::UnboundedSender<ForwardedTransaction>,
    metrics: tokio::sync::mpsc::Receiver<RequestMessageEnvelope>,
) -> std::result::Result<(), Box<dyn std::error::Error>> {
    info!("Loading TLS configuration.");
    let tls = get_tls_config(tls_grpc_ca_cert, tls_grpc_client_key, tls_grpc_client_cert).await?;

    let _domain_name = grpc_url.host();

    info!("Opening the gRPC channel: {:?}", grpc_url.host());
    let channel = match tls {
        Some(tls) => Channel::builder(grpc_url.clone()).tls_config(tls)?,
        _ => Channel::builder(grpc_url.clone()),
    }
    .connect()
    .await?;

    let grpc_host = grpc_url.host();

    info!("Streaming from gRPC server: {:?}", grpc_host);

    let mut client = MTransactionClient::new(channel);

    mtx_stream(
        grpc_host.unwrap_or("unknown").to_string(),
        identity,
        &mut client,
        tx_transactions,
        metrics,
    )
    .await
    .map(|v| {
        info!("Stream ended from gRPC server: {:?}, {:?}", grpc_host, v);
        v
    })
}
