pub mod pb {
    tonic::include_proto!("validator");
}
use log::{error, info, warn};
use pb::{
    m_transaction_client::MTransactionClient, Metrics, Ping, Pong, RequestMessageEnvelope,
    ResponseMessageEnvelope, Tx,
};
use std::{pin::Pin, time::Duration};
use tokio::sync::mpsc::UnboundedSender;
use tokio_stream::{Stream, StreamExt};
use tonic::{
    transport::{Certificate, Channel, ClientTlsConfig, Identity, Uri},
    Status,
};

fn process_ping(ping: Ping, tx_response: UnboundedSender<RequestMessageEnvelope>) {
    info!("Sending pong: {}", ping.id);
    if let Err(err) = tx_response.send(RequestMessageEnvelope {
        pong: Some(Pong { id: ping.id }),
        ..Default::default()
    }) {
        error!("Failed to enqueue pong: {}", err);
    }
}

fn process_tx(tx: Tx, tx_queue: UnboundedSender<Tx>) {
    info!("Enqueuing tx: {:?}", &tx);
    if let Err(err) = tx_queue.send(tx) {
        error!("Failed to enqueue tx: {}", err);
    }
}

fn process_upstream_message(
    response: Result<ResponseMessageEnvelope, Status>,
    tx_response: UnboundedSender<RequestMessageEnvelope>,
    tx_queue: UnboundedSender<Tx>,
) {
    info!("Upstream: {:?}", response);

    match response {
        Ok(response_message_envelope) => {
            if let Some(ping) = response_message_envelope.ping {
                process_ping(ping, tx_response);
            }
            if let Some(tx) = response_message_envelope.tx {
                process_tx(tx, tx_queue);
            }
        }
        Err(err) => {
            error!("Received error from upstream: {}", err);
        }
    };
}

fn create_metrics_stream() -> Pin<Box<dyn Stream<Item = RequestMessageEnvelope>>> {
    Box::pin(
        tokio_stream::iter(std::iter::repeat(()))
            .map(|_| RequestMessageEnvelope {
                metrics: Some(Metrics {
                    ..Default::default()
                }),
                ..Default::default()
            })
            .throttle(Duration::from_secs(15)),
    )
}

async fn mtx_stream(
    client: &mut MTransactionClient<Channel>,
    tx_queue: tokio::sync::mpsc::UnboundedSender<Tx>,
) {
    let mut metrics_stream = create_metrics_stream();

    let (tx_response, mut rx) = tokio::sync::mpsc::unbounded_channel::<RequestMessageEnvelope>();
    let request_stream = async_stream::stream! {
        while let Some(item) = rx.recv().await {
            yield item;
        }
    };

    let response = client.tx_stream(request_stream).await.unwrap();
    let mut response_stream = response.into_inner();

    loop {
        tokio::select! {
            metrics = metrics_stream.next() => {
                if let Some(metrics) = metrics {
                    info!("Sending metrics: {:?}", metrics);
                    if let Err(err) = tx_response.send(metrics) {
                        error!("Failed to enqueue metrics: {}", err);
                    }
                } else {
                    error!("Stream of metrics dropped!");
                    break
                }
            }

            response = response_stream.next() => {
                if let Some(response) = response {
                    process_upstream_message(response, tx_response.clone(), tx_queue.clone());
                } else {
                    error!("Upstream closed!");
                    break
                }
            }
        }
    }
}

async fn get_tls_config(
    tls_ca_cert: Option<String>,
    tls_client_key: Option<String>,
    tls_client_cert: Option<String>,
) -> std::result::Result<Option<ClientTlsConfig>, Box<dyn std::error::Error>> {
    let tls = if let (Some(ca_cert), Some(client_key), Some(client_cert)) =
        (tls_ca_cert, tls_client_key, tls_client_cert)
    {
        info!("Loading CA from {}.", ca_cert);
        let ca_cert = tokio::fs::read(ca_cert).await?;
        let ca_cert = Certificate::from_pem(ca_cert);

        info!(
            "Loading client TLS from {} and {}.",
            client_cert, client_key
        );
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
    tx_queue: tokio::sync::mpsc::UnboundedSender<Tx>,
) -> std::result::Result<(), Box<dyn std::error::Error>> {
    info!("Loading TLS configuration.");
    let tls = get_tls_config(tls_grpc_ca_cert, tls_grpc_client_key, tls_grpc_client_cert).await?;

    info!("Opening the gRPC channel.");
    let channel = match tls {
        Some(tls) => Channel::builder(grpc_url).tls_config(tls)?,
        _ => Channel::builder(grpc_url),
    }
    .connect()
    .await?;

    info!("Streaming from gRPC server.");
    let mut client = MTransactionClient::new(channel);
    mtx_stream(&mut client, tx_queue).await;

    Ok(())
}
