use jsonrpc_core::futures::future;
use jsonrpc_core::{BoxFuture, IoHandler, Result};
use jsonrpc_derive::rpc;
use log::{debug, error, info};

#[rpc(server)]
pub trait Rpc {
    #[rpc(name = "sendTransaction")]
    fn send_transaction(&self) -> Result<String>;
}
#[derive(Default)]
pub struct RpcServer;

impl Rpc for RpcServer {
    fn send_transaction(&self) -> Result<String> {
        info!("Forwarding tx.");
        Ok("this went well".to_string())
    }
}

pub fn get_io_handler() -> IoHandler {
    let mut io = IoHandler::new();
    io.extend_with(RpcServer::default().to_delegate());

    io
}
