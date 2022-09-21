pub mod pb {
    tonic::include_proto!("validator");
}
pub mod balancer;
pub mod grpc_server;
pub mod rpc_server;

use crate::balancer::*;
use crate::grpc_server::*;
use crate::rpc_server::*;
use env_logger::Env;
use log::info;
use std::sync::Arc;
use structopt::StructOpt;
use tokio::sync::RwLock;

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

    let balancer = Arc::new(RwLock::new(Balancer::new()));

    {
        let balacer_cp = balancer.clone();
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
            }
        });
    }

    let _rpc_server = spawn_rpc_server(params.rpc_addr.parse().unwrap(), balancer.clone());

    spawn_grpc_server(
        params.grpc_addr.parse().unwrap(),
        params.tls_grpc_server_cert,
        params.tls_grpc_server_key,
        params.tls_grpc_client_ca_cert,
        balancer.clone(),
    )
    .await?;

    info!("Service stopped.");
    Ok(())
}
