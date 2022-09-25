use crate::grpc_server;
use jsonrpc_http_server::*;
use log::{error, info};
use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};
use std::collections::HashMap;
use tokio::sync::mpsc;
use tonic::Status;

#[derive(Debug)]
pub struct TxMessage {
    pub data: String,
}

#[derive(Clone)]
pub struct TxConsumer {
    identity: String,
    stake: u64,
    tx: mpsc::Sender<std::result::Result<grpc_server::pb::TxMessage, Status>>,
}

#[derive(Clone)]
pub struct Balancer {
    pub tx_consumers: HashMap<String, TxConsumer>,
    pub stake_weights: HashMap<String, u64>,
    pub total_connected_stake: u64,
}

impl Balancer {
    pub fn new() -> Self {
        Self {
            // cluster_url:  solana_client(common_params.rpc_url, common_params.commitment);
            tx_consumers: Default::default(),
            stake_weights: Default::default(),
            total_connected_stake: 0,
        }
    }

    pub fn subscribe(
        &mut self,
        identity: String,
    ) -> Option<(
        mpsc::Sender<std::result::Result<grpc_server::pb::TxMessage, Status>>,
        mpsc::Receiver<std::result::Result<grpc_server::pb::TxMessage, Status>>,
    )> {
        if self.tx_consumers.contains_key(&identity) {
            return None;
        }

        info!("Subscribing {}", &identity);
        let (tx, rx) = mpsc::channel(100);
        self.tx_consumers.insert(
            identity.clone(),
            TxConsumer {
                identity: identity.clone(),
                stake: *self.stake_weights.get(&identity).unwrap_or(&1),
                tx: tx.clone(),
            },
        );
        self.recalc_total_connected_stake();
        Some((tx, rx))
    }

    pub fn unsubscribe(&mut self, identity: &String) {
        self.tx_consumers.remove(identity);
        self.recalc_total_connected_stake();
    }

    pub fn pick_tx_consumer(&self) -> Option<TxConsumer> {
        let mut rng: StdRng = SeedableRng::from_entropy();
        if self.total_connected_stake == 0 {
            return None;
        }
        let random_stake_point = rng.gen_range(0..self.total_connected_stake);
        let mut accumulated_sum = 0;

        for (_, tx_consumer) in self.tx_consumers.iter() {
            accumulated_sum += tx_consumer.stake;
            if random_stake_point < accumulated_sum {
                return Some(tx_consumer.clone());
            }
        }
        return None;
    }

    pub fn update_stake_weights(&mut self, stake_weights: HashMap<String, u64>) {
        self.stake_weights = stake_weights;
        for (identity, tx_consumer) in self.tx_consumers.iter_mut() {
            tx_consumer.stake = *self.stake_weights.get(identity).unwrap_or(&1);
        }
        info!("Stake weights updated");
    }

    pub fn recalc_total_connected_stake(&mut self) {
        self.total_connected_stake = self
            .tx_consumers
            .iter()
            .map(|(_, consumer)| consumer.stake)
            .sum();
    }

    pub async fn publish(
        &self,
        data: String,
    ) -> std::result::Result<(), Box<dyn std::error::Error>> {
        if let Some(tx_consumer) = self.pick_tx_consumer() {
            info!("Forwarding to {}", tx_consumer.identity);
            let result = tx_consumer
                .tx
                .send(std::result::Result::<_, Status>::Ok(
                    grpc_server::pb::TxMessage { data },
                ))
                .await;
            match result {
                Ok(_) => info!("Successfully forwarded to {}", tx_consumer.identity),
                Err(err) => error!("Client disconnected {} {}", tx_consumer.identity, err),
            }
            return Ok(());
        }
        error!("Dropping tx, no non-delinquent listening clients");
        Err("No non-delinquent clients with stake are connected".into())
    }
}
