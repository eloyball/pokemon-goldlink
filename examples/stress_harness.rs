use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::net::UdpSocket;
use tokio::time::interval;

use coop_relay::protocol::{
    ClientToServerPacket, PlayerState, AUTO_PLAYER_ID, COOP_MAGIC, DEFAULT_ROOM_CODE,
    PROTOCOL_VERSION,
};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let server_addr: SocketAddr = "127.0.0.1:7777".parse()?;
    let duration_secs = 10;
    let num_clients = 6;

    println!("============================================================");
    println!("HGSS Co-op Relay Stress Test: {num_clients} clients @ 60 Hz for {duration_secs}s");
    println!("Target Server: {server_addr} | Room: JOHTO1");
    println!("============================================================");

    let running = Arc::new(AtomicBool::new(true));
    let total_sent = Arc::new(AtomicU64::new(0));
    let total_recv = Arc::new(AtomicU64::new(0));
    let total_rtt_us = Arc::new(AtomicU64::new(0));
    let max_rtt_us = Arc::new(AtomicU64::new(0));

    let mut handles = Vec::new();

    for client_id in 0..num_clients {
        let running_clone = Arc::clone(&running);
        let total_sent_clone = Arc::clone(&total_sent);
        let total_recv_clone = Arc::clone(&total_recv);
        let total_rtt_clone = Arc::clone(&total_rtt_us);
        let max_rtt_clone = Arc::clone(&max_rtt_us);

        let handle = tokio::spawn(async move {
            let socket = UdpSocket::bind("127.0.0.1:0").await.expect("bind client socket");
            let mut ticker = interval(Duration::from_micros(16_666)); // 60 Hz
            let mut seq = 0u32;
            let mut send_timestamps = std::collections::HashMap::new();
            let mut buf = [0u8; 512];

            while running_clone.load(Ordering::Relaxed) {
                tokio::select! {
                    _ = ticker.tick() => {
                        seq += 1;
                        send_timestamps.insert(seq, Instant::now());

                        let pkt = ClientToServerPacket {
                            magic: COOP_MAGIC,
                            version: PROTOCOL_VERSION,
                            room_code: DEFAULT_ROOM_CODE,
                            requested_player_id: AUTO_PLAYER_ID,
                            client_ready: 1,
                            badge_mask_local: 0,
                            sequence: seq,
                            heartbeat_rom: seq * 2,
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
                                pad: [0u8; 4],
                            },
                        };

                        let bytes = pkt.encode_le();
                        if socket.send_to(&bytes, server_addr).await.is_ok() {
                            total_sent_clone.fetch_add(1, Ordering::Relaxed);
                        }
                    }

                    Ok((len, _)) = socket.recv_from(&mut buf) => {
                        if len >= 320 {
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
    println!("Packets Sent (Clients -> Server)  : {sent} (expected ~{})", num_clients * 60 * duration_secs);
    println!("Packets Recv (Server -> Clients)  : {recv} (expected ~{})", num_clients * 30 * duration_secs);
    println!("Average Tick-Sync RTT             : {avg_rtt:.3} ms");
    println!("Maximum Spike RTT                 : {max_rtt:.3} ms");
    println!("------------------------------------------------------------");

    assert!(sent > 0 && recv > 0, "Traffic must flow in both directions");
    assert!(avg_rtt < 35.0, "Average RTT must stay within the 30 Hz broadcast tick window (~33.3ms)");

    Ok(())
}
