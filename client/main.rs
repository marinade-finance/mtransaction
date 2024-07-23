pub mod forwarder;
pub mod grpc_client;
pub mod metrics;
pub mod quic_forwarder;
pub mod rpc_forwarder;

use crate::forwarder::spawn_forwarder;
use crate::grpc_client::spawn_grpc_client;
use env_logger::Env;
use forwarder::ForwardedTransaction;
use log::{error, info};
use signal_hook_tokio::Signals;
use solana_sdk::signature::{read_keypair_file, Keypair};
use std::ops::Sub;
use std::panic;
use std::process;
use std::time::{Instant, SystemTime};
use std::sync::Arc;
use structopt::StructOpt;
use tokio::{
    sync::{mpsc::UnboundedSender, RwLock},
    task::JoinHandle,
    time::{sleep, Duration},
};
use tonic::transport::Uri;

use signal_hook::consts::SIGHUP;

use std::{collections::HashMap, io::Error};

use futures::stream::StreamExt;

pub const VERSION: &str = "rust-0.0.14-beta";

// Linearly delay retries up to 60 seconds
const GRPC_RECONNECT_DELAY: u64 = 1_000;
const GRPC_RECONNECT_MAX_DELAY: u64 = 60_000;

#[derive(Debug, StructOpt)]
struct Params {
    #[structopt(long = "tls-grpc-ca-cert")]
    tls_grpc_ca_cert: Option<String>,

    #[structopt(long = "tls-grpc-client-key")]
    tls_grpc_client_key: Option<String>,

    #[structopt(long = "tls-grpc-client-cert")]
    tls_grpc_client_cert: Option<String>,

