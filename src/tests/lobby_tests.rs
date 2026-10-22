use std::net::SocketAddr;
use std::thread::sleep;
use std::time::Duration;

use coop_relay::lobby::LobbyState;
use coop_relay::protocol::{
    ClientToServerPacket, PlayerState, AUTO_PLAYER_ID, COOP_MAGIC, MAX_PLAYERS, PROTOCOL_VERSION,
};

fn make_dummy_packet(
    player_id: u8,
    map_id: u16,
    x: i16,
    y: i16,
    species: u16,
    npc_count: u8,
    badges: u16,
) -> ClientToServerPacket {
    ClientToServerPacket {
        magic: COOP_MAGIC,
        version: PROTOCOL_VERSION,
        requested_player_id: player_id,
        map_npc_count: npc_count,
        sequence: 1,
        heartbeat_rom: 100,
        local_player: PlayerState {
            map_id,
            x,
            y,
            prev_x: x,
            prev_y: y - 1,
            direction: 0,
            movement_type: 0,
            sprite_id: 1,
            animation_frame: 0,
            is_active: 1,
            pad: [0u8; 16],
        },
        lead_species: species,
        lead_form: 0,
        lead_is_shiny: 0,
        lead_gender: 0,
        badge_mask_local: badges,
    }
}

#[test]
fn test_auto_slot_assignment_and_saturation() {
    let mut lobby = LobbyState::new(Duration::from_secs(5), 2, false);

    // 1. Connect 6 distinct clients with AUTO_PLAYER_ID
    for i in 0..MAX_PLAYERS {
        let addr: SocketAddr = format!("127.0.0.1:{}", 50000 + i).parse().unwrap();
        let pkt = make_dummy_packet(AUTO_PLAYER_ID, 4, 10, 10, 152, 10, 0);
        let assigned = lobby.handle_client_packet(addr, pkt);
        assert_eq!(assigned, Some(i as u8), "Expected slot P{i}");
    }

    // 2. Attempt 7th client connection -> Must be rejected (None)
    let overflow_addr: SocketAddr = "127.0.0.1:59999".parse().unwrap();
    let pkt = make_dummy_packet(AUTO_PLAYER_ID, 4, 10, 10, 152, 10, 0);
    assert_eq!(
        lobby.handle_client_packet(overflow_addr, pkt),
        None,
        "7th connection must fail when lobby is full"
    );
}

#[test]
fn test_room_filtering_and_follower_culling() {
    let mut lobby = LobbyState::new(Duration::from_secs(5), 2, false);

    let p0_addr: SocketAddr = "127.0.0.1:50000".parse().unwrap();
    let p1_addr: SocketAddr = "127.0.0.1:50001".parse().unwrap();
    let p2_addr: SocketAddr = "127.0.0.1:50002".parse().unwrap();

    // P0: Map 4 (New Bark Town), 10 NPCs
    lobby.handle_client_packet(p0_addr, make_dummy_packet(0, 4, 12, 14, 155, 10, 0));
    // P1: Map 4 (New Bark Town), same room, 10 NPCs
    lobby.handle_client_packet(p1_addr, make_dummy_packet(1, 4, 15, 14, 158, 10, 0));
    // P2: Map 28 (Violet City), different room
    lobby.handle_client_packet(p2_addr, make_dummy_packet(2, 28, 5, 5, 152, 10, 0));

    let batch = lobby.build_broadcast_batch();
    assert_eq!(batch.len(), 3);

    // Inspect P0's broadcast packet
    let (addr, p0_raw) = &batch[0];
    assert_eq!(*addr, p0_addr);

    // Remote slot 0 in P0's packet corresponds to P1 (same map) -> is_active == 1, is_visible == 1
    let p1_active = p0_raw[16 + 15]; // offset 15 in PlayerState is is_active
    let p1_follower_visible = p0_raw[16 + (5 * 32) + 11]; // offset 11 in FollowerState is is_visible
    assert_eq!(p1_active, 1, "P1 should be active on P0 screen (same map)");
    assert_eq!(p1_follower_visible, 1, "P1 follower should be visible (<26 NPCs)");

    // Remote slot 1 in P0's packet corresponds to P2 (different map) -> is_active == 0, is_visible == 0
    let p2_active = p0_raw[16 + 32 + 15];
    let p2_follower_visible = p0_raw[16 + (5 * 32) + 24 + 11];
    assert_eq!(p2_active, 0, "P2 should be inactive on P0 screen (different map)");
    assert_eq!(p2_follower_visible, 0, "P2 follower should be culled (different map)");

    // Now simulate P0 entering a crowded city (>26 NPCs)
    lobby.handle_client_packet(p0_addr, make_dummy_packet(0, 4, 12, 14, 155, 27, 0));
    let batch_culled = lobby.build_broadcast_batch();
    let (_, p0_raw_culled) = &batch_culled[0];
    let p1_culled_visible = p0_raw_culled[16 + (5 * 32) + 11];
    assert_eq!(
        p1_culled_visible, 0,
        "P1 follower must cull when local map NPC count exceeds 26"
    );
}

#[test]
fn test_progression_sync_barrier_gate() {
    let mut lobby = LobbyState::new(Duration::from_secs(5), 2, false);

    let p0_addr: SocketAddr = "127.0.0.1:50000".parse().unwrap();
    let p1_addr: SocketAddr = "127.0.0.1:50001".parse().unwrap();

    // P0 and P1 both at Violet City (Map 28) with 0 badges
    lobby.handle_client_packet(p0_addr, make_dummy_packet(0, 28, 10, 10, 155, 5, 0b0000_0000));
    lobby.handle_client_packet(p1_addr, make_dummy_packet(1, 28, 11, 10, 158, 5, 0b0000_0000));

    let batch = lobby.build_broadcast_batch();
    let gates_offset = 16 + (5 * 32) + (5 * 24);
    let sync_ready = batch[0].1[gates_offset + 4];
    assert_eq!(sync_ready, 1, "Barrier gate must unlock when all players match map and badges");

    // Desync: P0 beats Falkner and gets Badge 1, P1 has not beaten him yet
    lobby.handle_client_packet(p0_addr, make_dummy_packet(0, 28, 10, 10, 155, 5, 0b0000_0001));
    let batch_desync = lobby.build_broadcast_batch();
    let sync_ready_desync = batch_desync[0].1[gates_offset + 4];
    let synced_badges = u16::from_le_bytes([batch_desync[0].1[gates_offset + 2], batch_desync[0].1[gates_offset + 3]]);

    assert_eq!(sync_ready_desync, 0, "Barrier gate must lock when badges desync");
    assert_eq!(synced_badges, 0, "Common badge mask must reflect lowest common denominator");
}

#[test]
fn test_client_timeout_pruning() {
    let mut lobby = LobbyState::new(Duration::from_millis(50), 2, false);
    let p0_addr: SocketAddr = "127.0.0.1:50000".parse().unwrap();

    lobby.handle_client_packet(p0_addr, make_dummy_packet(0, 4, 10, 10, 155, 5, 0));
    assert_eq!(lobby.build_broadcast_batch().len(), 1);

    // Sleep past the 50ms timeout window
    sleep(Duration::from_millis(70));
    lobby.prune_stale_clients();

    assert_eq!(
        lobby.build_broadcast_batch().len(),
        0,
        "Stale client must be pruned after timeout"
    );
}
