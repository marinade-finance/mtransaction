pub mod pb {
    tonic::include_proto!("validator");
}
use futures::Stream;
use jsonrpc_http_server::*;
use log::{debug, error, info, warn};
use rand::Rng;
use std::{error::Error, io::ErrorKind, net::ToSocketAddrs, pin::Pin, time::Duration};
use structopt::StructOpt;
use tokio::sync::mpsc;
use tokio_stream::{wrappers::ReceiverStream, StreamExt};
use tonic::transport::{Certificate, Identity, Server, ServerTlsConfig};
use tonic::{Request, Response, Status, Streaming};

use pb::{EchoRequest, EchoResponse};

type EchoResult<T> = std::result::Result<Response<T>, Status>;
type EchoResponseStream =
    Pin<Box<dyn Stream<Item = std::result::Result<EchoResponse, Status>> + Send>>;

fn match_for_io_error(err_status: &Status) -> Option<&std::io::Error> {
    let mut err: &(dyn Error + 'static) = err_status;

    loop {
        if let Some(io_err) = err.downcast_ref::<std::io::Error>() {
            return Some(io_err);
        }

        // h2::Error do not expose std::io::Error with `source()`
        // https://github.com/hyperium/h2/pull/462
        if let Some(h2_err) = err.downcast_ref::<h2::Error>() {
            if let Some(io_err) = h2_err.get_io() {
                return Some(io_err);
            }
        }

        err = match err.source() {
            Some(err) => err,
            None => return None,
        };
    }
}

#[derive(Debug)]
pub struct MTransactionServer {}

#[tonic::async_trait]
impl pb::m_transaction_server::MTransaction for MTransactionServer {
    async fn echo(&self, req: Request<EchoRequest>) -> EchoResult<EchoResponse> {
        let message = req.into_inner().message;
        info!("Echo called with {}", &message);
        Ok(Response::new(EchoResponse {
            message: format!("{} is best", message),
        }))
    }

    type EchoStreamStream = EchoResponseStream;
    async fn echo_stream(&self, req: Request<EchoRequest>) -> EchoResult<Self::EchoStreamStream> {
        info!("Client connected: {:?}.", req.remote_addr());
        let mut rng = rand::thread_rng();
        let count = rng.gen_range(3..10);

        let message = req.into_inner().message;
        info!("Echo stream called with {}", &message);
        let repeat = std::iter::repeat(EchoResponse {
            message: format!("{} is best", message),
        })
        .take(count);
        let mut stream = Box::pin(tokio_stream::iter(repeat).throttle(Duration::from_millis(200)));

        let (tx, rx) = mpsc::channel(128);
        tokio::spawn(async move {
            while let Some(item) = stream.next().await {
                match tx.send(std::result::Result::<_, Status>::Ok(item)).await {
                    Ok(_) => {
                        info!("Message sent");
                        // item (server response) was queued to be send to client
                    }
                    Err(_item) => {
                        // output_stream was build from rx and both are dropped
                        break;
                    }
                }
            }
            info!("Client disconnected");
        });

        let output_stream = ReceiverStream::new(rx);
        Ok(Response::new(
            Box::pin(output_stream) as Self::EchoStreamStream
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
) -> std::result::Result<(), Box<dyn std::error::Error>> {
    let server = MTransactionServer {};

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
