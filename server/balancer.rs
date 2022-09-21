use crate::grpc_server;
use futures::Stream;
use jsonrpc_http_server::*;
use log::{debug, error, info, warn};
use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};
use std::collections::HashMap;
use std::sync::Arc;
use std::{error::Error, io::ErrorKind, net::ToSocketAddrs, pin::Pin, time::Duration};
use structopt::StructOpt;
use tokio::sync::mpsc;
use tokio::sync::RwLock;
use tokio_stream::{wrappers::ReceiverStream, StreamExt};
use tonic::transport::{Certificate, Identity, Server, ServerTlsConfig};
use tonic::{Request, Response, Status, Streaming};

#[derive(Debug)]
pub struct TxMessage {
    pub data: String,
}

#[derive(Clone)]
pub struct Balancer {
    pub tx_consumers:
        HashMap<String, mpsc::Sender<std::result::Result<grpc_server::pb::TxMessage, Status>>>,
}

impl Balancer {
    pub fn new() -> Self {
        Self {
            tx_consumers: HashMap::new(),
        }
    }

    pub fn subscribe(
        &mut self,
        identity: String,
    ) -> (
        mpsc::Sender<std::result::Result<grpc_server::pb::TxMessage, Status>>,
        mpsc::Receiver<std::result::Result<grpc_server::pb::TxMessage, Status>>,
    ) {
        let (tx, rx) = mpsc::channel(100);
        // @todo if already exists
        self.tx_consumers.insert(identity, tx.clone());
        info!("Client added");
        (tx, rx)
    }

    pub fn unsubscribe(&mut self, identity: &String) {
        self.tx_consumers.remove(identity);
    }

    pub async fn publish(
        &self,
        data: String,
    ) -> std::result::Result<(), Box<dyn std::error::Error>> {
        let mut rng: StdRng = SeedableRng::from_entropy();
        let consumers_count = self.tx_consumers.len();
        for (index, (identity, tx)) in self.tx_consumers.iter().enumerate() {
            if rng.gen_ratio(index as u32 + 1, consumers_count as u32) {
                info!("Forwarding to {}", identity);
                let result = tx
                    .send(std::result::Result::<_, Status>::Ok(
                        grpc_server::pb::TxMessage { data },
                    ))
                    .await;
                match result {
                    Ok(_) => info!("Successfully forwarded to {}", identity),
                    Err(err) => {
                        // todo clean the connection
                        error!("Client disconnected {} {}", identity, err)
                    }
                }
                return Ok(());
            }
        }
        error!("Dropping tx, no listening clients");
        Err("No clients with stake are connected".into())
    }
}

pub async fn send_tx(
    balancer: Arc<RwLock<Balancer>>,
    data: String,
) -> std::result::Result<(), Box<dyn std::error::Error>> {
    // let mut rng = rand::thread_rng();
    let mut rng: StdRng = SeedableRng::from_entropy();
    let balancer = balancer.read().await;
    let consumers_count = balancer.tx_consumers.len();
    // let n = rng.
    for (index, (identity, tx)) in balancer.tx_consumers.iter().enumerate() {
        if rng.gen_ratio(index as u32 + 1, consumers_count as u32) {
            // if true {
            info!("Forwarding to {}", identity);
            tx.send(std::result::Result::<_, Status>::Ok(
                grpc_server::pb::TxMessage { data },
            ))
            .await;
            return Ok(());
        }
    }
    error!("wat?");
    Err("RR failure".into())
}
