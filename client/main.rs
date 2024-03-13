pub mod forwarder;
pub mod grpc_client;
pub mod metrics;
pub mod quic_forwarder;
pub mod rpc_forwarder;

use crate::forwarder::spawn_forwarder;
use crate::grpc_client::spawn_grpc_client;
use env_logger::Env;
use forwarder::ForwardedTransaction;
use futures::{executor::block_on, TryFutureExt};
use grpc_client::pb::RequestMessageEnvelope;
use log::{error, info};
use solana_sdk::signature::read_keypair_file;
use structopt::StructOpt;
use tokio::{
    sync::mpsc::{Receiver, UnboundedSender},
    time::{sleep, Duration},
};
use tonic::transport::Uri;

pub const VERSION: &str = "rust-0.0.7-beta";
const MAX_RETRIES: i32 = 3;

#[derive(Debug, StructOpt)]
struct Params {
    #[structopt(long = "tls-grpc-ca-cert")]
    tls_grpc_ca_cert: Option<String>,

    #[structopt(long = "tls-grpc-client-key")]
    tls_grpc_client_key: Option<String>,

    #[structopt(long = "tls-grpc-client-cert")]
    tls_grpc_client_cert: Option<String>,

    #[structopt(long = "grpc-url", default_value = "http://127.0.0.1:50051")]
    grpc_urls: Vec<String>,

    #[structopt(long = "identity")]
    identity: Option<String>,

    #[structopt(long = "tpu-addr")]
    tpu_addr: Option<String>,

    #[structopt(long = "metrics-addr", default_value = "127.0.0.1:9091")]
    metrics_addr: String,

    #[structopt(long = "rpc-url")]
    rpc_url: Option<String>,

    #[structopt(long = "blackhole")]
    blackhole: bool,

    #[structopt(long = "throttle-parallel", default_value = "1000")]
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

    let _metrics = metrics::spawn_metrics(params.metrics_addr.parse().unwrap());

    let tx_transactions = spawn_forwarder(
        identity,
        tpu_addr,
        params.rpc_url,
        params.blackhole,
        params.throttle_parallel,
    );

    let tasks: Vec<_> = params
        .grpc_urls
        .clone()
        .iter_mut()
        .map(|grpc_url| {
            let grpc_parsed_url: Uri = grpc_url.parse().unwrap();
            let mut feeder =
                metrics::spawn_feeder(grpc_parsed_url.host().unwrap_or("unknown").to_string());

            let tls_grpc_ca_cert = params.tls_grpc_ca_cert.clone();
            let tls_grpc_client_key = params.tls_grpc_client_key.clone();
            let tls_grpc_client_cert = params.tls_grpc_client_cert.clone();
            let tx_transactions = tx_transactions.clone();

            let mut retry_attempts = 0;

            tokio::spawn(with_retry(
                grpc_parsed_url.clone(),
                tls_grpc_ca_cert.clone(),
                tls_grpc_client_key.clone(),
                tls_grpc_client_cert.clone(),
                tx_transactions.clone(),
            ))

            // tokio::spawn(async move {
            //     // while retry_attempts <= MAX_RETRIES {
            //     // if let Err(error) =
            //     spawn_grpc_client(
            //         grpc_parsed_url.clone(),
            //         tls_grpc_ca_cert.clone(),
            //         tls_grpc_client_key.clone(),
            //         tls_grpc_client_cert.clone(),
            //         tx_transactions.clone(),
            //         &mut feeder,
            //     )
            //     .await
            //     // .unwrap_or_else(handle_it)
            //     .unwrap_or_else(|e| {
            //         error!("IN UNWRAP OR ELSE: {:?}", e);
            //         block_on(async {
            //             tokio::spawn(spawn_with_delay(
            //                 grpc_parsed_url.clone(),
            //                 tls_grpc_ca_cert.clone(),
            //                 tls_grpc_client_key.clone(),
            //                 tls_grpc_client_cert.clone(),
            //                 tx_transactions.clone(),
            //             ))
            //             .await;
            //         });
            //     });

            //     // .await
            //     // {
            //     //     error!("gRPC client failed: {error}");
            //     // }
            //     // retry_attempts += 1;
            //     // info!(
            //     //     "retrying with a delay of 1 second. Retry attempt: {retry_attempts}"
            //     // );
            //     // sleep(Duration::from_millis(1_000)).await;
            //     // }
            // })
        })
        .collect();

    futures::future::join_all(tasks).await;

    info!("Service stopped.");
    Ok(())
}

// fn handle_it(x:Box<dyn std::error::Error>){
//     block_on(spawn_with_delay(grpc_parsed_url, tls_grpc_ca_cert, tls_grpc_client_key, tls_grpc_client_cert, tx_transactions))
// }

async fn spawn_with_delay(
    grpc_parsed_url: Uri,
    tls_grpc_ca_cert: Option<String>,
    tls_grpc_client_key: Option<String>,
    tls_grpc_client_cert: Option<String>,
    tx_transactions: UnboundedSender<ForwardedTransaction>,
) {
    info!("IN SPAWN WITH DELAY");

    let mut feeder = metrics::spawn_feeder(grpc_parsed_url.host().unwrap_or("unknown").to_string());

    let mut retry_attempts = 0;
    let _ = tokio::spawn(async move {
        info!("IN SPAWN WITH DELAY 2");
        while retry_attempts <= MAX_RETRIES {
            info!("IN WHILE {retry_attempts}");
            // if let Err(error) =
            spawn_grpc_client(
                grpc_parsed_url.clone(),
                tls_grpc_ca_cert.clone(),
                tls_grpc_client_key.clone(),
                tls_grpc_client_cert.clone(),
                tx_transactions.clone(),
                &mut feeder,
            )
            .await
            .map_err(|_e| error!("Retry Failed in spawn"));
            // {
            //     error!("gRPC client failed: {error}");
            // }
            retry_attempts += 1;
            info!("retrying with a delay of 1 second. Retry attempt: {retry_attempts}");
            sleep(Duration::from_millis(1_000)).await;
        }
    });

    // let _ = spawn_grpc_client(
    //     grpc_parsed_url.clone(),
    //     tls_grpc_ca_cert.clone(),
    //     tls_grpc_client_key.clone(),
    //     tls_grpc_client_cert.clone(),
    //     tx_transactions.clone(),
    //     &mut feeder,
    // )
    // .await
    // .map_err(|_e| error!("Retry Failed in spawn"));
    info!("BELOW");

    return;
}

async fn with_retry(
    grpc_parsed_url: Uri,
    tls_grpc_ca_cert: Option<String>,
    tls_grpc_client_key: Option<String>,
    tls_grpc_client_cert: Option<String>,
    tx_transactions: UnboundedSender<ForwardedTransaction>,
) -> () {
    let mut feeder = metrics::spawn_feeder(grpc_parsed_url.host().unwrap_or("unknown").to_string());
    for _ in 0..5 {
        match spawn_grpc_client(
            grpc_parsed_url.clone(),
            tls_grpc_ca_cert.clone(),
            tls_grpc_client_key.clone(),
            tls_grpc_client_cert.clone(),
            tx_transactions.clone(),
            &mut feeder,
        )
        .await
        {
            Ok(o) => {
                info!("{:?}", o);
                info!("gRPC client succeeded");
                break;
            }
            Err(error) => {
                error!("gRPC client failed: {error}");
                info!("retrying with a delay of 1 second");
            }
        }
        sleep(Duration::from_millis(1_000)).await;
    };
}
