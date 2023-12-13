use crate::auth::{authenticate, load_public_key, Auth};
use crate::solana_service::SignatureWrapper;
use crate::{balancer::*, metrics};
use bincode::config::Options;
use jsonrpc_core::{BoxFuture, MetaIoHandler, Metadata, Result};
use jsonrpc_derive::rpc;
use jsonrpc_http_server::hyper::http::header::AUTHORIZATION;
use jsonrpc_http_server::*;
use log::{error, info};
use rand::Rng;
use serde::{Deserialize, Serialize};
use solana_sdk::{packet::PACKET_DATA_SIZE, transaction::VersionedTransaction};
use std::{fmt, sync::Arc};
use tokio::sync::{mpsc::UnboundedSender, RwLock};

#[derive(Clone, Debug, PartialEq)]
pub enum Mode {
    BLACKHOLE,
    FORWARD,
}

impl fmt::Display for Mode {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Mode::BLACKHOLE => write!(f, "BLACKHOLE"),
            Mode::FORWARD => write!(f, "FORWARD"),
        }
    }
}

#[derive(Clone)]
pub struct RpcMetadata {
    auth: std::result::Result<Auth, String>,
    balancer: Arc<RwLock<Balancer>>,
    tx_signatures: UnboundedSender<SignatureWrapper>,
    mode: Mode,
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
        metrics::SERVER_RPC_TX_ACCEPTED
            .with_label_values(&[&auth.to_string(), &meta.mode.to_string()])
            .inc();
        metrics::SERVER_RPC_TX_BYTES_IN.inc_by(data.len() as u64);

        let wire_transaction = base64::decode(&data).unwrap();
        let decoded: Result<VersionedTransaction> = bincode::options()
            .with_limit(PACKET_DATA_SIZE as u64)
            .with_fixint_encoding()
            .allow_trailing_bytes()
            .deserialize_from(&wire_transaction[..])
            .map_err(|err| {
                error!("Deserialize error: {}", err);
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

        if let Err(err) = meta.tx_signatures.send(SignatureWrapper {
            signature: signature.clone(),
            partner_name: auth.to_string(),
            mode: meta.mode.clone(),
        }) {
            error!("Failed to propagate signature to the watcher: {}", err);
        }

        if meta.mode == Mode::BLACKHOLE {
            info!("Transaction blackholed: {}", signature.to_string());
            return Box::pin(async move { Ok(signature.to_string()) });
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

fn select_mode(auth: Option<Auth>, partners: Vec<String>) -> Mode {
    let mut mode = Mode::FORWARD;

    if let Some(auth) = auth {
        // If this is from a partner randomly blackhole based on rng?
        if partners.contains(&auth.to_string()) {
            let mut rng = rand::thread_rng();
            if rng.gen::<bool>() {
                mode = Mode::BLACKHOLE;
            }
        }
    }
    mode
}

pub fn spawn_rpc_server(
    rpc_addr: std::net::SocketAddr,
    jwt_public_key_path: Option<String>,
    test_partners: Option<Vec<String>>,
    balancer: Arc<RwLock<Balancer>>,
    tx_signatures: UnboundedSender<SignatureWrapper>,
) -> Server {
    info!("Spawning RPC server.");

    let public_key = jwt_public_key_path.map(|jwt_public_key_path| {
        Arc::new(
            load_public_key(jwt_public_key_path)
                .expect("Failed to load public key used to verify JWTs"),
        )
    });
    let partners = test_partners.unwrap_or_default();

    ServerBuilder::with_meta_extractor(
        get_io_handler(),
        move |req: &hyper::Request<hyper::Body>| {
            // If we are using authentication, then check the auth header
            // Otherwise just pass along allow and forward transactions
            if let Some(public_key) = public_key.clone() {
                let auth_header = req
                    .headers()
                    .get(AUTHORIZATION)
                    .map(|header_value| header_value.to_str().unwrap().to_string());

                let auth =
                    authenticate((*public_key).clone(), auth_header).map_err(|err| err.to_string());
                RpcMetadata {
                    auth: auth.clone(),
                    balancer: balancer.clone(),
                    tx_signatures: tx_signatures.clone(),
                    mode: select_mode(auth.clone().ok(), partners.clone()),
                }
            } else {
                RpcMetadata {
                    auth: Ok(Auth::Allow(String::from("internal"))),
                    balancer: balancer.clone(),
                    tx_signatures: tx_signatures.clone(),
                    mode: Mode::FORWARD,
                }
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
