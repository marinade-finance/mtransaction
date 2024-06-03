use crate::grpc_server::{self, build_tx_message_envelope};
use crate::json_str;
use crate::metrics;
use crate::solana_service::{
    get_jito_validators, get_tpu_by_identity, get_vote_to_identity_map, leaders_stream,
};
use crate::GOSSIP_ENTRYPOINT;
use crate::{NODES_REFRESH_SECONDS, N_CONSUMERS, N_COPIES};
use jsonrpc_http_server::*;
use log::{error, info};
use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};
use serde::Serialize;
use solana_client::{nonblocking::pubsub_client::PubsubClient, rpc_client::RpcClient};
use solana_gossip::contact_info::Protocol;
use solana_gossip::gossip_service::make_gossip_node;
use solana_sdk::pubkey::Pubkey;
use solana_sdk::signer::keypair;
use solana_streamer::socket::SocketAddrSpace;
use std::net::ToSocketAddrs;
use std::str::FromStr;
use std::{
    collections::{HashMap, HashSet},
    sync::atomic::{AtomicBool, Ordering},
    sync::Arc,
};
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

#[derive(Clone, Serialize)]
pub struct LeaderInfo {
    pub identity: String,
    pub tpu: String,
    pub is_jito: bool,
}

#[derive(Default)]
pub struct Balancer {
    pub tx_consumers: HashMap<String, TxConsumer>,
    pub stake_weights: HashMap<String, u64>,
    pub total_connected_stake: u64,
    pub leaders: HashMap<String, LeaderInfo>,
    pub leader_tpus: Vec<LeaderInfo>,
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

    pub fn pick_consumers(&self) -> Vec<(&TxConsumer, Vec<LeaderInfo>)> {
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

        for (tx_consumer, info) in tx_consumers {
            let info_json = json_str!(&info);
            let tpus = info.iter().map(|x| x.tpu.clone()).collect();
            let result = tx_consumer
                .tx
                .send(std::result::Result::<_, Status>::Ok(
                    build_tx_message_envelope(signature.clone(), data.clone(), tpus),
                ))
                .await;
            match result {
                Ok(_) => info!("forwarded to # {info_json}"),
                Err(err) => {
                    error!("Client disconnected {} {}", tx_consumer.identity, err);
                    tx_consumer.do_unsubscribe.store(true, Ordering::Relaxed);
                }
            };
        }
        Ok(())
    }

    pub fn set_leaders(
        &mut self,
        leader_tpus: Vec<LeaderInfo>,
        leaders: HashMap<String, LeaderInfo>,
    ) {
        self.leader_tpus = leader_tpus;
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

    let exit = Arc::new(AtomicBool::new(false));
    let keypair = keypair::Keypair::new();
    let entry_addrs: Vec<_> = GOSSIP_ENTRYPOINT.to_socket_addrs().unwrap().collect();
    info!("balancer_updater spawning...");
    let (_gossip_service, _, cluster_info) = make_gossip_node(
        keypair,
        entry_addrs.first(),
        exit.clone(),
        None,
        0,
        false,
        SocketAddrSpace::Global,
    );
    info!("balancer_updater peer started");

    let mut nodes_hint = Box::pin(
        tokio_stream::iter(std::iter::repeat(()))
            .throttle(tokio::time::Duration::from_secs(NODES_REFRESH_SECONDS)),
    );

    let mut tpu_by_identity = Default::default();
    let mut jito_identities: HashSet<String> = Default::default();

    tokio::spawn(async move {
        loop {
            tokio::select! {
                _ = nodes_hint.next() => {
                    match get_tpu_by_identity(client.as_ref()) {
                        Ok(result) => {
                            info!("updated TPUs by identity # {{\"count\":{}}}", result.len());
                            tpu_by_identity = result;
                            info!("identities updated # {}", json_str!(&tpu_by_identity));
                        },
                        Err(err) => error!("Failed to update TPUs by identity: {}", err)
                    };
                    let map = match get_vote_to_identity_map().await {
                        Ok(value) => { value }
                        Err(err) => {
                            error!("{}", err);
                            continue;
                        }
                    };
                    jito_identities = match get_jito_validators(&map).await {
                        Ok(value) => { value }
                        Err(err) => {
                            error!("{}", err);
                            continue;
                        }
                    };

                },
                Some(plain_leaders) = rx_leaders.next() => {
                    let leaders_info: HashMap<_, _> = plain_leaders
                        .into_iter()
                        .filter_map(
                            |identity| match cluster_info
                                .lookup_contact_info(&Pubkey::from_str(&identity).unwrap(), |x| x.clone())
                                .and_then(|x| x.tpu(Protocol::QUIC).ok())
                                .map(|x| x.to_string())
                                .or(tpu_by_identity.get(&identity).cloned()) {
                                    Some(tpu) => {
                                        let is_jito = jito_identities.get(&identity).is_some();
                                        Some((
                                            identity.clone(),
                                            LeaderInfo {
                                                identity: identity.clone(),
                                                tpu: tpu.clone(),
                                                is_jito,
                                            }))
                                    }
                                    _ => { None }
                                })
                        .collect();
                    info!("new leaders # {}", json_str!(&leaders_info));
                    let leader_values = leaders_info.values().cloned().collect();
                    let mut lock = balancer.write().await;
                    lock.set_leaders(leader_values, leaders_info);
                    lock.flush_unsubscribes();
                },
            }
        }

        // If you ever wanted to stop the gossip_service
        // exit.store(true, Ordering::Relaxed);
        // gossip_service.join().unwrap();
    });
}
