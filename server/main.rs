pub mod pb {
    tonic::include_proto!("validator");
}

use env_logger::Env;
use futures::Stream;
use log::{debug, error, info};
use std::{error::Error, io::ErrorKind, net::ToSocketAddrs, pin::Pin, time::Duration};
use tokio::sync::mpsc;
use tokio_stream::{wrappers::ReceiverStream, StreamExt};
use tonic::{transport::Server, Request, Response, Status, Streaming};

use pb::{EchoRequest, EchoResponse};

type EchoResult<T> = Result<Response<T>, Status>;
type EchoResponseStream = Pin<Box<dyn Stream<Item = Result<EchoResponse, Status>> + Send>>;

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
        Ok(Response::new(EchoResponse { message }))
    }

    type EchoStreamStream = EchoResponseStream;
    async fn echo_stream(&self, req: Request<EchoRequest>) -> EchoResult<Self::EchoStreamStream> {
        info!("Client connected: {:?}.", req.remote_addr());

        let message = req.into_inner().message;
        info!("Echo stream called with {}", &message);
        let repeat = std::iter::repeat(EchoResponse { message });
        let mut stream = Box::pin(tokio_stream::iter(repeat).throttle(Duration::from_millis(200)));

        let (tx, rx) = mpsc::channel(128);
        tokio::spawn(async move {
            while let Some(item) = stream.next().await {
                match tx.send(Result::<_, Status>::Ok(item)).await {
                    Ok(_) => {
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
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    env_logger::Builder::from_env(Env::default().default_filter_or("info")).init();

    info!("Launching the server.");
    let server = MTransactionServer {};
    Server::builder()
        .add_service(pb::m_transaction_server::MTransactionServer::new(server))
        .serve("0.0.0.0:50051".to_socket_addrs().unwrap().next().unwrap())
        .await
        .unwrap();

    info!("Exiting.");

    Ok(())
}
