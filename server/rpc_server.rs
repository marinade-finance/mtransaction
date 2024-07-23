use crate::auth::{authenticate, load_public_key, Auth};
use crate::{balancer::*, metrics, time_ms};
use crate::solana_service::SignatureRecord;
use tracing::Instrument;
use bincode::config::Options;
use jsonrpc_core::{BoxFuture, MetaIoHandler, Metadata, Result};
use jsonrpc_derive::rpc;
use jsonrpc_http_server::hyper::http::header::AUTHORIZATION;
use jsonrpc_http_server::*;
use log::{error, info};
use rand::Rng;
use serde::{Deserialize, Serialize};
use solana_sdk::{packet::PACKET_DATA_SIZE, transaction::VersionedTransaction};
use std::{fmt, sync::{Arc, atomic::{AtomicUsize, Ordering}}};
use tokio::sync::RwLock;
use tracing::info_span;

#[derive(Clone, Debug, PartialEq, Serialize)]
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
pub struct RpcState {
    auth: std::result::Result<Auth, String>,
    balancer: Arc<RwLock<Balancer>>,
    req_id: usize,
    mode: Mode,
}
impl Metadata for RpcState {}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SendPriorityTransactionConfig {
    skip_preflight: Option<bool>,
}

pub type SendTransactionConfig = SendPriorityTransactionConfig;

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

    #[rpc(meta, name = "sendTransaction")]
    fn send_transaction(
        &self,
        meta: Self::Metadata,
        data: String,
        config: Option<SendTransactionConfig>,
    ) -> BoxFuture<Result<String>>;

    #[rpc(name = "getHealth")]
    fn get_health(&self) -> Result<()>;
}

#[derive(Default)]
pub struct RpcServer;
impl Rpc for RpcServer {
    type Metadata = RpcState;

    fn send_priority_transaction(
        &self,
        meta: Self::Metadata,
        data: String,
        config: Option<SendPriorityTransactionConfig>,
    ) -> BoxFuture<Result<String>> {
        let req_id = meta.req_id;
        let ctime = time_ms();
        let span = info_span!(
            "send_priority_transaction",
            req_id = req_id,
            ctime = ctime,
            partner_name = meta.auth.clone()
                .map(|x| x.to_string())
                .unwrap_or_else(|_| "UNAUTHORIZED".to_string()),
            mode = meta.mode.to_string(),
        );
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
                }.instrument(span));
            }
        };

        match config {
            Some(config) => match config.skip_preflight {
                Some(skip_preflight) => {
                    if !skip_preflight {
                        return Box::pin(async move {
                            Err(jsonrpc_core::error::Error {
                                code: jsonrpc_core::error::ErrorCode::InvalidParams,
                                message: "skipPreflight must be true".into(),
                                data: None,
                            })
                        }.instrument(span));
                    }

                    info!("Skipping preflight checks");
                }
                None => {
                    return Box::pin(async move {
                        Err(jsonrpc_core::error::Error {
                            code: jsonrpc_core::error::ErrorCode::InvalidParams,
                            message: "skipPreflight is mandatory".into(),
                            data: None,
                        })
                    }.instrument(span));
                }
            },
            None => {
                return Box::pin(async move {
                    Err(jsonrpc_core::error::Error {
                        code: jsonrpc_core::error::ErrorCode::InvalidParams,
                        message: "config options are mandatory".into(),
                        data: None,
                    })
                }.instrument(span));
            }
        }

        span.in_scope(|| info!("RPC method sendPriorityTransaction called"));
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
                    }.instrument(span));
                }
            }
            Err(err) => return Box::pin(async move { Err(err) }.instrument(span)),
        };

        if meta.mode == Mode::BLACKHOLE {
            span.in_scope(|| info!("Transaction blackholed: {}", signature.to_string()));
            return Box::pin(async move { Ok(signature.to_string()) }.instrument(span));
        }

        let span_ = span.clone();
        let session = SignatureRecord {
            created_at: tokio::time::Instant::now(),
            span: span.clone(),
            req_id,
            ctime,
            rtime: time_ms(),
            signature: signature.clone(),
            partner_name: auth.to_string(),
            mode: meta.mode.clone(),
            consumers: vec![],
            tpu_ips: Default::default(),
        };
        Box::pin(async move {
            match meta
                .balancer
                .read()
                .await
                .publish(span_, session, signature.to_string(), data)
                .await
            {
                Ok(_) => Ok(signature.to_string()),
                Err(_) => Err(jsonrpc_core::error::Error {
                    code: jsonrpc_core::error::ErrorCode::InternalError,
                    message: "Failed to forward the transaction".into(),
                    data: None,
                }),
            }
        }.instrument(span.clone()))
    }

    fn send_transaction(
        &self,
        meta: Self::Metadata,
        data: String,
        config: Option<SendTransactionConfig>,
    ) -> BoxFuture<Result<String>> {
        self.send_priority_transaction(meta, data, config)
    }

    fn get_health(&self) -> Result<()> {
        // TODO: check total connected stake?
        // Would need to make the balancer an Arc?
        // let total_connected_stake = self.balancer.read().await.total_connected_stake;
        let tx_consumers = metrics::SERVER_TOTAL_CONNECTED_TX_CONSUMERS.get();

        if tx_consumers == 0 {
            Err(jsonrpc_core::error::Error {
                code: jsonrpc_core::error::ErrorCode::ServerError(503),
                message: "No connected tx consumers".into(),
                data: None,
            })
        } else {
            Ok(())
        }
    }
}

pub fn get_io_handler() -> MetaIoHandler<RpcState> {
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
) -> Server {
    info!("Spawning RPC server.");

    let public_key = jwt_public_key_path.map(|jwt_public_key_path| {
        Arc::new(
            load_public_key(jwt_public_key_path)
                .expect("Failed to load public key used to verify JWTs"),
        )
    });
    let partners = test_partners.unwrap_or_default();
    let req_id_gen = AtomicUsize::default();

    ServerBuilder::with_meta_extractor(
        get_io_handler(),
        move |req: &hyper::Request<hyper::Body>| {
            // If we are using authentication, then check the auth header
            // Otherwise just pass along allow and forward transactions
            let auth_header = req
                .headers()
                .get(AUTHORIZATION)
                .map(|header_value| header_value.to_str().unwrap().to_string());

            let (auth, mode) = if let Some(public_key) = public_key.clone() {
                let auth =
                    authenticate((*public_key).clone(), auth_header).map_err(|err| err.to_string());

                (
                    auth.clone(),
                    select_mode(auth.clone().ok(), partners.clone()),
                )
            } else {
                // In this mode, we trust whatever the end user puts in the Authorization header
                // This assumes that the end user is well known
                let from = auth_header.unwrap_or_else(|| String::from("unknown"));

                (Ok(Auth::Allow(from)), Mode::FORWARD)
            };

            RpcState {
                auth,
                balancer: balancer.clone(),
                req_id: req_id_gen.fetch_add(1, Ordering::Relaxed),
                mode,
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
