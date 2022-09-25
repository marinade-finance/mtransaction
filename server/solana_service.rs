// use crate::validators::*;
// use bincode::deserialize;
// use log::{debug, error, info};
// use serde_json::{Map, Value};
use solana_client::{rpc_client::RpcClient, rpc_response::RpcVoteAccountStatus};
// use solana_config_program::{config_instruction, get_config_data, ConfigKeys, ConfigState};
use solana_sdk::{commitment_config::CommitmentConfig, native_token::LAMPORTS_PER_SOL};
//     account::from_account,
//     clock::{Epoch, Slot},
//     commitment_config::CommitmentConfig,
//     slot_history::{self, SlotHistory},
//     sysvar,
// };
// use solana_sdk::{
//     account::Account,
//     message::Message,
//     pubkey::Pubkey,
//     signature::{Keypair, Signer},
//     transaction::Transaction,
// };
use std::collections::HashMap;
use std::error::Error;
use std::str::FromStr;
//     collections::{HashMap, HashSet},
//     str::FromStr,
// };
//
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
