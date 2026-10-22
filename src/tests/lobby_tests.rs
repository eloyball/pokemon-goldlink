use std::fs;
use std::net::SocketAddr;
use std::thread::sleep;
use std::time::Duration;

use coop_relay::lobby::{LobbyState, RoomManager};
use coop_relay::protocol::{
    ClientToServerPacket, PlayerState, AUTO_PLAYER_ID, COOP_MAGIC, DEFAULT_ROOM_CODE, MAX_PLAYERS,
    PROTOCOL_VERSION, ROOM_ACTION_CREATE, ROOM_ACTION_JOIN, ROOM_STATUS_ERR_ALREADY_EXISTS,
    ROOM_STATUS_ERR_FULL, ROOM_STATUS_ERR_LOCKED, ROOM_STATUS_ERR_NOT_FOUND,
};

#[allow(clippy::too_many_arguments)]
fn make_packet(
    room_code: [u8; 6],
    room_action: u8,
    host_token: u32,
    player_id: u8,
    trainer_id: u32,
    target_party_size: u8,
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
        room_code,
        room_action,
        requested_player_id: player_id,
        client_ready: ready,
        pad_header: 0,
        host_token,
        badge_mask_local: badges,
        sequence: 1,
        heartbeat_rom: 100,
        reserved: 0,
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
            target_party_size,
            pad: [0u8; 3],
        },
    }
}

#[test]
fn test_safeguard_duplicate_room_create_and_nonexistent_join() {
    let test_dir = "target/test_saves_safeguards";
    let _ = fs::remove_dir_all(test_dir);
    let mut manager = RoomManager::new(Duration::from_secs(5), 2, false, test_dir);

    let host_addr: SocketAddr = "127.0.0.1:51000".parse().unwrap();
    let intruder_addr: SocketAddr = "127.0.0.1:52000".parse().unwrap();
    let joiner_addr: SocketAddr = "127.0.0.1:53000".parse().unwrap();

    // 1. Joining a room that hasn't been created yet fails with ERR_NOT_FOUND
    let ghost_join = make_packet(
        *b"GHOST1",
        ROOM_ACTION_JOIN,
        0,
        AUTO_PLAYER_ID,
        0x3333,
        2,
        0,
        4,
        10,
        10,
        155,
        5,
        0,
    );
    let err_not_found = manager.handle_client_packet(joiner_addr, ghost_join).unwrap_err();
    assert_eq!(err_not_found[12], ROOM_STATUS_ERR_NOT_FOUND);

    // 2. Host creates room "SAFE01" with host_token 0xAAAA_BBBB and party size 3
    let create_ok = make_packet(
        *b"SAFE01",
        ROOM_ACTION_CREATE,
        0xAAAA_BBBB,
        AUTO_PLAYER_ID,
        0x1111,
        3,
        1,
        4,
        10,
        10,
        155,
        5,
        0,
    );
    assert_eq!(manager.handle_client_packet(host_addr, create_ok), Ok(0));

    // 3. Another user tries to CREATE "SAFE01" with a different host_token -> Rejected with ERR_ALREADY_EXISTS
    let duplicate_create = make_packet(
        *b"SAFE01",
        ROOM_ACTION_CREATE,
        0xDEAD_BEEF,
        AUTO_PLAYER_ID,
        0x2222,
        2,
        1,
        4,
        10,
        10,
        158,
        5,
        0,
    );
    let err_exists = manager
        .handle_client_packet(intruder_addr, duplicate_create)
        .unwrap_err();
    assert_eq!(err_exists[12], ROOM_STATUS_ERR_ALREADY_EXISTS);

    // 4. Legitimate friend joins "SAFE01" with ROOM_ACTION_JOIN -> Assigned Slot P1
    let valid_join = make_packet(
        *b"SAFE01",
        ROOM_ACTION_JOIN,
        0,
        AUTO_PLAYER_ID,
        0x2222,
        2, // Non-host tries passing party_size=2, server must ignore and keep host's 3
        1,
        4,
        10,
        10,
        158,
        5,
        0,
    );
    assert_eq!(manager.handle_client_packet(joiner_addr, valid_join), Ok(1));

    let _ = fs::remove_dir_all(test_dir);
}

