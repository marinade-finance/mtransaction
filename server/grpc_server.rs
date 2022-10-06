pub mod pb {
    tonic::include_proto!("validator");
}
use crate::balancer::*;
use futures::Stream;
use jsonrpc_http_server::*;
use log::{error, info, warn};
use rand::distributions::{Alphanumeric, DistString};
use std::pin::Pin;
use std::sync::Arc;
use tokio::sync::RwLock;
use tokio_stream::{wrappers::ReceiverStream, StreamExt};
use tonic::transport::{Certificate, Identity, Server, ServerTlsConfig};
use tonic::{Request, Response, Status, Streaming};
use x509_parser::prelude::*;

use pb::{RequestMessageEnvelope, ResponseMessageEnevelope};

type ResponseMessageEnevelopeStream =
    Pin<Box<dyn Stream<Item = std::result::Result<ResponseMessageEnevelope, Status>> + Send>>;

pub struct MTransactionServer {
    balancer: Arc<RwLock<Balancer>>,
}

pub fn build_tx_message_envelope(data: String) -> ResponseMessageEnevelope {
    ResponseMessageEnevelope {
        tx: Some(pb::Tx { data }),
        ..Default::default()
    }
}

pub fn build_ping_message_envelope(id: String) -> ResponseMessageEnevelope {
    ResponseMessageEnevelope {
        ping: Some(pb::Ping { id }),
        ..Default::default()
    }
}

fn get_identity_from_req(req: &Request<Streaming<RequestMessageEnvelope>>) -> Option<String> {
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
    type TxStreamStream = ResponseMessageEnevelopeStream;
    async fn tx_stream(
        &self,
        req: Request<Streaming<RequestMessageEnvelope>>,
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
        let token = Alphanumeric.sample_string(&mut rand::thread_rng(), 16);
        info!(
            "New client connected: {} {:?} {}.",
            &identity,
            req.remote_addr().unwrap(),
            &token,
        );

        let mut balancer = self.balancer.write().await;
        let (tx, rx) = balancer.subscribe(identity.clone(), token.clone());

        let mut input_stream = req.into_inner();
        let output_stream = ReceiverStream::new(rx);

        {
            let identity = identity.clone();
            tokio::spawn(async move {
                while let Some(request_message_envelope) = input_stream.next().await {
                    match request_message_envelope {
                        Ok(request_message_envelope) => info!(
                            "Received message from client {}: {:?}",
                            &identity, request_message_envelope
                        ),
                        Err(err) => error!(
                            "Error receiving message from the client {}: {}",
                            &identity, err
                        ),
                    }
                }
                println!("\tstream ended");
            });
        }

        {
            let tx = tx.clone();
            let identity = identity.clone();
            tokio::spawn(async move {
                let mut id = 0;
                loop {
                    tokio::time::sleep(tokio::time::Duration::from_secs(10)).await;
                    id += 1;
                    match tx
                        .send(Result::<_, Status>::Ok(build_ping_message_envelope(
                            id.to_string(),
                        )))
                        .await
                    {
                        Ok(_) => info!("Ping has been sent to {}: {}", &identity, id),
                        Err(err) => {
                            error!("Error sending ping to the client {}: {}", &identity, err)
                        }
                    }
                }
            });
        }

        {
            let balancer = self.balancer.clone();
            tokio::spawn(async move {
                tx.closed().await;
                info!(
                    "The connection to the client has been closed {} ({})",
                    &identity, &token
                );
                balancer.write().await.unsubscribe(&identity, &token);
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
