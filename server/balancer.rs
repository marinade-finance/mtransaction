use crate::grpc_server::{self, build_tx_message_envelope};
use crate::metrics;
use crate::solana_service::{get_tpu_by_identity, leaders_stream};
use crate::{NODES_REFRESH_SECONDS, N_CONSUMERS, N_COPIES};
use jsonrpc_http_server::*;
use log::{error, info};
use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};
use solana_client::{nonblocking::pubsub_client::PubsubClient, rpc_client::RpcClient};
use std::{collections::HashMap, sync::Arc, sync::atomic::{Ordering, AtomicBool}};
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
    do_unsubscribe: AtomicBool,
}

impl Drop for TxConsumer {
    fn drop(&mut self) {
        if let Some(tx_unsubscribe) = self.tx_unsubscribe.take() {
            let _ = tx_unsubscribe.send(());
        }
    }
}

#[derive(Default)]
pub struct Balancer {
    pub tx_consumers: HashMap<String, TxConsumer>,
    pub stake_weights: HashMap<String, u64>,
    pub total_connected_stake: u64,
    pub leaders: HashMap<String, String>,
    pub leader_tpus: Vec<String>,
}

impl Balancer {
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
                do_unsubscribe: false.into(),
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

    pub fn sample_consumer_stake_weighted(&self, rng: &mut StdRng) -> Option<&TxConsumer> {
        let total_weight = if self.total_connected_stake == 0 {
            self.tx_consumers.len() as u64
        } else {
            self.total_connected_stake
        };

        let random_stake_point = rng.gen_range(0..total_weight);
        let mut accumulated_sum = 0;

        for (_, tx_consumer) in self.tx_consumers.iter() {
            accumulated_sum += tx_consumer.stake + 1;
            if random_stake_point < accumulated_sum {
                return Some(tx_consumer);
            }
        }
        return None;
    }

    pub fn pick_consumers(&self) -> Vec<(&TxConsumer, Vec<String>)> {
        // If some of the consumers are leaders, pick them.
        // Pick the rest randomly.
        let mut consumers: Vec<_> = self
            .tx_consumers
            .iter()
            .flat_map(|(identity, tx_consumer)| match self.leaders.get(identity) {
                Some(tpu) => Some((tx_consumer, vec![tpu.clone()])),
                _ => None,
            })
            .collect();

        let mut rng = SeedableRng::from_entropy();

        for _ in 0..N_CONSUMERS - consumers.len().min(N_CONSUMERS) {
            // TODO ... make it so that the sample can not degenerate
            match self.sample_consumer_stake_weighted(&mut rng) {
                Some(pick) => {
                    consumers.push((pick, vec![]));
                }
                None => {
                    break;
                }
            }
        }

        let n = consumers.len();
        if n > 0 && !self.leader_tpus.is_empty() {
            let chunk_len = self.leader_tpus.len() / n;
            for (i, tpus) in self.leader_tpus.chunks(chunk_len).enumerate() {
                for tpu in tpus {
                    for j in 0..N_COPIES {
                        consumers[(i + j) % n].1.push(tpu.clone());
                    }
                }
            }
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
        metrics::SERVER_TOTAL_CONNECTED_TX_CONSUMERS.set(self.tx_consumers.len() as i64);

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

        let tx_consumers = self.pick_consumers();

        if tx_consumers.len() == 0 {
            error!("Dropping tx, no available clients");
            return Err("No available clients connected".into());
        }

        for (tx_consumer, tpus) in tx_consumers {
            let tpus_json = serde_json::to_string(&tpus).unwrap_or_else(|_| "null".to_string());
            let result = tx_consumer
                .tx
                .send(std::result::Result::<_, Status>::Ok(
                    build_tx_message_envelope(signature.clone(), data.clone(), tpus),
                ))
                .await;
            match result {
                Ok(_) => info!("forwarded to {} # {tpus_json}", tx_consumer.identity),
                Err(err) => {
                    error!("Client disconnected {} {}", tx_consumer.identity, err);
                    tx_consumer.do_unsubscribe.store(true, Ordering::Relaxed);
                }
            };
        }
        Ok(())
    }

    pub fn set_leaders(&mut self, leaders: HashMap<String, String>) {
        self.leader_tpus = leaders.values().cloned().collect();
        self.leaders = leaders;
    }

    pub fn flush_unsubscribes(&mut self) {
        let mut identities: Vec<(String, String)> = vec![];
        for tx_consumer in self.tx_consumers.values() {
            if tx_consumer.do_unsubscribe.load(Ordering::Relaxed) {
                identities.push((tx_consumer.identity.clone(), tx_consumer.token.clone()));
            }
        }
        for entry in identities {
            self.unsubscribe(&entry.0, &entry.1);
        }
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
        tokio_stream::iter(std::iter::repeat(()))
            .throttle(tokio::time::Duration::from_secs(NODES_REFRESH_SECONDS)),
    );

    let mut tpu_by_identity = Default::default();

    tokio::spawn(async move {
        loop {
            tokio::select! {
                _ = refresh_cluster_nodes_hint.next() => {
                    match get_tpu_by_identity(client.as_ref()) {
                        Ok(result) => {
                            info!("updated TPUs by identity # {{\"count\":{}}}", result.len());
                            tpu_by_identity = result;
                            info!("identities updated # {}", serde_json::to_string(&tpu_by_identity).unwrap_or_else(|_| "null".to_string()));
                        },
                        Err(err) => error!("Failed to update TPUs by identity: {}", err)
                    };
                },
                Some(leaders) = rx_leaders.next() => {
                    let leaders_with_tpus = leaders
                        .into_iter()
                        .filter_map(|identity| match tpu_by_identity.get(&identity) {
                            Some(tpu) => Some((identity.clone(), tpu.clone())),
                            _=> None,
                        })
                        .collect();
                    info!("new leaders # {}", serde_json::to_string(&leaders_with_tpus).unwrap_or_else(|_| "null".to_string()));
                    let mut lock = balancer.write().await;
                    lock.set_leaders(leaders_with_tpus);
                    lock.flush_unsubscribes();
                },
            }
        }
    });
}
