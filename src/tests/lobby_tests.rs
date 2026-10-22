use std::net::SocketAddr;
use std::thread::sleep;
use std::time::Duration;

use coop_relay::lobby::LobbyState;
use coop_relay::protocol::{
    ClientToServerPacket, PlayerState, AUTO_PLAYER_ID, COOP_MAGIC, DEFAULT_ROOM_CODE, MAX_PLAYERS,
    PROTOCOL_VERSION,
};

fn make_packet(
    player_id: u8,
    trainer_id: u32,
    ready: u8,
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
        room_code: DEFAULT_ROOM_CODE,
        requested_player_id: player_id,
        client_ready: ready,
        badge_mask_local: badges,
        sequence: 1,
        heartbeat_rom: 100,
        local_player: PlayerState {
            map_id,
            x,
            y,
            prev_x: x,
            prev_y: y.saturating_sub(1),
            direction: 0,
            movement_type: 0,
            sprite_id: 1,
            animation_frame: 0,
            is_active: 1,
            lead_species: species,
            lead_form: 0,
            lead_is_shiny: 0,
            lead_gender: 0,
            map_npc_count: npc_count,
            client_ready: ready,
            trainer_id,
            pad: [0u8; 4],
        },
    }
}

#[test]
fn test_bedroom_gate_locks_roster_and_blocks_progression_when_member_offline() {
    let mut lobby = LobbyState::new(Duration::from_millis(60), 2, false);

    let p0_addr: SocketAddr = "127.0.0.1:50000".parse().unwrap();
    let p1_addr: SocketAddr = "127.0.0.1:50001".parse().unwrap();
    let p2_stranger: SocketAddr = "127.0.0.1:50002".parse().unwrap();

    // 1. P0 and P1 spawn in Map 4 (Bedroom), but P1 is not ready yet (ready = 0)
    lobby.handle_client_packet(p0_addr, make_packet(0, 0x1111_1111, 1, 4, 5, 5, 0, 2, 0));
    lobby.handle_client_packet(p1_addr, make_packet(1, 0x2222_2222, 0, 4, 5, 5, 0, 2, 0));
    assert!(!lobby.is_campaign_started(), "Campaign must wait until all players click Ready");

    // 2. P1 clicks Ready -> Roster locks at 2 players, Bedroom Gate unlocks!
    lobby.handle_client_packet(p1_addr, make_packet(1, 0x2222_2222, 1, 4, 5, 5, 0, 2, 0));
    assert!(lobby.is_campaign_started(), "Campaign must start once all connected players are Ready");
    assert_eq!(lobby.expected_party_size(), 2);

    // 3. Unregistered 3rd player tries to join after campaign locked -> Rejected
    let stranger_slot = lobby.handle_client_packet(
        p2_stranger,
        make_packet(AUTO_PLAYER_ID, 0x9999_9999, 1, 4, 5, 5, 0, 2, 0),
    );
    assert_eq!(stranger_slot, None, "Locked campaign must reject unknown TrainerIDs");

    // 4. Both P0 and P1 walk to Violet Gym (Map 28) -> Gym Gate is UNLOCKED (1)
    lobby.handle_client_packet(p0_addr, make_packet(0, 0x1111_1111, 1, 28, 10, 10, 155, 5, 0));
    lobby.handle_client_packet(p1_addr, make_packet(1, 0x2222_2222, 1, 28, 11, 10, 158, 5, 0));
    let batch_online = lobby.build_broadcast_batch();
    let gates_offset = 16 + (5 * 32) + (5 * 24);
    assert_eq!(batch_online[0].1[gates_offset + 4], 1, "Gym gate unlocked when all 2/2 online");
    assert_eq!(batch_online[0].1[gates_offset + 8], 1, "flag_campaign_started == 1");

    // 5. P1 goes offline (times out), while P0 stays at the Gym
    sleep(Duration::from_millis(80));
    lobby.prune_stale_clients();
    lobby.handle_client_packet(p0_addr, make_packet(0, 0x1111_1111, 1, 28, 10, 10, 155, 5, 0));

    let batch_offline = lobby.build_broadcast_batch();
    assert_eq!(
        batch_offline[0].1[gates_offset + 4], 0,
        "Gym gate MUST lock when a registered party member (P1) is offline!"
    );

    // 6. P1 reconnects from a brand-new ephemeral port using their TrainerID (0x2222_2222)
    let p1_new_addr: SocketAddr = "127.0.0.1:61234".parse().unwrap();
    let reconnected_slot = lobby.handle_client_packet(
        p1_new_addr,
        make_packet(AUTO_PLAYER_ID, 0x2222_2222, 1, 28, 11, 10, 158, 5, 0),
    );
    assert_eq!(reconnected_slot, Some(1), "P1 must reclaim Slot P1 via TrainerID");

    let batch_rejoined = lobby.build_broadcast_batch();
    assert_eq!(
        batch_rejoined[0].1[gates_offset + 4], 1,
        "Gym gate unlocks again once P1 reconnects at the Gym"
    );
}

#[test]
fn test_auto_slot_assignment_and_saturation() {
    let mut lobby = LobbyState::new(Duration::from_secs(5), 6, false);
    for i in 0..MAX_PLAYERS {
        let addr: SocketAddr = format!("127.0.0.1:{}", 50000 + i).parse().unwrap();
        let pkt = make_packet(AUTO_PLAYER_ID, 1000 + i as u32, 0, 4, 10, 10, 152, 10, 0);
        assert_eq!(lobby.handle_client_packet(addr, pkt), Some(i as u8));
    }
    let overflow: SocketAddr = "127.0.0.1:59999".parse().unwrap();
    assert_eq!(
        lobby.handle_client_packet(overflow, make_packet(AUTO_PLAYER_ID, 9999, 0, 4, 10, 10, 152, 10, 0)),
        None
    );
}
