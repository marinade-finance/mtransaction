use std::net::ToSocketAddrs;
use std::str::FromStr;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use solana_gossip::contact_info::{LegacyContactInfo, Protocol};
use solana_gossip::gossip_service::make_gossip_node;
use solana_sdk::pubkey::Pubkey;
use solana_sdk::signer::keypair;
use solana_streamer::socket::SocketAddrSpace;

pub fn gossip_for_peer(entrypoint: &str, pubkey: &str, timeout: u64) {
    let keypair = keypair::Keypair::new();
    let exit = Arc::new(AtomicBool::new(false));
    let entry_addrs: Vec<_> = entrypoint.to_socket_addrs().unwrap().collect();
    let (gossip_service, _, cluster_info) = make_gossip_node(
        keypair,
        entry_addrs.first(),
        exit.clone(),
        None,
        0,
        false,
        SocketAddrSpace::Global);

    let pubkey = Pubkey::from_str(pubkey).unwrap();
    let instant = std::time::Instant::now();
    loop {
        if let Some(contact_info) = cluster_info.lookup_contact_info(&pubkey, |x: &LegacyContactInfo| x.clone()) {
            println!("tpu: {:?}\ntpu_forward: {:?}",contact_info.tpu(Protocol::QUIC).unwrap(), contact_info.tpu_forwards(Protocol::QUIC).unwrap());
            break;
        } else if instant.elapsed() >= std::time::Duration::from_secs(timeout) {
            println!("Timed out after {} seconds",timeout);
            break;
        } else {
            std::thread::sleep(std::time::Duration::from_secs(1));
        }
    }

    exit.store(true, Ordering::Relaxed);
    gossip_service.join().unwrap();
}

fn main() {
    gossip_for_peer("entrypoint3.mainnet-beta.solana.com:8001", "C1ocKDYMCm2ooWptMMnpd5VEB2Nx4UMJgRuYofysyzcA", 60);
}