    #[structopt(long = "grpc-urls-file")]
    grpc_urls_file: String,

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

#[inline]
pub fn time_ms() -> u64 {
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64
}

#[inline]
pub fn time_us() -> u64 {
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap()
        .as_micros() as u64
}

#[derive(Clone, Debug)]
pub struct PerfCounterNanos(Instant);
impl Sub for PerfCounterNanos {
    type Output = u64;
    fn sub(self, rhs: Self) -> u64 {
        (self.0 - rhs.0).as_nanos() as u64
    }
}
#[inline]
pub fn perf_counter_ns() -> PerfCounterNanos {
    PerfCounterNanos(Instant::now())
}

fn setup_panic_hook() {
    let hook = panic::take_hook();
    panic::set_hook(Box::new(move |info| {
        hook(info);
        error!("panic_hook forcing exit");
        process::exit(1);
    }));
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    setup_panic_hook();

    env_logger::Builder::from_env(Env::default().default_filter_or("info")).init();

    let params = Params::from_args();

    let (grpc_urls_channel_sender, mut grpc_urls_channel_receiver) =
        tokio::sync::mpsc::channel::<Vec<String>>(1);

    let mut grpc_urls_from_file = read_grpc_urls_from_file(params.grpc_urls_file)
        .expect("failed to read gRPC urls from file");

    let signals = Signals::new(&[SIGHUP])?;

    let _ = tokio::spawn(handle_signals(signals, grpc_urls_channel_sender));

    let (identity, tpu_addr) = match (&params.identity, &params.tpu_addr) {
        (Some(identity), Some(tpu_addr)) => (
            Some(read_keypair_file(identity).unwrap()),
            Some(tpu_addr.parse().unwrap()),
        ),
        _ => (None, None),
    };

    let _metrics = metrics::spawn_metrics(params.metrics_addr.parse().unwrap());

    let tx_transactions = spawn_forwarder(
        identity.as_ref().map(|x| x.insecure_clone()),
        tpu_addr,
        params.rpc_url,
        params.blackhole,
        params.throttle_parallel,
    );

    let all_tasks: RwLock<HashMap<String, JoinHandle<()>>> = RwLock::new(HashMap::new());

    let identity = Arc::new(identity.unwrap_or_else(|| Keypair::new()));
    build_tasks(
        &identity,
        grpc_urls_from_file.clone(),
        &Params::from_args(),
        tx_transactions.clone(),
        &all_tasks,
    )
    .await;

    loop {
        match grpc_urls_channel_receiver.recv().await {
            Some(new_grpc_urls) => {
                let urls_to_spawn: Vec<String> = new_grpc_urls
                    .iter()
                    .filter(|x| !grpc_urls_from_file.contains(x))
                    .map(|x| x.to_string())
                    .collect();

                let mut locked_all_tasks = all_tasks.write().await;
                for grpc in grpc_urls_from_file
                    .iter()
                    .filter(|x| !new_grpc_urls.contains(x))
                {
                    if let Some(task) = locked_all_tasks.remove(grpc) {
                        info!("Aborting task for url: {grpc:?}");
                        task.abort();
                    }
                }
                drop(locked_all_tasks);

                grpc_urls_from_file.clear();
                grpc_urls_from_file.extend(new_grpc_urls.clone());

                if urls_to_spawn.is_empty() {
                    info!("No new urls to spawn for");
                    continue;
                }

                info!("Spawning tasks for new urls: {:?}", urls_to_spawn);

                build_tasks(
                    &identity,
                    urls_to_spawn,
                    &Params::from_args(),
                    tx_transactions.clone(),
                    &all_tasks,
                )
                .await;
            }
            None => {
                info!("No more urls to process");
                break;
            }
        }
    }

    info!("Service stopped.");
    Ok(())
}

async fn spawn_grpc_connection_with_retry(
    identity: Arc<Keypair>,
    grpc_parsed_url: Uri,
    tls_grpc_ca_cert: Option<String>,
    tls_grpc_client_key: Option<String>,
    tls_grpc_client_cert: Option<String>,
    tx_transactions: UnboundedSender<ForwardedTransaction>,
) -> () {
    let mut retry = 0;
    loop {
        let retry_delay = match spawn_grpc_client(
            identity.clone(),
            grpc_parsed_url.clone(),
            tls_grpc_ca_cert.clone(),
            tls_grpc_client_key.clone(),
            tls_grpc_client_cert.clone(),
            tx_transactions.clone(),
            metrics::spawn_feeder(grpc_parsed_url.host().unwrap_or("unknown").to_string()),
        )
        .await
        {
            Ok(_) => {
                break;
            }
            Err(error) => {
                error!("gRPC client failed: {error}");
                retry += 1;

                // Bound the max retry by GRPC_RECONNECT_MAX_DELAY
                GRPC_RECONNECT_MAX_DELAY.min(GRPC_RECONNECT_DELAY * retry)
            }
        };
        info!("retrying {retry} time with a delay of {retry_delay} ms");
        sleep(Duration::from_millis(retry_delay)).await;
    }
}

async fn handle_signals(mut signals: Signals, sender: tokio::sync::mpsc::Sender<Vec<String>>) {
    while let Some(signal) = signals.next().await {
        match signal {
            SIGHUP => {
                info!("Received SIGHUP signal");
                let params = Params::from_args();
                if let Ok(new_grpc_urls) = read_grpc_urls_from_file(params.grpc_urls_file) {
                    info!("Found urls: {:?}", new_grpc_urls);

                    match sender.send(new_grpc_urls).await {
                        Ok(_) => {
                            info!("Sent urls to main thread");
                        }
                        Err(e) => {
                            error!("Error sending new urls to main thread: {:?}", e);
                        }
                    }
                }
            }
            _ => unreachable!(),
        }
    }
}

fn read_grpc_urls_from_file(file_path: String) -> Result<Vec<String>, Error> {
    if let Ok(file) = std::fs::File::open(file_path) {
        let reader = std::io::BufReader::new(file);

        let content: Result<serde_yaml::Value, serde_yaml::Error> = serde_yaml::from_reader(reader);

        match content {
            Ok(content) => {
                if let Some(result) = content["mtransaction_servers"].as_sequence() {
                    let new_urls = result
                        .iter()
                        .map(|x| {
                            if let Some(url) = x.as_str() {
                                return url.to_string();
                            }
                            return "".to_string();
                        })
                        .collect::<Vec<_>>();

                    let new_urls = new_urls
                        .iter()
                        .filter(|x| !x.is_empty())
                        .map(|x| x.to_string())
                        .collect::<Vec<_>>();

                    return Ok(new_urls);
                } else {
                    return Err(Error::new(
                        std::io::ErrorKind::Other,
                        "No mtransaction_servers found",
                    ));
                }
            }
            Err(e) => {
                error!("error reading content: {:?}", e);
                return Err(Error::new(std::io::ErrorKind::Other, e));
            }
        }
    }
    return Err(Error::new(
        std::io::ErrorKind::Other,
        "No mtransaction_servers found",
    ));
}

async fn build_tasks(
    identity: &Arc<Keypair>,
    grpc_urls: Vec<String>,
    params: &Params,
    tx_transactions: UnboundedSender<ForwardedTransaction>,
    all_tasks: &RwLock<HashMap<String, JoinHandle<()>>>,
) {
    let mut all_tasks = all_tasks.write().await;
    for i in grpc_urls {
        let grpc_parsed_url: Uri = i.parse().expect("failed to parse grpc url");

        let tls_grpc_ca_cert = params.tls_grpc_ca_cert.clone();
        let tls_grpc_client_key = params.tls_grpc_client_key.clone();
        let tls_grpc_client_cert = params.tls_grpc_client_cert.clone();
        let tx_transactions = tx_transactions.clone();

        let tsk = tokio::spawn(spawn_grpc_connection_with_retry(
            identity.clone(),
            grpc_parsed_url.clone(),
            tls_grpc_ca_cert.clone(),
            tls_grpc_client_key.clone(),
            tls_grpc_client_cert.clone(),
            tx_transactions.clone(),
        ));

        all_tasks.insert(i.clone(), tsk);
    }
}
