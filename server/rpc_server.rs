use jsonrpc_core::futures::future;
use jsonrpc_core::{BoxFuture, IoHandler, Result};
use jsonrpc_derive::rpc;
use log::{debug, error, info};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct RpcVersionInfo {
    /// The current version of solana-core
    pub solana_core: String,
    /// first 4 bytes of the FeatureSet identifier
    pub feature_set: Option<u32>,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RpcSendTransactionConfig {
    #[serde(default)]
    pub skip_preflight: bool,
    pub preflight_commitment: Option<String>,
    pub encoding: Option<String>,
    pub max_retries: Option<usize>,
    pub min_context_slot: Option<u64>,
}

#[rpc(server)]
pub trait Rpc {
    #[rpc(name = "sendTransaction")]
    fn send_transaction(
        &self,
        data: String,
        config: Option<RpcSendTransactionConfig>,
    ) -> Result<String>;
    #[rpc(name = "getVersion")]
    fn get_version(&self) -> Result<RpcVersionInfo>;
}
#[derive(Default)]
pub struct RpcServer;

impl Rpc for RpcServer {
    fn send_transaction(
        &self,
        data: String,
        config: Option<RpcSendTransactionConfig>,
    ) -> Result<String> {
        info!("RPC method sendTransaction called.");
        Ok("this went well".to_string())
    }
    fn get_version(&self) -> Result<RpcVersionInfo> {
        info!("RPC method getVersion called.");
        Ok(RpcVersionInfo {
            solana_core: "1.10.38".to_string(),
            feature_set: None,
        })
    }
}

pub fn get_io_handler() -> IoHandler {
    let mut io = IoHandler::new();
    io.extend_with(RpcServer::default().to_delegate());

    io
}
