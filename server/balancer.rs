use crate::grpc_server::{self, build_tx_message_envelope};
use crate::metrics::Metric;
use crate::solana_service::{get_tpu_by_identity, leaders_stream};
use jsonrpc_http_server::*;
use log::{error, info};
use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};
use solana_client::{nonblocking::pubsub_client::PubsubClient, rpc_client::RpcClient};
use std::{collections::HashMap, sync::Arc};
use tokio::sync::mpsc::UnboundedSender;
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
    pub leader_tpus: Vec<String>,
    tx_metrics: UnboundedSender<Vec<Metric>>,
}

impl Balancer {
    pub fn new(tx_metrics: UnboundedSender<Vec<Metric>>) -> Self {
        Self {
            tx_consumers: Default::default(),
            stake_weights: Default::default(),
            total_connected_stake: 0,
            leader_tpus: Default::default(),
            tx_metrics: tx_metrics,
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

    pub fn pick_tx_consumer(&self) -> Option<&TxConsumer> {
        let mut rng: StdRng = SeedableRng::from_entropy();
        if self.total_connected_stake == 0 {
            return None;
        }
        let random_stake_point = rng.gen_range(0..self.total_connected_stake);
        let mut accumulated_sum = 0;

        for (_, tx_consumer) in self.tx_consumers.iter() {
            accumulated_sum += tx_consumer.stake;
            if random_stake_point < accumulated_sum {
                return Some(tx_consumer);
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
        if let Err(err) = self
            .tx_metrics
            .send(vec![Metric::ServerTotalConnectedStake {
                stake: self.total_connected_stake,
            }])
        {
            error!("Failed to update total stake metrics: {}", err);
        }
    }

    pub async fn publish(
        &self,
        signature: String,
        data: String,
    ) -> std::result::Result<(), Box<dyn std::error::Error>> {
        info!("Forwarding tx {}...", &signature);
        if let Some(tx_consumer) = self.pick_tx_consumer() {
            let result = tx_consumer
                .tx
                .send(std::result::Result::<_, Status>::Ok(
                    build_tx_message_envelope(signature, data, self.leader_tpus.clone()),
                ))
                .await;
            match result {
                Ok(_) => info!("Successfully forwarded to {}", tx_consumer.identity),
                Err(err) => error!("Client disconnected {} {}", tx_consumer.identity, err), // TODO unsubscribe
            }
            return Ok(());
        }
        error!("Dropping tx, no non-delinquent listening clients");
        Err("No non-delinquent clients with stake are connected".into())
    }

    pub fn set_leader_tpus(&mut self, tpus: Vec<String>) {
        self.leader_tpus = tpus;
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
                leaders = rx_leaders.next() => {
                    if let Some(leaders) = leaders {
                        let tpus = leaders
                            .iter()
                            .filter_map(|identity| tpu_by_identity.get(identity))
                            .cloned()
                            .collect();
                        info!("New leaders {:?} {:?}", &leaders, &tpus);
                        balancer.write().await.set_leader_tpus(tpus);
                    }
                },
            }
        }
    });
}
