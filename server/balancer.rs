use crate::grpc_server::pb;
use crate::grpc_server::{self, build_tpu_message_envelope, build_tx_message_envelope};
use crate::json_str;
use crate::metrics;
use crate::solana_service::SignatureRecord;
use crate::solana_service::{get_leader_info, leaders_stream, LeaderInfo};
use crate::{GOSSIP_ENTRYPOINT, NODES_REFRESH_SECONDS, N_COPIES};
use jsonrpc_http_server::*;
use log::{error, info, warn};
use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};
use solana_client::{nonblocking::pubsub_client::PubsubClient, rpc_client::RpcClient};
use solana_gossip::cluster_info::ClusterInfo;
use solana_gossip::contact_info::Protocol;
use solana_gossip::gossip_service::make_gossip_node;
use solana_sdk::pubkey::Pubkey;
use solana_sdk::signer::keypair;
use solana_streamer::socket::SocketAddrSpace;
use serde::Serialize;
use std::net::ToSocketAddrs;
use std::str::FromStr;
use std::{
    collections::{HashMap, HashSet},
    sync::atomic::{AtomicBool, Ordering},
    sync::Arc,
};
use tokio::sync::mpsc::UnboundedSender;
use tokio::sync::{mpsc, oneshot, RwLock};
use tokio_stream::{wrappers::UnboundedReceiverStream, StreamExt};
use tonic::Status;
use tracing::Instrument;

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

#[derive(Default, Serialize)]
struct Values {
    rtt: Proportion,
    success: Proportion,
}

#[derive(Default, Serialize)]
struct Proportion {
    count: f64,
    total: f64,
}

pub struct Balancer {
    tx_consumers: HashMap<String, TxConsumer>,
    stake_weights: HashMap<String, u64>,
    total_connected_stake: u64,
    leaders: HashMap<String, LeaderInfo>,
    leader_tpus: Vec<LeaderInfo>,
    values: HashMap<String, HashMap<String, Values>>,
    watcher_inbox: UnboundedSender<SignatureRecord>,
}

pub async fn publish_tpus_to_consumer(tx_consumer: &TxConsumer, tpus: Vec<String>) {
    let result = tx_consumer
        .tx
        .send(Ok(build_tpu_message_envelope(tpus)))
        .await;
    match result {
        Ok(_) => {
            info!("notified {} of new leaders", tx_consumer.identity);
        }
        Err(err) => {
            error!("Client disconnected {} {}", tx_consumer.identity, err);
            tx_consumer.do_unsubscribe.store(true, Ordering::Relaxed);
        }
    };
}

