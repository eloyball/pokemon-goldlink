use std::fs;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::net::UdpSocket;
use tokio::time::interval;

use coop_relay::protocol::{
    ClientToServerPacket, PlayerState, AUTO_PLAYER_ID, COOP_MAGIC, PROTOCOL_VERSION,
    ROOM_ACTION_CREATE, ROOM_ACTION_JOIN, ROOM_STATUS_OK,
};

const STRESS_ROOM: [u8; 6] = *b"STRESS";
const STRESS_HOST_TOKEN: u32 = 0xCAFE_BEEF;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Remove any previous STRESS.json save file so repeated runs start fresh
    let _ = fs::remove_file("campaigns/STRESS.json");

    let server_addr: SocketAddr = "127.0.0.1:7777".parse()?;
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

    // Have Client 0 create the room with target_party_size = 6 before joiners start
    let init_pkt = ClientToServerPacket {
        magic: COOP_MAGIC,
        version: PROTOCOL_VERSION,
        room_code: STRESS_ROOM,
        room_action: ROOM_ACTION_CREATE,
        requested_player_id: AUTO_PLAYER_ID,
        client_ready: 1,
        pad_header: 0,
        host_token: STRESS_HOST_TOKEN,
        badge_mask_local: 0,
        sequence: 1,
        heartbeat_rom: 1,
        reserved: 0,
        local_player: PlayerState {
            map_id: 4,
            x: 10,
            y: 20,
            prev_x: 10,
            prev_y: 19,
            direction: 0,
            movement_type: 1,
            sprite_id: 100,
            animation_frame: 0,
            is_active: 1,
            lead_species: 155,
            lead_form: 0,
            lead_is_shiny: 0,
            lead_gender: 0,
            map_npc_count: 8,
            client_ready: 1,
            trainer_id: 0x1000_0000,
            target_party_size: num_clients as u8,
            pad: [0u8; 3],
        },
    };
    sockets[0].send_to(&init_pkt.encode_le(), server_addr).await?;
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

            let action = if client_id == 0 {
                ROOM_ACTION_CREATE
            } else {
                ROOM_ACTION_JOIN
            };
            let token = if client_id == 0 { STRESS_HOST_TOKEN } else { 0 };

            while running_clone.load(Ordering::Relaxed) {
                tokio::select! {
                    _ = ticker.tick() => {
                        seq += 1;
                        send_timestamps.insert(seq, Instant::now());

                        let pkt = ClientToServerPacket {
                            magic: COOP_MAGIC,
                            version: PROTOCOL_VERSION,
                            room_code: STRESS_ROOM,
                            room_action: action,
                            requested_player_id: AUTO_PLAYER_ID,
                            client_ready: 1,
                            pad_header: 0,
                            host_token: token,
                            badge_mask_local: 0,
                            sequence: seq,
                            heartbeat_rom: seq * 2,
                            reserved: 0,
                            local_player: PlayerState {
                                map_id: 4,
                                x: 10 + (seq % 16) as i16,
                                y: 20,
                                prev_x: 10,
                                prev_y: 20,
                                direction: (seq % 4) as u8,
                                movement_type: 1,
                                sprite_id: 100 + client_id as u16,
                                animation_frame: 0,
                                is_active: 1,
                                lead_species: 155 + client_id as u16,
                                lead_form: 0,
                                lead_is_shiny: 0,
                                lead_gender: 0,
                                map_npc_count: 8,
                                client_ready: 1,
                                trainer_id: 0x1000_0000 + client_id as u32,
                                target_party_size: num_clients as u8,
                                pad: [0u8; 3],
                            },
                        };

                        let bytes = pkt.encode_le();
                        if socket.send_to(&bytes, server_addr).await.is_ok() {
                            total_sent_clone.fetch_add(1, Ordering::Relaxed);
                        }
                    }

                    Ok((len, _)) = socket.recv_from(&mut buf) => {
                        if len >= 320 && buf[12] == ROOM_STATUS_OK {
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

    assert!(sent > 0 && recv >= 1750, "All 6 clients must receive 30 Hz broadcasts");
    assert!(avg_rtt < 35.0, "Average RTT must stay within the 30 Hz tick window (~33.3ms)");

    Ok(())
}
