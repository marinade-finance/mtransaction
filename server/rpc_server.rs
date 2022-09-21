use crate::balancer::*;
use jsonrpc_core::futures::future;
use jsonrpc_core::{BoxFuture, MetaIoHandler, Metadata, Result};
use jsonrpc_derive::rpc;
use jsonrpc_http_server::*;
use log::{debug, error, info};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::sync::Arc;
use tokio::sync::RwLock;

#[derive(Clone)]
pub struct RpcMetadata {
    balancer: Arc<RwLock<Balancer>>,
}
impl Metadata for RpcMetadata {}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SendPriorityTransactionConfig {}

#[rpc]
pub trait Rpc {
    type Metadata;

    #[rpc(meta, name = "sendPriorityTransaction")]
    fn send_priority_transaction(
        &self,
        meta: Self::Metadata,
        data: String,
        config: Option<SendPriorityTransactionConfig>,
    ) -> BoxFuture<Result<()>>;
}

#[derive(Default)]
pub struct RpcServer;
impl Rpc for RpcServer {
    type Metadata = RpcMetadata;

    fn send_priority_transaction(
        &self,
        meta: Self::Metadata,
        data: String,
        config: Option<SendPriorityTransactionConfig>,
    ) -> BoxFuture<Result<()>> {
        info!("RPC method sendTransaction called.");
        Box::pin(async move {
            match meta.balancer.read().await.publish(data).await {
                Ok(_) => Ok(()),
                Err(err) => Err(jsonrpc_core::error::Error {
                    code: jsonrpc_core::error::ErrorCode::InternalError,
                    message: "Failed to forward the transaction".into(),
                    data: None,
                }),
            }
        })
    }
}

pub fn get_io_handler() -> MetaIoHandler<RpcMetadata> {
    let mut io = MetaIoHandler::default();
    io.extend_with(RpcServer::default().to_delegate());

    io
}

pub fn spawn_rpc_server(rpc_addr: std::net::SocketAddr, balancer: Arc<RwLock<Balancer>>) -> Server {
    info!("Spawning RPC server.");

    ServerBuilder::with_meta_extractor(
        get_io_handler(),
        move |_req: &hyper::Request<hyper::Body>| RpcMetadata {
            balancer: balancer.clone(),
        },
    )
    .cors(DomainsValidation::AllowOnly(vec![
        AccessControlAllowOrigin::Any,
    ]))
    .threads(4)
    .start_http(&rpc_addr)
    .expect("Unable to start TCP RPC server")
}
