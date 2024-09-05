use crossbeam_channel::{unbounded, Receiver};
use env_logger::Env;
use log::{debug, info};
use std::collections::HashMap;
use std::net::UdpSocket;
use std::thread;
use {
    solana_sdk::signature::Keypair,
    std::{
        sync::{atomic::AtomicBool, Arc, RwLock},
        time::Duration,
    },
};

use solana_streamer::{
    nonblocking::quic::DEFAULT_WAIT_FOR_CHUNK_TIMEOUT,
    packet::PacketBatch,
    quic::{spawn_server, MAX_STAKED_CONNECTIONS, MAX_UNSTAKED_CONNECTIONS},
    streamer::StakedNodes,
};

use solana_core::tpu::MAX_QUIC_CONNECTIONS_PER_PEER;

fn main_loop(channel: Receiver<PacketBatch>) {
    info!("main_loop thread started");
    while let Ok(data) = channel.recv() {
        debug!("{data:?}");
    }
}

fn main() {
    env_logger::Builder::from_env(Env::default().default_filter_or("info")).init();

    let transactions_quic_sockets =
        UdpSocket::bind("127.0.0.1:49876").expect("couldn't bind to address");
    let (packet_sender, packet_receiver) = unbounded();
    let keypair = Keypair::new();
    let tpu_coalesce = Duration::from_secs(10);
    let overrides = HashMap::default();
    let staked_nodes = Arc::new(RwLock::new(StakedNodes::new(
        Arc::new(HashMap::default()),
        overrides,
    )));
    let exit = Arc::new(AtomicBool::new(false));
    let gossip_host = "127.0.0.1".parse().expect("invalid ip address");

    let (_, server_thread) = spawn_server(
        "solQuicTpu",
        transactions_quic_sockets,
        &keypair,
        gossip_host,
        packet_sender,
        exit.clone(),
        MAX_QUIC_CONNECTIONS_PER_PEER,
        staked_nodes,
        MAX_STAKED_CONNECTIONS,
        MAX_UNSTAKED_CONNECTIONS,
        1,
        DEFAULT_WAIT_FOR_CHUNK_TIMEOUT,
        tpu_coalesce,
    )
    .unwrap();

    info!("QUIC TPU server thread started");

    let main_thread = thread::Builder::new()
        .name("main_loop".to_string())
        .spawn(move || main_loop(packet_receiver))
        .expect("failed to spawn main thread");

    main_thread.join().expect("main_loop thread panicked");
    server_thread.join().expect("server thread panicked");
}
