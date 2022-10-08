pub mod pb {
    tonic::include_proto!("validator");
}

use pb::m_transaction_client::MTransactionClient;
use pb::{Metrics, Pong, RequestMessageEnvelope, Tx};
use tonic::transport::{Certificate, Channel, ClientTlsConfig, Identity};

use env_logger::Env;
use log::{error, info, warn};
use std::time::Duration;
use structopt::StructOpt;
use tokio_stream::StreamExt;

#[derive(Debug, StructOpt)]
struct Params {
    #[structopt(long = "tls-grpc-ca-cert")]
    tls_grpc_ca_cert: Option<String>,

    #[structopt(long = "tls-grpc-client-key")]
    tls_grpc_client_key: Option<String>,

    #[structopt(long = "tls-grpc-client-cert")]
    tls_grpc_client_cert: Option<String>,

    #[structopt(long = "grpc-url", default_value = "http://127.0.0.1:50051")]
    grpc_url: String,

    #[structopt(long = "identity")]
    identity: Option<String>,
}

async fn metrics_streaming(
    client: &mut MTransactionClient<Channel>,
    tx_queue: tokio::sync::mpsc::UnboundedSender<Tx>,
) {
    let mut metrics_stream = Box::pin(
        tokio_stream::iter(std::iter::repeat(()))
            .map(|_| RequestMessageEnvelope {
                metrics: Some(Metrics {
                    ..Default::default()
                }),
                ..Default::default()
            })
            .throttle(Duration::from_secs(15)),
    );

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
                    error!("What are you doing, metrics?");
                }
            }

            response_message_envelope = response_stream.next() => {
                if let Some(response_message_envelope) = response_message_envelope {
                    info!("Upstream: {:?}", response_message_envelope);
                    if let Ok(response_message_envelope) = response_message_envelope {
                        if let Some(ping) = response_message_envelope.ping {
                            info!("Sending pong: {}", ping.id);
                            if let Err(err) = tx_response.send(RequestMessageEnvelope {
                                pong: Some(Pong {
                                    id: ping.id,
                                }),
                                ..Default::default()
                            }) {
                                error!("Failed to enqueue pong: {}", err);
                            }
                        }
                        if let Some(tx) = response_message_envelope.tx {
                            info!("Enqueuing tx: {:?}", &tx);
                            if let Err(err) = tx_queue.send(tx) {
                                error!("Failed to enqueue tx: {}", err);
                            }
                        }
                    }
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

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    env_logger::Builder::from_env(Env::default().default_filter_or("info")).init();
    let params = Params::from_args();

    let tls = get_tls_config(
        params.tls_grpc_ca_cert,
        params.tls_grpc_client_key,
        params.tls_grpc_client_cert,
    )
    .await?;

    let grpc_url = params.grpc_url.parse().unwrap();
    let channel = match tls {
        Some(tls) => Channel::builder(grpc_url).tls_config(tls)?,
        _ => Channel::builder(grpc_url),
    }
    .connect()
    .await?;

    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    {
        tokio::spawn(async move {
            while let Some(tx) = rx.recv().await {
                info!("Forwarding tx {:?}", &tx);
            }
        });
    }

    let mut client = MTransactionClient::new(channel);
    metrics_streaming(&mut client, tx).await;

    info!("Service stopped.");
    Ok(())
}
