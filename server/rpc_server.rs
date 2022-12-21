use crate::auth::{authenticate, load_public_key, Auth};
use crate::balancer::*;
use crate::metrics::Metric;
use bincode::config::Options;
use jsonrpc_core::{BoxFuture, MetaIoHandler, Metadata, Result};
use jsonrpc_derive::rpc;
use jsonrpc_http_server::hyper::http::header::AUTHORIZATION;
use jsonrpc_http_server::*;
use log::{error, info};
use serde::{Deserialize, Serialize};
use solana_sdk::{
    packet::PACKET_DATA_SIZE, signature::Signature, transaction::VersionedTransaction,
};
use std::sync::Arc;
use tokio::sync::{mpsc::UnboundedSender, RwLock};

#[derive(Clone)]
pub struct RpcMetadata {
    auth: std::result::Result<Auth, String>,
    balancer: Arc<RwLock<Balancer>>,
    tx_metrics: UnboundedSender<Vec<Metric>>,
    tx_signatures: UnboundedSender<Signature>,
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
    ) -> BoxFuture<Result<String>>;

    #[rpc(name = "getHealth")]
    fn get_health(&self) -> Result<()>;
}

#[derive(Default)]
pub struct RpcServer;
impl Rpc for RpcServer {
    type Metadata = RpcMetadata;

    fn send_priority_transaction(
        &self,
        meta: Self::Metadata,
        data: String,
        _config: Option<SendPriorityTransactionConfig>,
    ) -> BoxFuture<Result<String>> {
        let auth = match meta.auth {
            Ok(auth) => auth,
            Err(err) => {
                error!("Authentication error: {}", err);
                return Box::pin(async move {
                    Err(jsonrpc_core::error::Error {
                        code: jsonrpc_core::error::ErrorCode::ServerError(403),
                        message: "Failed to authenticate".into(),
                        data: None,
                    })
                });
            }
        };

        info!("RPC method sendPriorityTransaction called: {:?}", auth);
        if let Err(err) = meta.tx_metrics.send(vec![
            Metric::ServerRpcTxAccepted,
            Metric::ServerRpcTxBytesIn {
                bytes: data.len() as u64,
            },
        ]) {
            error!("Failed to update RPC metrics: {}", err);
        }

        let wire_transaction = base64::decode(&data).unwrap();
        let decoded: Result<VersionedTransaction> = bincode::options()
            .with_limit(PACKET_DATA_SIZE as u64)
            .with_fixint_encoding()
            .allow_trailing_bytes()
            .deserialize_from(&wire_transaction[..])
            .map_err(|err| {
                info!("Deserialize error: {}", err);
                jsonrpc_core::error::Error::invalid_params(&err.to_string())
            });

        let signature = match decoded {
            Ok(decoded) => {
                if let Some(signature) = decoded.signatures.get(0) {
                    signature.clone()
                } else {
                    return Box::pin(async move {
                        Err(jsonrpc_core::error::Error {
                            code: jsonrpc_core::error::ErrorCode::InternalError,
                            message: "Failed to get the transaction's signature".into(),
                            data: None,
                        })
                    });
                }
            }
            Err(err) => return Box::pin(async move { Err(err) }),
        };

        if let Err(err) = meta.tx_signatures.send(signature.clone()) {
            error!("Failed to propagate signature to the watcher: {}", err);
        }

        Box::pin(async move {
            match meta
                .balancer
                .read()
                .await
                .publish(signature.to_string(), data)
                .await
            {
                Ok(_) => Ok(signature.to_string()),
                Err(_) => Err(jsonrpc_core::error::Error {
                    code: jsonrpc_core::error::ErrorCode::InternalError,
                    message: "Failed to forward the transaction".into(),
                    data: None,
                }),
            }
        })
    }

    fn get_health(&self) -> Result<()> {
        Ok(())
    }
}

pub fn get_io_handler() -> MetaIoHandler<RpcMetadata> {
    let mut io = MetaIoHandler::default();
    io.extend_with(RpcServer::default().to_delegate());

    io
}

pub fn spawn_rpc_server(
    rpc_addr: std::net::SocketAddr,
    jwt_public_key_path: String,
    balancer: Arc<RwLock<Balancer>>,
    tx_metrics: UnboundedSender<Vec<Metric>>,
    tx_signatures: UnboundedSender<Signature>,
) -> Server {
    info!("Spawning RPC server.");
    let public_key = Arc::new(
        load_public_key(jwt_public_key_path)
            .expect("Failed to load public key used to verify JWTs"),
    );

    ServerBuilder::with_meta_extractor(
        get_io_handler(),
        move |req: &hyper::Request<hyper::Body>| {
            let auth_header = req
                .headers()
                .get(AUTHORIZATION)
                .map(|header_value| header_value.to_str().unwrap().to_string());

            RpcMetadata {
                auth: authenticate((*public_key).clone(), auth_header)
                    .map_err(|err| err.to_string()),
                balancer: balancer.clone(),
                tx_metrics: tx_metrics.clone(),
                tx_signatures: tx_signatures.clone(),
            }
        },
    )
    .cors(DomainsValidation::AllowOnly(vec![
        AccessControlAllowOrigin::Any,
    ]))
    .health_api(("/health", "getHealth"))
    .threads(64)
    .start_http(&rpc_addr)
    .expect("Unable to start TCP RPC server")
}
