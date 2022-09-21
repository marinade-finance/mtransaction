pub mod pb {
    tonic::include_proto!("validator");
}
use crate::balancer::*;
use futures::Stream;
use jsonrpc_http_server::*;
use log::{debug, error, info, warn};
use rand::Rng;
use std::sync::Arc;
use std::{error::Error, io::ErrorKind, net::ToSocketAddrs, pin::Pin, time::Duration};
use structopt::StructOpt;
use tokio::sync::{mpsc, RwLock};
use tokio_stream::{wrappers::ReceiverStream, StreamExt};
use tonic::transport::{Certificate, Identity, Server, ServerTlsConfig};
use tonic::{Request, Response, Status, Streaming};

use pb::{TxMessage, TxMessageRequest};

type TxMessageStream = Pin<Box<dyn Stream<Item = std::result::Result<TxMessage, Status>> + Send>>;

pub struct MTransactionServer {
    balancer: Arc<RwLock<Balancer>>,
}

#[tonic::async_trait]
impl pb::m_transaction_server::MTransaction for MTransactionServer {
    type TxStreamStream = TxMessageStream;
    async fn tx_stream(
        &self,
        req: Request<TxMessageRequest>,
    ) -> std::result::Result<Response<Self::TxStreamStream>, Status> {
        let identity = req.remote_addr();
        // let identity = "test".to_string();
        info!("Client connected: {:?}.", &identity);

        let mut balancer = self.balancer.write().await;
        let (tx, rx) = balancer.subscribe(format!("{:?}", &identity));
        let output_stream = ReceiverStream::new(rx);

        {
            let balancer = self.balancer.clone();
            tokio::spawn(async move {
                tx.closed().await;
                info!("The client has disconnected {:?}", &identity);
                balancer
                    .write()
                    .await
                    .unsubscribe(&format!("{:?}", &identity));
            });
        }

        Ok(Response::new(
            Box::pin(output_stream) as Self::TxStreamStream
        ))
    }
}

async fn get_tls_config(
    tls_server_cert: Option<String>,
    tls_server_key: Option<String>,
    tls_client_ca_cert: Option<String>,
) -> std::result::Result<Option<ServerTlsConfig>, Box<dyn std::error::Error>> {
    let tls = ServerTlsConfig::new();

    let tls = if let (Some(cert), Some(key)) = (tls_server_cert, tls_server_key) {
        info!("Loading server TLS from from {} and {}", cert, key);
        let cert = tokio::fs::read(cert).await?;
        let key = tokio::fs::read(key).await?;
        let tls = tls.identity(Identity::from_pem(cert, key));

        if let Some(cert) = tls_client_ca_cert {
            info!("Loading client CA from from {}", cert);
            let cert = tokio::fs::read(cert).await?;
            Some(tls.client_ca_root(Certificate::from_pem(cert)))
        } else {
            Some(tls)
        }
    } else {
        warn!("TLS is disabled by (lack of) configuration!");
        None
    };

    Ok(tls)
}

pub async fn spawn_grpc_server(
    grpc_addr: std::net::SocketAddr,
    tls_server_cert: Option<String>,
    tls_server_key: Option<String>,
    tls_client_ca_cert: Option<String>,
    balancer: Arc<RwLock<Balancer>>,
) -> std::result::Result<(), Box<dyn std::error::Error>> {
    let server = MTransactionServer { balancer };

    let tls = get_tls_config(tls_server_cert, tls_server_key, tls_client_ca_cert).await?;
    let mut server_builder = match tls {
        Some(tls) => Server::builder().tls_config(tls)?,
        _ => Server::builder(),
    };

    info!("Spawning the gRPC server.");
    Ok(server_builder
        .add_service(pb::m_transaction_server::MTransactionServer::new(server))
        .serve(grpc_addr)
        .await?)
}