#[test]
fn test_bedroom_gate_roster_lock_and_offline_progression_block() {
    let mut lobby = LobbyState::with_room(
        DEFAULT_ROOM_CODE,
        0x1234_5678,
        Duration::from_millis(60),
        2,
        false,
        None,
    );

    let p0_addr: SocketAddr = "127.0.0.1:50000".parse().unwrap();
    let p1_addr: SocketAddr = "127.0.0.1:50001".parse().unwrap();
    let p2_stranger: SocketAddr = "127.0.0.1:50002".parse().unwrap();

    // 1. P0 (Host) and P1 spawn in Map 4 (Bedroom), P1 not ready yet
    assert_eq!(
        lobby.handle_client_packet(
            p0_addr,
            make_packet(DEFAULT_ROOM_CODE, ROOM_ACTION_CREATE, 0x1234_5678, 0, 0x1111_1111, 2, 1, 4, 5, 5, 0, 2, 0)
        ),
        Ok(0)
    );
    assert_eq!(
        lobby.handle_client_packet(
            p1_addr,
            make_packet(DEFAULT_ROOM_CODE, ROOM_ACTION_JOIN, 0, 1, 0x2222_2222, 2, 0, 4, 5, 5, 0, 2, 0)
        ),
        Ok(1)
    );
    assert!(!lobby.is_campaign_started());

    // 2. P1 checks Ready -> Roster locks at 2 players!
    assert_eq!(
        lobby.handle_client_packet(
            p1_addr,
            make_packet(DEFAULT_ROOM_CODE, ROOM_ACTION_JOIN, 0, 1, 0x2222_2222, 2, 1, 4, 5, 5, 0, 2, 0)
        ),
        Ok(1)
    );
    assert!(lobby.is_campaign_started());
    assert_eq!(lobby.expected_party_size(), 2);

    // 3. Stranger tries joining after roster lock -> Rejected with ROOM_STATUS_ERR_LOCKED
    assert_eq!(
        lobby.handle_client_packet(
            p2_stranger,
            make_packet(DEFAULT_ROOM_CODE, ROOM_ACTION_JOIN, 0, AUTO_PLAYER_ID, 0x9999_9999, 2, 1, 4, 5, 5, 0, 2, 0)
        ),
        Err(ROOM_STATUS_ERR_LOCKED)
    );

    // 4. Both P0 and P1 walk to Violet Gym (Map 28) -> Gym Gate UNLOCKED (1)
    let _ = lobby.handle_client_packet(
        p0_addr,
        make_packet(DEFAULT_ROOM_CODE, ROOM_ACTION_CREATE, 0x1234_5678, 0, 0x1111_1111, 2, 1, 28, 10, 10, 155, 5, 0),
    );
    let _ = lobby.handle_client_packet(
        p1_addr,
        make_packet(DEFAULT_ROOM_CODE, ROOM_ACTION_JOIN, 0, 1, 0x2222_2222, 2, 1, 28, 11, 10, 158, 5, 0),
    );
    let batch_online = lobby.build_broadcast_batch();
    let gates_offset = 16 + (5 * 32) + (5 * 24);
    assert_eq!(batch_online[0].1[gates_offset + 4], 1);

    // 5. P1 disconnects -> Gym Gate locks (0) for P0
    sleep(Duration::from_millis(80));
    lobby.prune_stale_clients();
    let _ = lobby.handle_client_packet(
        p0_addr,
        make_packet(DEFAULT_ROOM_CODE, ROOM_ACTION_CREATE, 0x1234_5678, 0, 0x1111_1111, 2, 1, 28, 10, 10, 155, 5, 0),
    );
    let batch_offline = lobby.build_broadcast_batch();
    assert_eq!(batch_offline[0].1[gates_offset + 4], 0);

    // 6. P1 reconnects from a new port using their TrainerID (0x2222_2222) -> Reclaims Slot P1
    let p1_new_addr: SocketAddr = "127.0.0.1:61234".parse().unwrap();
    assert_eq!(
        lobby.handle_client_packet(
            p1_new_addr,
            make_packet(DEFAULT_ROOM_CODE, ROOM_ACTION_JOIN, 0, AUTO_PLAYER_ID, 0x2222_2222, 2, 1, 28, 11, 10, 158, 5, 0)
        ),
        Ok(1)
    );
    let batch_rejoined = lobby.build_broadcast_batch();
    assert_eq!(batch_rejoined[0].1[gates_offset + 4], 1);
}

#[test]
fn test_auto_slot_assignment_and_saturation() {
    let mut lobby = LobbyState::new(Duration::from_secs(5), MAX_PLAYERS, false);
    for i in 0..MAX_PLAYERS {
        let addr: SocketAddr = format!("127.0.0.1:{}", 50000 + i).parse().unwrap();
        let pkt = make_packet(
            DEFAULT_ROOM_CODE,
            ROOM_ACTION_JOIN,
            0,
            AUTO_PLAYER_ID,
            1000 + i as u32,
            6,
            0,
            4,
            10,
            10,
            152,
            10,
            0,
        );
        assert_eq!(lobby.handle_client_packet(addr, pkt), Ok(i as u8));
    }

    let overflow: SocketAddr = "127.0.0.1:59999".parse().unwrap();
    let overflow_pkt = make_packet(
        DEFAULT_ROOM_CODE,
        ROOM_ACTION_JOIN,
        0,
        AUTO_PLAYER_ID,
        9999,
        6,
        0,
        4,
        10,
        10,
        152,
        10,
        0,
    );
    assert_eq!(
        lobby.handle_client_packet(overflow, overflow_pkt),
        Err(ROOM_STATUS_ERR_FULL)
    );
}
