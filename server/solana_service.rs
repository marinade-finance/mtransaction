use crate::metrics::Metric;
use log::{debug, error, info};
use solana_client::{
    nonblocking::pubsub_client::PubsubClient, rpc_client::RpcClient,
    rpc_response::RpcVoteAccountStatus,
};
use solana_sdk::{
    commitment_config::CommitmentConfig, native_token::LAMPORTS_PER_SOL, signature::Signature,
};
use std::cmp::Ordering;
use std::{
    collections::{BinaryHeap, HashMap, HashSet},
    error::Error,
    str::FromStr,
    sync::Arc,
};
use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender};
use tokio_stream::{wrappers::UnboundedReceiverStream, StreamExt, StreamMap};

pub fn solana_client(url: String, commitment: String) -> RpcClient {
    RpcClient::new_with_commitment(url, CommitmentConfig::from_str(&commitment).unwrap())
}

pub fn get_activated_stake(
    client: &RpcClient,
) -> Result<HashMap<String, u64>, Box<dyn Error + Send + Sync>> {
    let RpcVoteAccountStatus {
        current,
        delinquent: _,
    } = client.get_vote_accounts()?;

    Ok(current
        .iter()
        .map(|account| {
            (
                account.node_pubkey.clone(),
                account.activated_stake / LAMPORTS_PER_SOL,
            )
        })
        .collect())
}

pub fn get_current_epoch(client: &RpcClient) -> Result<u64, Box<dyn Error + Send + Sync>> {
    let epoch_info = client.get_epoch_info()?;

    Ok(epoch_info.epoch)
}

pub fn slot_stream(pubsub_client: Arc<PubsubClient>) -> Result<(), Box<dyn Error + Send + Sync>> {
    tokio::spawn(async move {
        let (mut slot_notifications, _slot_unsubscribe) = pubsub_client.slot_subscribe().await?;

        while let Some(slot_info) = slot_notifications.next().await {
            info!("slot: {:?}", slot_info);
        }

        Ok::<_, Box<dyn Error + Send + Sync>>(())
    });
    Ok(())
}

pub fn get_leader_schedule(
    client: &RpcClient,
) -> Result<HashMap<u64, String>, Box<dyn Error + Send + Sync>> {
    let leader_schedule = client
        .get_leader_schedule(None)?
        .expect("No leader schedule!");

    Ok(leader_schedule
        .iter()
        .map(|(identity, slots)| slots.iter().map(|slot| (*slot as u64, identity.clone())))
        .flatten()
        .collect())
}

pub fn get_tpu_by_identity(
    client: &RpcClient,
) -> Result<HashMap<String, String>, Box<dyn Error + Send + Sync>> {
    let nodes = client.get_cluster_nodes()?;

    Ok(nodes
        .iter()
        .flat_map(|node| match node.tpu {
            Some(tpu) => Some((node.pubkey.clone(), tpu.to_string())),
            _ => None,
        })
        .collect())
}

pub fn leaders_stream(
    client: Arc<RpcClient>,
    pubsub_client: Arc<PubsubClient>,
) -> Result<UnboundedReceiver<HashSet<String>>, Box<dyn Error + Send + Sync>> {
    let (tx, rx) = unbounded_channel();

    tokio::spawn(async move {
        let mut refresh_leaders_schedule_hint = Box::pin(
            tokio_stream::iter(std::iter::repeat(()))
                .throttle(tokio::time::Duration::from_secs(3600)),
        ); // todo implement some sound logic to refresh
        let (mut slot_notifications, _slot_unsubscribe) = pubsub_client.slot_subscribe().await?;

        let mut schedule = Default::default();
        let mut last_leaders: HashSet<String> = Default::default();

        loop {
            tokio::select! {
                _ = refresh_leaders_schedule_hint.next() => {
                    info!("Will refresh leaders..");
                    schedule = get_leader_schedule(client.as_ref())?;
                },
                Some(slot_info) = slot_notifications.next() => {
                    let leader_at = slot_info.slot % 432000; // todo get from epoch info
                    let next_leader_at = leader_at + 4; // todo constant
                    let current_leaders: HashSet<_> = vec![schedule.get(&leader_at), schedule.get(&next_leader_at)]
                        .into_iter()
                        .flatten()
                        .cloned()
                        .collect(); // todo separate function
                    debug!("Slot: {:?}, {:?}", slot_info, &current_leaders);
                    if !current_leaders.eq(&last_leaders) {
                        if let Err(err) = tx.send(current_leaders.clone()) {
                            error!("Failed to propagate new leaders: {}", err);
                        }
                        last_leaders = current_leaders;
                    }
                },
                else => break,
            }
        }

        Ok::<_, Box<dyn Error + Send + Sync>>(())
    });

    Ok(rx)
}

