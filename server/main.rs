pub mod pb {
    tonic::include_proto!("validator");
}
pub mod rpc_server;

use crate::rpc_server::*;
use env_logger::Env;
use futures::Stream;
use jsonrpc_http_server::*;
use log::{debug, error, info};
use rand::Rng;
use std::{error::Error, io::ErrorKind, net::ToSocketAddrs, pin::Pin, time::Duration};
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

#[tokio::main]
async fn main() -> std::result::Result<(), Box<dyn std::error::Error>> {
    let grpc_addr = "0.0.0.0:50051".parse().unwrap();
    let rpc_addr = "0.0.0.0:3000".parse().unwrap();
    env_logger::Builder::from_env(Env::default().default_filter_or("info")).init();

    info!("Spawning RPC server.");
    let rpc_server = ServerBuilder::new(get_io_handler())
        .start_http(&rpc_addr)
        .expect("Unable to start TCP RPC server");

    info!("Loading server certificate.");
    let cert = tokio::fs::read("/var/local/backend.cert").await?;
    let key = tokio::fs::read("/var/local/backend.key").await?;
    let server_identity = Identity::from_pem(cert, key);
    //
    // let client_ca_cert = tokio::fs::read("./cert/client-ca-cert.pem").await?;
    // let client_ca_cert = Certificate::from_pem(client_ca_cert);
    //
    let tls = ServerTlsConfig::new().identity(server_identity);
    //     .client_ca_root(client_ca_cert);

    info!("Spawning the gRPC server.");
    let server = MTransactionServer {};
    Server::builder()
        .tls_config(tls)?
        .add_service(pb::m_transaction_server::MTransactionServer::new(server))
        .serve(grpc_addr)
        .await?;

    info!("Exiting.");

    Ok(())
}
