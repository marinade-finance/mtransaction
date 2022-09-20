pub mod pb {
    tonic::include_proto!("validator");
}
pub mod grpc_server;
pub mod rpc_server;

use crate::grpc_server::*;
use crate::rpc_server::*;
use env_logger::Env;
use jsonrpc_http_server::*;
use log::info;
use structopt::StructOpt;

#[derive(Debug, StructOpt)]
struct Params {
    #[structopt(long = "tls-grpc-server-cert")]
    tls_grpc_server_cert: Option<String>,

    #[structopt(long = "tls-grpc-server-key")]
    tls_grpc_server_key: Option<String>,

    #[structopt(long = "tls-grpc-client-ca-cert")]
    tls_grpc_client_ca_cert: Option<String>,

    #[structopt(long = "grpc-addr", default_value = "0.0.0.0:50051")]
    grpc_addr: String,

    #[structopt(long = "rpc-addr", default_value = "0.0.0.0:3000")]
    rpc_addr: String,
}

#[tokio::main]
async fn main() -> std::result::Result<(), Box<dyn std::error::Error>> {
    env_logger::Builder::from_env(Env::default().default_filter_or("info")).init();
    let params = Params::from_args();

    let rpc_addr = params.rpc_addr.parse().unwrap();

    info!("Spawning RPC server.");
    let _rpc_server = ServerBuilder::new(get_io_handler())
        .start_http(&rpc_addr)
        .expect("Unable to start TCP RPC server");

    spawn_grpc_server(
        params.grpc_addr.parse().unwrap(),
        params.tls_grpc_server_cert,
        params.tls_grpc_server_key,
        params.tls_grpc_client_ca_cert,
    )
    .await?;

    info!("Service stopped.");
    Ok(())
}
