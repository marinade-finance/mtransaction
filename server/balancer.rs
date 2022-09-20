use log::{error, info};
use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::mpsc;
use tokio::sync::RwLock;

#[derive(Debug)]
pub struct TxMessage {
    pub data: String,
}

#[derive(Clone)]
pub struct Balancer {
    pub tx_consumers: HashMap<String, mpsc::Sender<TxMessage>>,
}

impl Balancer {
    pub fn new() -> Self {
        Self {
            tx_consumers: HashMap::new(),
        }
    }

    pub fn add_tx_consumer(&mut self, identity: String) -> mpsc::Receiver<TxMessage> {
        let (tx, rx) = mpsc::channel(100);
        // @todo if already exists
        self.tx_consumers.insert(identity, tx);
        rx
    }

    pub fn remove_tx_consumer(&mut self, identity: &String) {
        self.tx_consumers.remove(identity);
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
            tx.send(TxMessage { data }).await;
            return Ok(());
        }
    }
    error!("wat?");
    Err("RR failure".into())
}