impl Balancer {
    pub fn new(watcher_inbox: UnboundedSender<SignatureRecord>) -> Self {
        Self {
            watcher_inbox,
            tx_consumers: Default::default(),
            stake_weights: Default::default(),
            total_connected_stake: Default::default(),
            leaders: Default::default(),
            leader_tpus: Default::default(),
            values: Default::default(),
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
        info!("Subscribing # {{\"identity\":\"{identity}\",\"token\":\"{token}\"}}");
        let (tx, rx) = mpsc::channel(100);
        let (tx_unsubscribe, rx_unsubcribe) = oneshot::channel();
        let tx_consumer = TxConsumer {
            identity: identity.clone(),
            stake: *self.stake_weights.get(&identity).unwrap_or(&1),
            tx: tx.clone(),
            token,
            tx_unsubscribe: Some(tx_unsubscribe),
            do_unsubscribe: false.into(),
        };
        self.tx_consumers.insert(identity.clone(), tx_consumer);
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

    pub fn log_consumer_densities(&self) {
        for consumer in self.tx_consumers.values() {
            for leader in &self.leader_tpus {
                // safety: split should always yield at least one element
                let tpu_ip = leader.tpu.split(':').next().unwrap_or("");
                let identity = &consumer.identity;
                let density = self.calculate_consumer_density(&consumer.identity, &leader.tpu);
                info!("calculate_consumer_density density # {{\"identity\":\"{identity}\",\"tpu_ip\":\"{tpu_ip}\",\"density\":{density}}}");
            }
        }
    }

    pub fn calculate_consumer_density(&self, identity: &str, tpu: &str) -> f64 {
        // the density dereases linearily with rtt reaching 40ms
        // it also decreases with increasing transaction loss for the consumer
        let tpu_ip = match tpu.split(':').next() {
            Some(value) => value,
            None => {
                return 0.1;
            }
        };
        let values = match self.values.get(identity).and_then(|x| x.get(tpu_ip)) {
            Some(value) => value,
            None => {
                return 0.1;
            }
        };

        let rtt_ratio = (values.rtt.total + 400_000.0) / (values.rtt.count + 10.0) / 60_000.0;
        let land_ratio = (values.success.total + 10.0) / (values.success.count + 10.0);
        let bound = (0.1 / (values.rtt.count + 5.0 * values.success.count)).max(0.01);

        // will be always lower than 1.
        (land_ratio * (1.0 - rtt_ratio)).max(bound)
    }

    pub fn select_consumer(&self, stake_point: f64) -> Option<&TxConsumer> {
        let total_weight = if self.total_connected_stake == 0 {
            if !self.tx_consumers.is_empty() {
                self.tx_consumers.len() as u64
            } else {
                return None;
            }
        } else {
            self.total_connected_stake
        };
        let point = (stake_point * total_weight as f64) as u64;

        let mut accumulated_sum = 0;
        for (_, tx_consumer) in self.tx_consumers.iter() {
            accumulated_sum += tx_consumer.stake + 1;
            if point < accumulated_sum {
                return Some(tx_consumer);
            }
        }
        None
    }

    pub fn safe_reject_sample(&self, rng: &mut StdRng, leader: &LeaderInfo) -> Option<&TxConsumer> {
        for i in 0..1000 {
            let candidate = match self.select_consumer(rng.gen()) {
                Some(value) => value,
                // TODO ... this means stake is empty ... should not have to be here
                None => {
                    return None;
                }
            };
            let density = self.calculate_consumer_density(&candidate.identity, &leader.tpu);
            // beware that the density has to be < 1.0, otherwise the
            // algorithm produces bad results
            if rng.gen::<f64>() < density {
                info!("safe_reject_sample took {i} tosses to sample");
                return Some(candidate);
            }
        }
        // if the sampling fails, ignore density, but return a sample
        // this way, the actual density sampled is a mixture
        warn!("safe_reject_sample failed to generate regular sample");
        self.select_consumer(rng.gen())
    }

    pub fn pick_consumers(&self) -> Vec<(&TxConsumer, Vec<LeaderInfo>)> {
        let mut consumers: HashMap<String, Vec<LeaderInfo>> = HashMap::default();
        let mut rng: StdRng = SeedableRng::from_entropy();
        for _ in 0..N_COPIES {
            for (identity, leader) in &self.leaders {
                let leader = leader.clone();
                if let Some(tx_consumer) = self.tx_consumers.get(identity) {
                    // If some of the consumers are leaders, pick them along with their TPU.
                    let entry = consumers.entry(tx_consumer.identity.clone()).or_default();
                    entry.push(leader);
                    continue;
                }

                // Pick the rest randomly.
                let tx_consumer = match self.safe_reject_sample(&mut rng, &leader) {
                    Some(value) => value,
                    None => {
                        continue;
                    }
                };
                let entry = consumers.entry(tx_consumer.identity.clone()).or_default();
                entry.push(leader);
            }
        }

        consumers
            .into_iter()
            .filter_map(|(identity, leaders)| {
                self.tx_consumers
                    .get(&identity)
                    .and_then(|x| Some((x, leaders)))
            })
            .collect()
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
        span: tracing::Span,
        mut session: SignatureRecord,
        signature: String,
        data: String,
    ) -> std::result::Result<(), Box<dyn std::error::Error>> {
        span.in_scope(|| info!("Forwarding tx {}...", &signature));

        let tx_consumers = self.pick_consumers();
        if tx_consumers.is_empty() {
            let _span = span.enter();
            error!("Dropping tx, no available clients");
            return Err(format!("No available clients connected").into());
        }

        for (tx_consumer, info) in tx_consumers {
            let info_json = json_str!(&info);
            let tpus: Vec<_> = info
                .iter()
                .map(|x| x.tpu.clone())
                .collect::<HashSet<_>>()
                .into_iter()
                .collect();
            let tpu_ips: Vec<_> = tpus
                .iter()
                .filter_map(|x| x.split(':').next())
                .map(|x| x.to_string())
                .collect();
            let result = tx_consumer
                .tx
                .send(Ok(build_tx_message_envelope(
                    signature.clone(),
                    data.clone(),
                    tpus,
                )))
                .instrument(span.clone())
                .await;
            let _span = span.enter();
            match result {
                Ok(_) => {
                    info!("forwarded to {} # {info_json}", tx_consumer.identity);
                    session.consumers.push(tx_consumer.identity.clone());
                    for tpu_ip in tpu_ips {
                        metrics::CHAIN_TX_SUBMIT_BY_TPU_IP
                            .with_label_values(&[&tpu_ip])
                            .inc();
                        session.tpu_ips.push(tpu_ip);
                    }
                    metrics::CHAIN_TX_SUBMIT_BY_CONSUMER
                        .with_label_values(&[&tx_consumer.identity])
                        .inc();
                }
                Err(err) => {
                    error!("Client disconnected {} {}", tx_consumer.identity, err);
                    tx_consumer.do_unsubscribe.store(true, Ordering::Relaxed);
                }
            };
        }

        if let Err(err) = self.watcher_inbox.send(session) {
            let _span = span.enter();
            error!("Failed to propagate signature to the watcher: {}", err);
        }

        Ok(())
    }

    pub fn set_leaders(
        &mut self,
        leader_tpus: Vec<LeaderInfo>,
        leaders: HashMap<String, LeaderInfo>,
    ) {
        self.log_consumer_densities();
        self.leader_tpus = leader_tpus;
        self.leaders = leaders;
    }

    pub async fn publish_tpus(&self, leader_tpus: &Vec<LeaderInfo>) {
        // TODO ... redesign into an actor so that txs are not blocked if rtt stalls
        let tpus: HashSet<_> = leader_tpus.iter().map(|x| x.tpu.clone()).collect();
        for tx_consumer in self.tx_consumers.values() {
            let tpus_copy = tpus.iter().cloned().collect();
            publish_tpus_to_consumer(tx_consumer, tpus_copy).await;
        }
    }

    pub fn update_success(&mut self, identity: &str, tpu_ip: &str, success: bool) {
        let slot = self
            .values
            .entry(identity.to_string())
            .or_default()
            .entry(tpu_ip.to_string())
            .or_default();
        if success {
            slot.success.total += 1.0;
        }
        slot.success.count += 1.0;
        let slot_str = format!("\"total\":{},\"count\":{}", slot.success.total, slot.success.count);
        info!("balancer success values updated # {{\"ip\":\"{}\",{slot_str}}}", tpu_ip);
    }

    pub fn update_rtt(&mut self, identity: &str, value: pb::Rtt) {
        let slot = self
            .values
            .entry(identity.to_string())
            .or_default()
            .entry(value.ip.clone())
            .or_default();
        slot.rtt.total += value.took as f64;
        slot.rtt.count += value.n as f64;
        let slot_str = format!("\"total\":{},\"count\":{}", slot.rtt.total, slot.rtt.count);
        info!("balancer rtt values updated # {{\"ip\":\"{}\",{slot_str}}}", value.ip);
        metrics::CLIENT_TPU_IP_PING_RTT
            .with_label_values(&[&identity, &value.ip])
            .set(value.took as f64 / value.n as f64)
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

fn lookup_info(
    cluster_info: &Arc<ClusterInfo>,
    validator_records: &HashMap<String, LeaderInfo>,
    identity: &str,
) -> Option<LeaderInfo> {
    validator_records.get(identity).map(|record| {
        let mut record = record.clone();
        cluster_info
            .lookup_contact_info(&Pubkey::from_str(identity).unwrap(), |x| x.clone())
            .and_then(|x| x.tpu(Protocol::QUIC).ok())
            .map(|x| record.tpu = x.to_string());
        record
    })
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
        exit,
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

    let mut info_map: HashMap<String, LeaderInfo> = HashMap::default();

    tokio::spawn(async move {
        loop {
            tokio::select! {
                _ = nodes_hint.next() => {
                    info!("balancer_updater refreshing validator records");
                    match get_leader_info(client.as_ref()).await {
                        Ok(value) => { info_map = value }
                        Err(err) => {
                            error!("Failed to refresh validator records: {}", err);
                            continue;
                        }
                    }
                },
                Some(plain_leaders) = rx_leaders.next() => {
                    let leaders_info: HashMap<String, LeaderInfo> = plain_leaders
                        .into_iter()
                        .filter_map(
                            |identity|
                            lookup_info(&cluster_info, &info_map, &identity)
                                .map(|value| (identity, value))
                        )
                        .collect();
                    info!("new leaders # {}", json_str!(&leaders_info));
                    let leader_values: Vec<_> = leaders_info.values().cloned().collect();
                    let mut lock = balancer.write().await;
                    lock.publish_tpus(&leader_values).await;
                    lock.set_leaders(leader_values, leaders_info);
                    lock.flush_unsubscribes();
                },
            }
        }
    });
}
