pub mod forwarder;
pub mod grpc_client;
pub mod metrics;
pub mod quic_forwarder;
pub mod rpc_forwarder;
pub mod watcher;

use crate::forwarder::spawn_forwarder;
use crate::grpc_client::spawn_grpc_client;
use crate::metrics::Metrics;
use crate::watcher::{check_validator, spawn_watcher};
use env_logger::Env;
use log::{error, info};
use solana_sdk::signature::read_keypair_file;
use std::sync::Arc;
use structopt::StructOpt;

pub const VERSION: &str = "rust-0.0.0-alpha";

#[derive(Debug, StructOpt)]
struct Params {
    #[structopt(long = "tls-grpc-ca-cert")]
    tls_grpc_ca_cert: Option<String>,

    #[structopt(long = "tls-grpc-client-key")]
    tls_grpc_client_key: Option<String>,

    #[structopt(long = "tls-grpc-client-cert")]
    tls_grpc_client_cert: Option<String>,

    #[structopt(long = "grpc-url", default_value = "http://127.0.0.1:50051")]
    grpc_url: String,

    #[structopt(long = "identity")]
    identity: Option<String>,

    #[structopt(long = "tpu-addr")]
    tpu_addr: Option<String>,

    #[structopt(long = "rpc-url")]
    rpc_url: Option<String>,

    #[structopt(long = "monitor-validator")]
    monitor_validator: bool,

    #[structopt(long = "blackhole")]
    blackhole: bool,

    #[structopt(long = "throttle-parallel", default_value = "100")]
    throttle_parallel: usize,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    env_logger::Builder::from_env(Env::default().default_filter_or("info")).init();
    let params = Params::from_args();

    let (identity, tpu_addr) = match (&params.identity, &params.tpu_addr) {
        (Some(identity), Some(tpu_addr)) => (
            Some(read_keypair_file(identity).unwrap()),
            Some(tpu_addr.parse().unwrap()),
        ),
        _ => (None, None),
    };

    let metrics = Arc::new(Metrics::default());

    let tx_transactions = spawn_forwarder(
        identity,
        tpu_addr,
        params.rpc_url,
        metrics.clone(),
        params.blackhole,
        params.throttle_parallel,
    );

    if params.monitor_validator {
        match check_validator().await {
            Ok(watcher::ValidatorStatus::Online) => {
                let exit_signal = spawn_watcher();
                spawn_grpc_client(
                    params.grpc_url.parse().unwrap(),
                    params.tls_grpc_ca_cert,
                    params.tls_grpc_client_key,
                    params.tls_grpc_client_cert,
                    tx_transactions,
                    metrics.clone(),
                    Some(exit_signal),
                )
                .await?;
            }
            Ok(watcher::ValidatorStatus::Offline) | Err(_) => {
                error!("Validator status: Offline");
            }
        }
    } else {
        spawn_grpc_client(
            params.grpc_url.parse().unwrap(),
            params.tls_grpc_ca_cert,
            params.tls_grpc_client_key,
            params.tls_grpc_client_cert,
            tx_transactions,
            metrics.clone(),
            None,
        )
        .await?;
    }

    info!("Service stopped.");
    Ok(())
}
