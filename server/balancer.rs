use crate::grpc_server::{self, build_tx_message_envelope};
use crate::metrics;
use crate::solana_service::{get_tpu_by_identity, leaders_stream};
use jsonrpc_http_server::*;
use log::{error, info};
use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};
use solana_client::{nonblocking::pubsub_client::PubsubClient, rpc_client::RpcClient};
use std::{collections::HashMap, sync::Arc};
use tokio::sync::{mpsc, oneshot, RwLock};
use tokio_stream::{wrappers::UnboundedReceiverStream, StreamExt};
use tonic::Status;

#[derive(Debug)]
pub struct TxMessage {
    pub data: String,
}

pub struct TxConsumer {
    identity: String,
    stake: u64,
    tx: mpsc::Sender<std::result::Result<grpc_server::pb::ResponseMessageEnvelope, Status>>,
    token: String,
    tx_unsubscribe: Option<oneshot::Sender<()>>,
}

impl Drop for TxConsumer {
    fn drop(&mut self) {
        if let Some(tx_unsubscribe) = self.tx_unsubscribe.take() {
            let _ = tx_unsubscribe.send(());
        }
    }
}

pub struct Balancer {
    pub tx_consumers: HashMap<String, TxConsumer>,
    pub stake_weights: HashMap<String, u64>,
    pub total_connected_stake: u64,
    pub leaders: HashMap<String, String>,
    pub leader_tpus: Vec<String>,
}

impl Balancer {
    pub fn new() -> Self {
        Self {
            tx_consumers: Default::default(),
            stake_weights: Default::default(),
            total_connected_stake: 0,
            leaders: Default::default(),
            leader_tpus: Default::default(),
        }
    }

    pub fn subscribe(
        &mut self,
        identity: String,
        token: String,
    ) -> (
        mpsc::Sender<std::result::Result<grpc_server::pb::ResponseMessageEnvelope, Status>>,
        mpsc::Receiver<std::result::Result<grpc_server::pb::ResponseMessageEnvelope, Status>>,
        oneshot::Receiver<()>,
    ) {
        info!("Subscribing {} ({})", &identity, &token);
        let (tx, rx) = mpsc::channel(100);
        let (tx_unsubscribe, rx_unsubcribe) = oneshot::channel();
        self.tx_consumers.insert(
            identity.clone(),
            TxConsumer {
                identity: identity.clone(),
                stake: *self.stake_weights.get(&identity).unwrap_or(&1),
                tx: tx.clone(),
                token,
                tx_unsubscribe: Some(tx_unsubscribe),
            },
        );
        self.recalc_total_connected_stake();
        (tx, rx, rx_unsubcribe)
    }

    pub fn unsubscribe(&mut self, identity: &String, token: &String) {
        if let Some(tx_consumer) = self.tx_consumers.get(identity) {
            if tx_consumer.token.eq(token) {
                self.tx_consumers.remove(identity);
                self.recalc_total_connected_stake();
            }
        }
    }

    pub fn pick_random_tx_consumer_with_target(&self) -> Option<(&TxConsumer, Vec<String>)> {
        let mut rng: StdRng = SeedableRng::from_entropy();
        if self.total_connected_stake == 0 {
            return None;
        }

        let random_stake_point = rng.gen_range(0..self.total_connected_stake);
        let mut accumulated_sum = 0;

        for (_, tx_consumer) in self.tx_consumers.iter() {
            accumulated_sum += tx_consumer.stake;
            if random_stake_point < accumulated_sum {
                return Some((tx_consumer, self.leader_tpus.clone()));
            }
        }
        return None;
    }

    pub fn pick_leader_tx_consumers_with_targets(&self) -> Vec<(&TxConsumer, Vec<String>)> {
        self.tx_consumers
            .iter()
            .flat_map(|(identity, tx_consumer)| match self.leaders.get(identity) {
                Some(tpu) => Some((tx_consumer, vec![tpu.clone()])),
                _ => None,
            })
            .collect()
    }

    pub fn pick_tx_consumers(&self) -> Vec<(&TxConsumer, Vec<String>)> {
        let mut consumers: Vec<_> = self.pick_leader_tx_consumers_with_targets();

        if let Some(random_tx_consumer) = self.pick_random_tx_consumer_with_target() {
            consumers.push(random_tx_consumer);
        }

        consumers
    }

    pub fn update_stake_weights(&mut self, stake_weights: HashMap<String, u64>) {
        self.stake_weights = stake_weights;
        for (identity, tx_consumer) in self.tx_consumers.iter_mut() {
            tx_consumer.stake = *self.stake_weights.get(identity).unwrap_or(&1);
        }
        info!("Stake weights updated");
    }

    pub fn recalc_total_connected_stake(&mut self) {
        for tx_consumer in self.tx_consumers.values() {
            metrics::SERVER_TOTAL_CONNECTED_STAKE
                .with_label_values(&[&tx_consumer.identity])
                .set(tx_consumer.stake as i64);
        }
        self.total_connected_stake = self
            .tx_consumers
            .iter()
            .map(|(_, consumer)| consumer.stake)
            .sum();
    }

    pub async fn publish(
        &self,
        signature: String,
        data: String,
    ) -> std::result::Result<(), Box<dyn std::error::Error>> {
        info!("Forwarding tx {}...", &signature);

        let tx_consumers = self.pick_tx_consumers();

        if tx_consumers.len() == 0 {
            error!("Dropping tx, no available clients");
            return Err("No available clients connected".into());
        }

        for (tx_consumer, tpus) in tx_consumers {
            let result = tx_consumer
                .tx
                .send(std::result::Result::<_, Status>::Ok(
                    build_tx_message_envelope(signature.clone(), data.clone(), tpus),
                ))
                .await;
            match result {
                Ok(_) => info!("Successfully forwarded to {}", tx_consumer.identity),
                Err(err) => error!("Client disconnected {} {}", tx_consumer.identity, err), // TODO unsubscribe
            };
        }
        Ok(())
    }

    pub fn set_leaders(&mut self, leaders: HashMap<String, String>) {
        self.leader_tpus = leaders.values().cloned().collect();
        self.leaders = leaders;
    }
}

pub fn balancer_updater(
    balancer: Arc<RwLock<Balancer>>,
    client: Arc<RpcClient>,
    pubsub_client: Arc<PubsubClient>,
) {
    let balancer = balancer.clone();
    let mut rx_leaders = UnboundedReceiverStream::new(
        leaders_stream(client.clone(), pubsub_client.clone()).unwrap(),
    );

    let mut refresh_cluster_nodes_hint = Box::pin(
        tokio_stream::iter(std::iter::repeat(())).throttle(tokio::time::Duration::from_secs(3600)),
    );

    let mut tpu_by_identity = Default::default();

    tokio::spawn(async move {
        loop {
            tokio::select! {
                _ = refresh_cluster_nodes_hint.next() => {
                    match get_tpu_by_identity(client.as_ref()) {
                        Ok(result) => {
                            info!("Updated TPUs by identity (count: {})", result.len());
                            tpu_by_identity = result;
                        },
                        Err(err) => error!("Failed to update TPUs by identity: {}", err)
                    };
                },
                Some(leaders) = rx_leaders.next() => {
                    let leaders_with_tpus = leaders
                        .iter()
                        .filter_map(|identity| match tpu_by_identity.get(identity) {
                            Some(tpu) => Some((identity.clone(), tpu.clone())),
                            _=> None,
                        })
                        .collect();
                    info!("New leaders {:?}", &leaders_with_tpus);
                    balancer.write().await.set_leaders(leaders_with_tpus);
                },
            }
        }
    });
}
