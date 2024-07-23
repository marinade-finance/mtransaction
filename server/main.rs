pub mod auth;
pub mod balancer;
pub mod grpc_server;
pub mod metrics;
pub mod rpc_server;
pub mod solana_service;

use crate::balancer::*;
use crate::grpc_server::*;
use crate::rpc_server::*;
use crate::solana_service::*;
use log::{error, info};
use solana_client::nonblocking::pubsub_client::PubsubClient;
use std::sync::Arc;
use std::time::SystemTime;
use std::{panic, process};
use structopt::StructOpt;
use tokio::sync::RwLock;
use tracing_log::LogTracer;

pub const N_COPIES: usize = 2;
pub const N_LEADERS: u64 = 7;
pub const N_CONSUMERS: usize = 3;
pub const LEADER_REFRESH_SECONDS: u64 = 3600;
pub const NODES_REFRESH_SECONDS: u64 = 60;
pub const GOSSIP_ENTRYPOINT: &str = "entrypoint3.mainnet-beta.solana.com:8001";

#[derive(Debug, StructOpt)]
struct Params {
    #[structopt(long = "tls-grpc-server-cert")]
    tls_grpc_server_cert: Option<String>,

    #[structopt(long = "tls-grpc-server-key")]
    tls_grpc_server_key: Option<String>,

    #[structopt(long = "tls-grpc-ca-cert")]
    tls_grpc_ca_cert: Option<String>,

    #[structopt(long = "grpc-addr", default_value = "0.0.0.0:50051")]
    grpc_addr: String,

    #[structopt(long = "metrics-addr", default_value = "0.0.0.0:9000")]
    metrics_addr: String,

    #[structopt(long = "rpc-addr", default_value = "0.0.0.0:3000")]
    rpc_addr: String,

    #[structopt(
        long = "rpc-url",
        default_value = "https://api.mainnet-beta.solana.com"
    )]
    rpc_url: String,

    #[structopt(
        long = "ws-rpc-url",
        default_value = "wss://api.mainnet-beta.solana.com"
    )]
    ws_rpc_url: String,

    #[structopt(long = "rpc-commitment", default_value = "finalized")]
    rpc_commitment: String,

    #[structopt(long = "test-partners")]
    test_partners: Option<Vec<String>>,

    #[structopt(long = "stake-override-identity")]
    stake_override_identity: Vec<String>,

    #[structopt(long = "stake-override-sol")]
    stake_override_sol: Vec<u64>,

    #[structopt(long = "jwt-public-key")]
    jwt_public_key: Option<String>,

    #[structopt(long = "debug")]
    debug: Option<bool>,
}

#[inline]
pub fn time_ms() -> u64 {
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64
}

#[macro_export]
macro_rules! json_str {
    (
        $val:expr $(,)?
    ) => {
        serde_json::to_string($val).unwrap_or_else(|_| "null".to_string())
    };
}

fn setup_panic_hook() {
    let hook = panic::take_hook();
    panic::set_hook(Box::new(move |info| {
        hook(info);
        error!("panic_hook forcing exit");
        process::exit(1);
    }));
}

fn setup_tracing(debug: bool) {
    LogTracer::init().expect("Setting up log compatibility failed");
    let subscriber = tracing_subscriber::fmt::Subscriber::builder()
        .with_target(false)
        .with_writer(std::io::stderr)
        .with_max_level(if debug {
            tracing::Level::DEBUG
        } else {
            tracing::Level::INFO
        })
        .compact()
        .finish();
    tracing::subscriber::set_global_default(subscriber).unwrap();
}

#[tokio::main]
async fn main() -> std::result::Result<(), Box<dyn std::error::Error>> {
    setup_panic_hook();

    let params = Params::from_args();

    setup_tracing(params.debug.unwrap_or(false));
 
    let client = Arc::new(solana_client(params.rpc_url, params.rpc_commitment));
    let tx_signatures = spawn_tx_signature_watcher(client.clone()).unwrap();
    let balancer = Arc::new(RwLock::new(Balancer::new(tx_signatures)));
    let pubsub_client = Arc::new(PubsubClient::new(&params.ws_rpc_url).await?);
    balancer_updater(balancer.clone(), client.clone(), pubsub_client.clone());
    metrics::spawn(params.metrics_addr.parse().unwrap());

    {
        let balancer = balancer.clone();
        let client = client.clone();
        let stake_override: std::collections::HashMap<_, _> = params
            .stake_override_identity
            .iter()
            .cloned()
            .zip(params.stake_override_sol.iter().cloned())
            .collect();
        tokio::spawn(async move {
            // todo move to balancer_updater
            let mut last_epoch = 0;
            info!("Stake overrides loaded: {:?}", &stake_override);
            loop {
                let epoch = match get_current_epoch(client.as_ref()) {
                    Ok(epoch) => {
                        info!("Current epoch is: {}", epoch);
                        epoch
                    }
                    Err(err) => {
                        error!("Failed to get current epoch: {}", err);
                        last_epoch
                    }
                };

                if last_epoch != epoch {
                    match get_activated_stake(client.as_ref()) {
                        Ok(stake_weights) => {
                            balancer.write().await.update_stake_weights(
                                stake_weights
                                    .into_iter()
                                    .chain(stake_override.clone())
                                    .collect(),
                            );
                            last_epoch = epoch;
                        }
                        Err(err) => error!("Failed to update stake weights: {}", err),
                    };
                }

                tokio::time::sleep(tokio::time::Duration::from_secs(60)).await;
            }
        });
    }
    if let Some(partners) = &params.test_partners {
        info!("Test partners loaded: {:?}", &partners);
    }

    let _rpc_server = spawn_rpc_server(
        params.rpc_addr.parse().unwrap(),
        params.jwt_public_key,
        params.test_partners,
        balancer.clone(),
    );

    spawn_grpc_server(
        params.grpc_addr.parse().unwrap(),
        params.tls_grpc_server_cert,
        params.tls_grpc_server_key,
        params.tls_grpc_ca_cert,
        balancer.clone(),
    )
    .await?;

    info!("Service stopped.");
    Ok(())
}
