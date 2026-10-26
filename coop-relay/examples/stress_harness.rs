use std::fs;
use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};

use tokio::net::UdpSocket;
use tokio::time::interval;

use coop_relay::protocol::{
    AUTO_PLAYER_ID, COOP_MAGIC, ClientToServerPacket, PROTOCOL_VERSION, PlayerState,
    ROOM_ACTION_CREATE, ROOM_ACTION_JOIN, ROOM_ACTION_SYNC, ROOM_STATUS_CHALLENGE, ROOM_STATUS_OK,
    SAVE_STATUS_FRESH, SERVER_PACKET_SIZE,
};

/// Cookie carried by a challenge reply (status 7, bytes 16..24).
fn challenge_cookie(reply: &[u8]) -> Option<u64> {
    (reply.len() >= 24 && reply[12] == ROOM_STATUS_CHALLENGE)
        .then(|| u64::from_le_bytes(reply[16..24].try_into().expect("8 bytes")))
}

fn player_secret(client_id: usize) -> u64 {
    0x57E5_0000_0000_0000 + client_id as u64
}

const STRESS_ROOM: [u8; 6] = *b"STRESS";
const STRESS_HOST_TOKEN: u32 = 0xCAFE_BEEF;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let _ = fs::remove_file("campaigns/STRESS.json");

    // Override with COOP_RELAY_ADDR to target a relay on another address.
    let server_addr: SocketAddr = std::env::var("COOP_RELAY_ADDR")
        .unwrap_or_else(|_| "127.0.0.1:7777".to_string())
        .parse()?;
    let duration_secs = 10;
    let num_clients: usize = 6;

    println!("============================================================");
    println!("HGSS Co-op Relay Stress Test: {num_clients} clients @ 60 Hz for {duration_secs}s");
    println!("Target Server: {server_addr} | Room: STRESS");
    println!("============================================================");

    let running = Arc::new(AtomicBool::new(true));
    let total_sent = Arc::new(AtomicU64::new(0));
    let total_recv = Arc::new(AtomicU64::new(0));
    let total_rtt_us = Arc::new(AtomicU64::new(0));
    let max_rtt_us = Arc::new(AtomicU64::new(0));

    let mut sockets = Vec::with_capacity(num_clients);
    for _ in 0..num_clients {
        sockets.push(UdpSocket::bind("127.0.0.1:0").await?);
    }

    // 1. Client 0 creates the room with party_size = 6 in Bedroom (Map 4)
    let mut init_pkt = ClientToServerPacket {
        magic: COOP_MAGIC,
        version: PROTOCOL_VERSION,
        room_code: STRESS_ROOM,
        room_action: ROOM_ACTION_CREATE,
        requested_player_id: AUTO_PLAYER_ID,
        allow_ahead: 0,
        target_party_size: num_clients as u8,
        host_token: STRESS_HOST_TOKEN,
        badge_mask_local: 0,
        sequence: 1,
        heartbeat_rom: 1,
        reserved: 0,
        local_player: PlayerState {
            map_id: 4,
            x: 10,
            z: 20,
            sprite_id: 100,
            is_active: 1,
            client_ready: 1,
            save_status: SAVE_STATUS_FRESH,
            activity: 1,
            trainer_id: 0x1000_0000,
            follower_species: 155,
            follower_visible: 1,
            ..PlayerState::default()
        },
        cookie: 0,
        player_secret: player_secret(0),
        session: Default::default(),
    };
    // Cookie handshake: the first send is challenged, the echo creates the room.
    let mut reply = [0u8; 512];
    for _ in 0..2 {
        sockets[0]
            .send_to(&init_pkt.encode_le(), server_addr)
            .await?;
        let (len, _) =
            tokio::time::timeout(Duration::from_secs(1), sockets[0].recv_from(&mut reply))
                .await??;
        if let Some(cookie) = challenge_cookie(&reply[..len]) {
            init_pkt.cookie = cookie;
        }
    }
    tokio::time::sleep(Duration::from_millis(20)).await;

    let mut handles = Vec::new();

    for (client_id, socket) in sockets.into_iter().enumerate() {
        let running_clone = Arc::clone(&running);
        let total_sent_clone = Arc::clone(&total_sent);
        let total_recv_clone = Arc::clone(&total_recv);
        let total_rtt_clone = Arc::clone(&total_rtt_us);
        let max_rtt_clone = Arc::clone(&max_rtt_us);

        let handle = tokio::spawn(async move {
            let mut ticker = interval(Duration::from_micros(16_666)); // 60 Hz
            let mut seq = 1u32;
            let mut send_timestamps = std::collections::HashMap::new();
            let mut buf = [0u8; 512];

            let mut current_action = if client_id == 0 {
                ROOM_ACTION_SYNC
            } else {
                ROOM_ACTION_JOIN
            };
            let token = if client_id == 0 { STRESS_HOST_TOKEN } else { 0 };
            let mut cookie = 0u64;

            while running_clone.load(Ordering::Relaxed) {
                tokio::select! {
                    _ = ticker.tick() => {
                        seq += 1;
                        send_timestamps.insert(seq, Instant::now());

                        let pkt = ClientToServerPacket {
                            magic: COOP_MAGIC,
                            version: PROTOCOL_VERSION,
                            room_code: STRESS_ROOM,
                            room_action: current_action,
                            requested_player_id: AUTO_PLAYER_ID,
                            allow_ahead: 0,
                            target_party_size: num_clients as u8,
                            host_token: token,
                            badge_mask_local: 0,
                            sequence: seq,
                            heartbeat_rom: seq * 2,
                            reserved: 0,
                            local_player: PlayerState {
                                map_id: 4,
                                x: 10 + (seq % 16) as i16,
                                z: 20,
                                direction: (seq % 4) as u8,
                                sprite_id: 100 + client_id as u16,
                                move_seq: seq as u8,
                                is_active: 1,
                                client_ready: 1,
                                save_status: SAVE_STATUS_FRESH,
                                activity: 1,
                                trainer_id: 0x1000_0000 + client_id as u32,
                                follower_species: 155 + client_id as u16,
                                follower_visible: 1,
                                ..PlayerState::default()
                            },
                            cookie,
                            player_secret: player_secret(client_id),
                            session: Default::default(),
                        };

                        let bytes = pkt.encode_le();
                        if socket.send_to(&bytes, server_addr).await.is_ok() {
                            total_sent_clone.fetch_add(1, Ordering::Relaxed);
                        }
                    }

                    Ok((len, _)) = socket.recv_from(&mut buf) => {
                        if let Some(issued) = challenge_cookie(&buf[..len]) {
                            cookie = issued;
                        } else if len >= SERVER_PACKET_SIZE && buf[12] == ROOM_STATUS_OK {
                            current_action = ROOM_ACTION_SYNC;
                            total_recv_clone.fetch_add(1, Ordering::Relaxed);
                            if let Some(&sent_time) = send_timestamps.get(&seq) {
                                let rtt = sent_time.elapsed().as_micros() as u64;
                                total_rtt_clone.fetch_add(rtt, Ordering::Relaxed);
                                max_rtt_clone.fetch_max(rtt, Ordering::Relaxed);
                            }
                        }
                    }
                }
            }
        });
        handles.push(handle);
    }

    tokio::time::sleep(Duration::from_secs(duration_secs)).await;
    running.store(false, Ordering::Relaxed);

    for h in handles {
        let _ = h.await;
    }

    let sent = total_sent.load(Ordering::Relaxed);
    let recv = total_recv.load(Ordering::Relaxed);
    let avg_rtt = if recv > 0 {
        total_rtt_us.load(Ordering::Relaxed) as f64 / recv as f64 / 1000.0
    } else {
        0.0
    };
    let max_rtt = max_rtt_us.load(Ordering::Relaxed) as f64 / 1000.0;

    println!("------------------------------------------------------------");
    println!("Test Finished.");
    println!(
        "Packets Sent (Clients -> Server)  : {sent} (expected ~{})",
        num_clients as u64 * 60 * duration_secs
    );
    println!(
        "Packets Recv (Server -> Clients)  : {recv} (expected ~{})",
        num_clients as u64 * 30 * duration_secs
    );
    println!("Average Tick-Sync RTT             : {avg_rtt:.3} ms");
    println!("Maximum Spike RTT                 : {max_rtt:.3} ms");
    println!("------------------------------------------------------------");

    assert!(
        sent > 0 && recv >= 1750,
        "All 6 clients must receive 30 Hz broadcasts"
    );
    assert!(
        avg_rtt < 35.0,
        "Average RTT must stay within the 30 Hz tick window (~33.3ms)"
    );

    Ok(())
}
