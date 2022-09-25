pub mod pb {
    tonic::include_proto!("validator");
}
use crate::balancer::*;
use futures::Stream;
use jsonrpc_http_server::*;
use log::{error, info, warn};
use std::pin::Pin;
use std::sync::Arc;
use tokio::sync::RwLock;
use tokio_stream::wrappers::ReceiverStream;
use tonic::transport::{Certificate, Identity, Server, ServerTlsConfig};
use tonic::{Request, Response, Status};
use x509_parser::prelude::*;

use pb::{TxMessage, TxMessageRequest};

type TxMessageStream = Pin<Box<dyn Stream<Item = std::result::Result<TxMessage, Status>> + Send>>;

pub struct MTransactionServer {
    balancer: Arc<RwLock<Balancer>>,
}

fn get_identity_from_req(req: &Request<TxMessageRequest>) -> Option<String> {
    if let Some(certs) = req.peer_certs() {
        for cert in certs.as_ref() {
            if let Ok((_, cert)) = X509Certificate::from_der(cert.get_ref()) {
                for cn in cert.subject().iter_common_name() {
                    if let Ok(cn) = cn.attr_value().as_str() {
                        return Some(cn.into());
                    }
                }
            }
        }
    }

    return None;
}

#[tonic::async_trait]
impl pb::m_transaction_server::MTransaction for MTransactionServer {
    type TxStreamStream = TxMessageStream;
    async fn tx_stream(
        &self,
        req: Request<TxMessageRequest>,
    ) -> std::result::Result<Response<Self::TxStreamStream>, Status> {
        let identity = match get_identity_from_req(&req) {
            Some(identity) => identity,
            _ => {
                error!(
                    "Unable to idetify the client by TLS certificate ({:?}).",
                    req.remote_addr()
                );
                return Err(Status::unauthenticated("Not authenticated!".to_string()));
            }
        };
        info!(
            "New client connected: {:?} {:?}.",
            &identity,
            req.remote_addr()
        );

        let mut balancer = self.balancer.write().await;
        let (tx, rx) = if let Some((tx, rx)) = balancer.subscribe(identity.clone()) {
            (tx, rx)
        } else {
            return Err(Status::resource_exhausted(format!(
                "Client with the same identity {} is already connected!",
                &identity
            )));
        };

        {
            let balancer = self.balancer.clone();
            tokio::spawn(async move {
                tx.closed().await;
                info!("The client has disconnected {:?}", &identity);
                balancer.write().await.unsubscribe(&identity);
            });
        }

        let output_stream = ReceiverStream::new(rx);
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
