pub mod pb {
    tonic::include_proto!("validator");
}
use crate::balancer::*;
use crate::metrics;
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

use pb::{RequestMessageEnvelope, ResponseMessageEnvelope};

type ResponseMessageEnvelopeStream =
    Pin<Box<dyn Stream<Item = std::result::Result<ResponseMessageEnvelope, Status>> + Send>>;

pub struct MTransactionServer {
    balancer: Arc<RwLock<Balancer>>,
}

#[derive(Clone)]
struct Ping {
    id: u64,
    at: tokio::time::Instant,
}

impl Ping {
    fn new() -> Self {
        Self {
            id: 0,
            at: tokio::time::Instant::now(),
        }
    }
}

impl Iterator for Ping {
    type Item = Ping;

    fn next(&mut self) -> Option<Ping> {
        self.id += 1;
        Some(Ping {
            id: self.id,
            at: tokio::time::Instant::now(),
        })
    }
}

pub struct RttValue {
    pub rtt: u64,
    pub n: u64,
}

pub fn build_tx_message_envelope(
    signature: String,
    data: String,
    tpu: Vec<String>,
) -> ResponseMessageEnvelope {
    ResponseMessageEnvelope {
        transaction: Some(pb::Transaction {
            signature,
            data,
            tpu,
        }),
        ..Default::default()
    }
}

pub fn build_ping_message_envelope(id: String) -> ResponseMessageEnvelope {
    ResponseMessageEnvelope {
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

fn handle_client_metrics(identity: &String, token: &String, metrics: pb::Metrics) {
    info!(
        "Accepted metrics from {} ({}): {:?}",
        &identity, &token, metrics
    );
    metrics::CLIENT_TX_RECEIVED
        .with_label_values(&[&identity])
        .set(metrics.tx_received as i64);
    metrics::CLIENT_TX_FORWARD_SUCCEEDED
        .with_label_values(&[&identity])
        .set(metrics.tx_forward_succeeded as i64);
    metrics::CLIENT_TX_FORWARD_FAILED
        .with_label_values(&[&identity])
        .set(metrics.tx_forward_failed as i64);
    metrics::CLIENT_QUIC_FORWARDER_PERMITS_USED_MAX
        .with_label_values(&[&identity])
        .set(metrics.quic_forwarder_permits_used_max as i64);
    metrics::CLIENT_MEMORY_PHYSICAL
        .with_label_values(&[&identity])
        .set(metrics.memory_physical as i64);
}

fn handle_client_pong(identity: &String, pong: pb::Pong, last_ping: &Ping) {
    if last_ping.id.to_string() == pong.id {
        metrics::CLIENT_PING_RTT
            .with_label_values(&[identity])
            .observe(last_ping.at.elapsed().as_micros() as f64 / 1.0e6);
    }
}

async fn handle_client_request(
    request_message_envelope: Result<RequestMessageEnvelope, Status>,
    identity: &String,
    token: &String,
    last_ping: &Ping,
    balancer: &Arc<RwLock<Balancer>>,
) {
    match request_message_envelope {
        Ok(request_message_envelope) => {
            if let Some(metrics) = request_message_envelope.metrics {
                handle_client_metrics(&identity, &token, metrics);
            }
            if let Some(rtt) = request_message_envelope.rtt {
                let mut balancer = balancer.write().await;
                balancer.update_rtt(identity, rtt);
            }
            if let Some(pong) = request_message_envelope.pong {
                handle_client_pong(&identity, pong, last_ping);
            }
        }
        Err(err) => error!(
            "Error receiving message from the client {} ({}): {}",
            &identity, &token, err
        ),
    };
}

#[tonic::async_trait]
impl pb::m_transaction_server::MTransaction for MTransactionServer {
    type TxStreamStream = ResponseMessageEnvelopeStream;
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
            "New client connected: {} ({}) {:?}.",
            &identity,
            &token,
            req.remote_addr().unwrap(),
        );

        let mut ping_hint_stream = Box::pin(
            tokio_stream::iter(Ping::new()).throttle(tokio::time::Duration::from_secs(10)),
        );

        let mut balancer = self.balancer.write().await;
        let (tx, rx, mut rx_unsubscribe) = balancer.subscribe(identity.clone(), token.clone());

        let mut input_stream = req.into_inner();
        let output_stream = ReceiverStream::new(rx);

        {
            let identity = identity.clone();
            let token = token.clone();
            let balancer = self.balancer.clone();
            let mut last_ping = Ping::new();
            tokio::spawn(async move {
                loop {
                    tokio::select! {
                        unsubscribed = (&mut rx_unsubscribe) => {
                            info!("Client unsubscribed {} ({}): {:?}", &identity, &token, unsubscribed);
                            break;
                        }

                        request_message_envelope = input_stream.next() => {
                            if let Some(request_message_envelope) = request_message_envelope{
                                handle_client_request(request_message_envelope, &identity, &token, &last_ping, &balancer).await;
                            } else {
                                info!("Stream from client {} ({}) has ended.", &identity, &token);
                                break
                            }
                        }

                        Some(ping) = ping_hint_stream.next() => {
                            last_ping = ping.clone();
                            match tx.send(std::result::Result::<_, Status>::Ok(build_ping_message_envelope(ping.id.to_string()))).await {
                                Ok(_) => info!("Ping has been sent to {} ({})", &identity, &token),
                                Err(err) => {
                                    error!(
                                        "Error sending ping to the client {} ({}): {}",
                                        &identity, &token, err
                                    );
                                    break;
                                }
                            }
                        }

                        _ = tx.closed() => {
                            info!(
                                "The connection to the client has been closed {} ({})",
                                &identity, &token
                            );
                            break
                        }
                    };
                }
                info!("Cleaning resources after client {} ({})", &identity, &token);
                balancer.write().await.unsubscribe(&identity, &token);
                metrics::reset_client_metrics(&identity);
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
    tls_grpc_ca_cert: Option<String>,
) -> std::result::Result<Option<ServerTlsConfig>, Box<dyn std::error::Error>> {
    let tls = if let (Some(cert), Some(key)) = (tls_server_cert, tls_server_key) {
        let tls = ServerTlsConfig::new();

        info!("Loading server TLS from from {} and {}", cert, key);
        let cert = tokio::fs::read(cert).await?;
        let key = tokio::fs::read(key).await?;
        let tls = tls.identity(Identity::from_pem(cert, key));

        if let Some(cert) = tls_grpc_ca_cert {
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
    tls_grpc_ca_cert: Option<String>,
    balancer: Arc<RwLock<Balancer>>,
) -> std::result::Result<(), Box<dyn std::error::Error>> {
    let server = MTransactionServer { balancer };

    let tls = get_tls_config(tls_server_cert, tls_server_key, tls_grpc_ca_cert).await?;
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
