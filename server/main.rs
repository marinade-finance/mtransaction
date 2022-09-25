pub mod pb {
    tonic::include_proto!("validator");
}
pub mod balancer;
pub mod grpc_server;
pub mod rpc_server;
pub mod solana_service;

use crate::balancer::*;
use crate::grpc_server::*;
use crate::rpc_server::*;
use crate::solana_service::*;
use env_logger::Env;
use log::{error, info};
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

    #[structopt(
        long = "rpc-url",
        default_value = "https://api.mainnet-beta.solana.com"
    )]
    rpc_url: String,

    #[structopt(long = "rpc-commitment", default_value = "finalized")]
    rpc_commitment: String,
}

#[tokio::main]
async fn main() -> std::result::Result<(), Box<dyn std::error::Error>> {
    env_logger::Builder::from_env(Env::default().default_filter_or("info")).init();
    let params = Params::from_args();

    let solana_client = Arc::new(solana_client(params.rpc_url, params.rpc_commitment));
    let balancer = Arc::new(RwLock::new(Balancer::new()));

    {
        let balancer = balancer.clone();
        let solana_client = solana_client.clone();
        tokio::spawn(async move {
            loop {
                match get_activated_stake(solana_client.as_ref()) {
                    Ok(stake_weights) => {
                        balancer.write().await.update_stake_weights(stake_weights);
                    }
                    Err(err) => error!("Failed to update stake weights: {}", err),
                };
                tokio::time::sleep(tokio::time::Duration::from_secs(60)).await;
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
