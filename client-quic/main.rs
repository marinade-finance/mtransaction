pub mod grpc_client;
pub mod metrics;
pub mod quic_forwarder;

use crate::grpc_client::spawn_grpc_client;
use crate::quic_forwarder::spawn_quic_forwarded;
use env_logger::Env;
use log::info;
use solana_sdk::signature::read_keypair_file;
use structopt::StructOpt;

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

    let tx_transactions = spawn_quic_forwarded(identity, tpu_addr);

    spawn_grpc_client(
        params.grpc_url.parse().unwrap(),
        params.tls_grpc_ca_cert,
        params.tls_grpc_client_key,
        params.tls_grpc_client_cert,
        tx_transactions,
    )
    .await?;

    info!("Service stopped.");
    Ok(())
}