#[derive(Eq, PartialEq)]
struct SignatureWatchTimeout {
    subscribed_at: tokio::time::Instant,
    signature: Signature,
}

impl Ord for SignatureWatchTimeout {
    fn cmp(&self, other: &Self) -> Ordering {
        other
            .subscribed_at
            .cmp(&self.subscribed_at)
            .then_with(|| self.signature.cmp(&other.signature))
    }
}
impl PartialOrd for SignatureWatchTimeout {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

pub fn spawn_tx_signature_watcher(
    pubsub_client: Arc<PubsubClient>,
    tx_metrics: UnboundedSender<Vec<Metric>>,
) -> Result<UnboundedSender<Signature>, Box<dyn Error + Send + Sync>> {
    let (tx_signature, rx_signature) = unbounded_channel::<Signature>();

    let mut rx_signature = UnboundedReceiverStream::new(rx_signature);

    let mut prune_subscriptions = Box::pin(
        tokio_stream::iter(std::iter::repeat(())).throttle(tokio::time::Duration::from_secs(10)),
    );

    let minute = tokio::time::Duration::from_secs(60);

    tokio::spawn(async move {
        let mut signature_timeouts: BinaryHeap<SignatureWatchTimeout> = BinaryHeap::new();
        let mut siganture_channels = StreamMap::new();
        let mut signature_unsubscibers = HashMap::new();

        loop {
            tokio::select! {
                _ = prune_subscriptions.next() => {
                    loop {
                        match signature_timeouts.peek() {
                            Some(oldest) => if oldest.subscribed_at.elapsed() < minute {
                                break
                            },
                            _=> break
                        };
                        if let Some(oldest) = signature_timeouts.pop() {
                            error!("Tx is NOT on-chain: {}", oldest.signature);
                            siganture_channels.remove(&oldest.signature);
                            signature_unsubscibers.remove(&oldest.signature);
                            if let Err(err) = tx_metrics.send(vec![Metric::ChainTxTimeout]) {
                                error!("Failed to propagate metrics: {}", err);
                            }
                        }
                    }
                    info!("Signature watchers remaining: {}", signature_timeouts.len());
                },

                Some(signature) = rx_signature.next() => {
                    let (signature_notifications, unsubscribe) = pubsub_client
                        .signature_subscribe(&signature, None)
                        .await?;
                    siganture_channels.insert(signature.clone(), signature_notifications);
                    signature_timeouts.push(SignatureWatchTimeout {
                        subscribed_at: tokio::time::Instant::now(),
                        signature: signature.clone(),
                    });
                    signature_unsubscibers.insert(signature.clone(), unsubscribe);
                    info!("Will watch for {:?}", &signature);
                },

                Some((signature, response)) = siganture_channels.next() => {
                    info!("Tx is on-chain: {}, slot: {}", signature, response.context.slot);
                    signature_unsubscibers.remove(&signature);
                    siganture_channels.remove(&signature);
                    if let Err(err) = tx_metrics.send(vec![Metric::ChainTxFinalized]) {
                        error!("Failed to propagate metrics: {}", err);
                    }
                }
                else => break,
            }
        }

        Ok::<_, Box<dyn Error + Send + Sync>>(())
    });

    Ok(tx_signature)
}
