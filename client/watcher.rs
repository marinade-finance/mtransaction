use log::{info, warn};
use sysinfo::{System, SystemExt};
use tokio::{
    sync::oneshot::{channel, Receiver},
    time::{sleep, Duration},
};
const RETRY_COUNT: u64 = 3;
const RETRY_TIME_IN_SECONDS: u64 = 20;

pub enum ValidatorStatus {
    Offline,
    Online,
}

pub async fn check_validator() -> Result<ValidatorStatus, ()> {
    tokio::spawn(async move {
        let mut retry_count = RETRY_COUNT;
        loop {
            let sys = System::new_all();
            let process = sys.processes_by_name("solana-valid").next();
            match process {
                None => {
                    if retry_count == 0 {
                        return Ok(ValidatorStatus::Offline);
                    }
                    retry_count = retry_count.saturating_sub(1);
                    warn!(
                        "Validator service is unreachable. Retrying in {:?} seconds",
                        RETRY_TIME_IN_SECONDS
                    );
                    sleep(Duration::from_secs(RETRY_TIME_IN_SECONDS)).await;

                    continue;
                }
                Some(_) => {
                    return Ok(ValidatorStatus::Online);
                }
            }
        }
    })
    .await
    .unwrap()
}

pub fn spawn_watcher() -> Receiver<ValidatorStatus> {
    let (status_tx, status_rx) = channel();

    tokio::spawn(async move {
        loop {
            match check_validator().await {
                Ok(ValidatorStatus::Online) => {
                    info!("Validator status: Online");
                }
                Ok(ValidatorStatus::Offline) => {
                    if let Err(_) = status_tx.send(ValidatorStatus::Offline) {
                        info!("Validator watcher receiver dropped.");
                    }
                    break;
                }
                Err(_) => {
                    if let Err(_) = status_tx.send(ValidatorStatus::Offline) {
                        info!("Validator watcher receiver dropped.");
                    }
                    break;
                }
            }
            sleep(Duration::from_secs(RETRY_TIME_IN_SECONDS)).await;
        }
    });
    status_rx
}
