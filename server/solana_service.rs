use crate::{metrics, rpc_server::Mode, json_str};
use crate::{LEADER_REFRESH_SECONDS, N_LEADERS};
use eyre::bail;
use eyre::Result;
use log::{debug, error, info};
use serde::{Deserialize, Serialize};
use solana_client::{
    nonblocking::pubsub_client::PubsubClient, rpc_client::RpcClient,
    rpc_response::RpcVoteAccountStatus,
};
use solana_sdk::{
    commitment_config::CommitmentConfig, native_token::LAMPORTS_PER_SOL, signature::Signature,
};
use solana_transaction_status::TransactionStatus;
use std::{
    collections::{HashMap, HashSet, VecDeque},
    error::Error,
    str::FromStr,
    sync::Arc,
};
use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender};
use tokio_stream::{wrappers::UnboundedReceiverStream, StreamExt};

const SLOTS_PER_EPOCH: u64 = 432000;

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
        .flat_map(|node| match node.tpu_quic {
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
                .throttle(tokio::time::Duration::from_secs(LEADER_REFRESH_SECONDS)),
        ); // todo implement some sound logic to refresh
        let (mut slot_notifications, _slot_unsubscribe) = pubsub_client.slot_subscribe().await?;

        let mut schedule = Default::default();
        let mut last_leaders: HashSet<String> = Default::default();

        loop {
            tokio::select! {
                _ = refresh_leaders_schedule_hint.next() => {
                    info!("Will refresh leaders...");
                    schedule = get_leader_schedule(client.as_ref())?;
                    info!("leaders refreshed # {}", serde_json::to_string(&schedule).unwrap_or_else(|_| "null".to_string()));
                },
                Some(slot_info) = slot_notifications.next() => {
                    let current_leaders: HashSet<_> = (0..N_LEADERS)
                        .map(|nth_leader| nth_leader * 4 + (slot_info.slot % SLOTS_PER_EPOCH))
                        .map(|slot| schedule.get(&slot))
                        .flatten()
                        .cloned()
                        .collect();
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

#[derive(Debug, Clone, Serialize)]
pub struct SignatureRecord {
    #[serde(skip)]
    pub span: tracing::Span,
    #[serde(skip)]
    pub created_at: tokio::time::Instant,
    pub ctime: u64,
    pub rtime: u64,
    pub req_id: usize,
    pub signature: Signature,
    pub mode: Mode,
    pub consumers: Vec<String>,
    pub tpu_ips: HashSet<String>,
    pub partner_name: String,
}

pub fn spawn_tx_signature_watcher(
    client: Arc<RpcClient>,
) -> Result<UnboundedSender<SignatureRecord>, Box<dyn Error + Send + Sync>> {
    let (tx_signature, rx_signature) = unbounded_channel::<SignatureRecord>();

    let mut rx_signature = UnboundedReceiverStream::new(rx_signature);

    let mut bundle_subscriptions_signal = Box::pin(
        tokio_stream::iter(std::iter::repeat(())).throttle(tokio::time::Duration::from_secs(1)),
    );

    let signature_check_after = tokio::time::Duration::from_secs(10);
    let max_bundle_size = 250;

    tokio::spawn(async move {
        let mut signature_queue: VecDeque<SignatureRecord> = Default::default();

        loop {
            tokio::select! {
                _ = bundle_subscriptions_signal.next() => {
                    loop {
                        let mut to_be_bundled_count = 0;
                        for record in signature_queue.iter() {
                            if record.created_at.elapsed() > signature_check_after && to_be_bundled_count < max_bundle_size {
                                to_be_bundled_count += 1;
                            } else {
                                break;
                            }
                        }
                        if to_be_bundled_count == 0 {
                            break;
                        }
                        {
                            let bundle: Vec<_> = signature_queue.drain(0..to_be_bundled_count).collect();

                            tokio::spawn(signature_checker(client.clone(), bundle));
                        }
                    }
                },
                Some(mut record) = rx_signature.next() => {
                    record.created_at = tokio::time::Instant::now();
                    let span = record.span.clone();
                    let _span = span.enter();
                    info!("Will watch for {}", record.signature);
                    signature_queue.push_back(record);
                },
                else => break,
            }
        }

        Ok::<_, Box<dyn Error + Send + Sync>>(())
    });

    Ok(tx_signature)
}

fn format_status(status: &TransactionStatus) -> String {
    let slot = &status.slot;
    let confirmations = match &status.confirmations {
        Some(value) => value.to_string(),
        None => "null".to_string(),
    };
    let status = match &status.status {
        Ok(()) => "ok".to_string(),
        Err(err) => json_str!(&format!("{}", err)),
    };
    format!("\"slot\":{slot},\"confirmations\":{confirmations},\"status\":\"{status}\"")
}

async fn signature_checker(client: Arc<RpcClient>, bundle: Vec<SignatureRecord>) {
    match client.get_signature_statuses(&bundle.iter().map(|f| f.signature).collect::<Vec<_>>())
    {
        Ok(response) => {
            for (record, signature_status) in bundle.iter().zip(response.value.iter()) {
                let span = record.span.clone();
                let _span = span.enter();
                if let Some(known_status) = signature_status {
                    info!(
                        "new signature status # {{{},\"rtime\":{},\"clients\":{},\"tpu_ips\":{}}}",
                        format_status(known_status),
                        record.rtime,
                        json_str!(&record.consumers),
                        json_str!(&record.tpu_ips),
                    );
                    match known_status.err {
                        Some(_) => metrics::CHAIN_TX_EXECUTION_SUCCESS
                            .with_label_values(&[
                                &record.partner_name,
                                &record.mode.to_string(),
                            ])
                            .inc(),
                        _ => metrics::CHAIN_TX_EXECUTION_SUCCESS
                            .with_label_values(&[
                                &record.partner_name,
                                &record.mode.to_string(),
                            ])
                            .inc(),
                    };
                    metrics::CHAIN_TX_FINALIZED
                        .with_label_values(&[&record.partner_name, &record.mode.to_string()])
                        .inc();
                    for consumer in &record.consumers {
                        metrics::CHAIN_TX_FINALIZED_BY_CONSUMER
                            .with_label_values(&[&consumer])
                            .inc();
                    }
                    for tpu_ip in &record.tpu_ips {
                        metrics::CHAIN_TX_FINALIZED_BY_TPU_IP
                            .with_label_values(&[&tpu_ip])
                            .inc();
                    }
                } else {
                    metrics::CHAIN_TX_TIMEOUT
                        .with_label_values(&[&record.partner_name, &record.mode.to_string()])
                        .inc();
                    for consumer in &record.consumers {
                        metrics::CHAIN_TX_TIMEOUT_BY_CONSUMER
                            .with_label_values(&[&consumer])
                            .inc();
                    }
                    for tpu_ip in &record.tpu_ips {
                        metrics::CHAIN_TX_TIMEOUT_BY_TPU_IP
                            .with_label_values(&[&tpu_ip])
                            .inc();
                    }
                }
            }
        }
        Err(err) => {
            for tx in bundle {
                let _span = tx.span.enter();
                error!("Failed to get signature status: {}", err);
                metrics::CHAIN_TX_TIMEOUT
                    .with_label_values(&[&tx.partner_name, &tx.mode.to_string()])
                    .inc();
            }
        }
    }
}

#[derive(Deserialize)]
struct JitoValidatorResponse {
    validators: Vec<JitoValidatorRecord>,
}

#[derive(Deserialize)]
struct JitoValidatorRecord {
    vote_account: String,
    running_jito: bool,
}

async fn fetch_jito_validators() -> Result<HashSet<String>> {
    let url = "https://kobe.mainnet.jito.network/api/v1/validators";
    let resp = reqwest::get(url).await?.text().await?;
    let data: JitoValidatorResponse = serde_json::from_str(&resp)?;
    let mut result = HashSet::default();
    for entry in data.validators {
        if entry.running_jito {
            result.insert(entry.vote_account);
        }
    }
    Ok(result)
}

#[derive(Deserialize)]
struct ValidatorResponse {
    validators: Vec<ValidatorRecord>,
}

#[derive(Deserialize)]
struct ValidatorRecord {
    identity: String,
    vote_account: String,
    pub dc_full_city: String,
}

async fn fetch_validator_records() -> Result<HashMap<String, ValidatorRecord>> {
    let url = "http://validators-api.marinade.finance/validators?limit=9999&epochs=0";
    let resp = reqwest::get(url).await?.text().await?;
    let data: ValidatorResponse = serde_json::from_str(&resp)?;
    let mut result = HashMap::default();
    for entry in data.validators {
        result.insert(entry.identity.clone(), entry);
    }
    Ok(result)
}

#[derive(Clone, Serialize, Debug)]
pub struct LeaderInfo {
    pub identity: String,
    pub tpu: String,
    pub dc_full_city: String,
    pub is_jito: bool,
}

pub async fn get_leader_info(client: &RpcClient) -> eyre::Result<HashMap<String, LeaderInfo>> {
    let tpus = match get_tpu_by_identity(client) {
        Ok(result) => {
            info!("updated TPUs by identity # {{\"count\":{}}}", result.len());
            result
        }
        Err(err) => bail!("Failed to update TPUs by identity: {}", err),
    };
    let validator_records = match fetch_validator_records().await {
        Ok(value) => value,
        Err(err) => bail!("Failed to fetch validator records: {}", err),
    };
    let jito_identities = match fetch_jito_validators().await {
        Ok(value) => value,
        Err(err) => bail!("Failed to fetch Jito validators: {}", err),
    };

    Ok(merge_validator_records(
        tpus,
        validator_records,
        jito_identities,
    ))
}

fn merge_validator_records(
    tpus: HashMap<String, String>,
    validator_records: HashMap<String, ValidatorRecord>,
    jito_identities: HashSet<String>,
) -> HashMap<String, LeaderInfo> {
    let mut result: HashMap<String, LeaderInfo> = HashMap::default();
    for (identity, tpu) in tpus.into_iter() {
        validator_records.get(&identity).map(|record| {
            let is_jito = jito_identities.get(&record.vote_account).is_some();
            let info = LeaderInfo {
                identity: identity.clone(),
                tpu,
                dc_full_city: record.dc_full_city.clone(),
                is_jito,
            };
            result.insert(identity, info);
        });
    }
    result
}
