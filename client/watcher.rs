use log::{info, warn};
use sysinfo::{System, SystemExt};
use tokio::{
    sync::oneshot::{channel, Receiver},
    time::{sleep, Duration},
};
pub const RETRY_TIME_IN_SECONDS: u64 = 20;

pub enum ValidatorStatus {
    Offline,
    Online,
}

pub fn connected() -> bool {
    if let Ok(ValidatorStatus::Online) = check_validator() {
        return true;
    }
    false
}

pub fn check_validator() -> Result<ValidatorStatus, ()> {
    let sys = System::new_all();
    if let None = sys.processes_by_name("solana-valid").next() {
        return Ok(ValidatorStatus::Offline);
    } else {
        return Ok(ValidatorStatus::Online);
    };
}

pub fn spawn_watcher() -> Receiver<ValidatorStatus> {
    let (status_tx, status_rx) = channel();

    tokio::spawn(async move {
        loop {
            match check_validator() {
                Ok(ValidatorStatus::Online) => {
                    info!("Validator status: Online");
                }
                Ok(ValidatorStatus::Offline) => {
                    warn!("Validator status: Offline");
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
