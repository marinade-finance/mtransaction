pub mod pb {
    tonic::include_proto!("validator");
}

use pb::m_transaction_client::MTransactionClient;
use pb::{Metrics, Pong, RequestMessageEnvelope};
use tonic::transport::{Certificate, Channel, ClientTlsConfig, Identity};

use env_logger::Env;
use futures::stream::Stream;
use log::{error, info, warn};
use std::sync::Arc;
use std::time::Duration;
use structopt::StructOpt;
use tokio::sync::RwLock;
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

async fn metrics_streaming(client: &mut MTransactionClient<Channel>) {
    let metrics_stream = tokio_stream::iter(std::iter::repeat(()))
        .map(|_| RequestMessageEnvelope {
            metrics: Some(Metrics {
                ..Default::default()
            }),
            ..Default::default()
        })
        .throttle(Duration::from_secs(15));

    let response = client.tx_stream(metrics_stream).await.unwrap();

    let mut resp_stream = response.into_inner();

    while let Some(received) = resp_stream.next().await {
        let received = received.unwrap();
        info!("received message: `{:?}`", received);
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

    let mut client = MTransactionClient::new(channel);
    metrics_streaming(&mut client).await;

    info!("Service stopped.");
    Ok(())
}
