use std::fs;
use std::net::SocketAddr;
use std::thread::sleep;
use std::time::Duration;

use coop_relay::lobby::{LobbyState, RoomManager};
use coop_relay::protocol::{
    ClientToServerPacket, PlayerState, AUTO_PLAYER_ID, COOP_MAGIC, MAX_PLAYERS, PROTOCOL_VERSION,
    ROOM_ACTION_CREATE, ROOM_ACTION_JOIN, ROOM_ACTION_SYNC, ROOM_STATUS_ERR_ALREADY_EXISTS,
    ROOM_STATUS_ERR_FULL, ROOM_STATUS_ERR_INVALID_CODE, ROOM_STATUS_ERR_LOCKED,
    ROOM_STATUS_ERR_MIDGAME_SAVE, ROOM_STATUS_ERR_NOT_FOUND,
};

#[allow(clippy::too_many_arguments)]
fn make_packet(
    room_code: [u8; 6],
    room_action: u8,
    host_token: u32,
    player_id: u8,
    trainer_id: u32,
    target_party_size: u8,
    map_id: u16,
    x: i16,
    y: i16,
    species: u16,
    npc_count: u8,
    badges: u16,
) -> ClientToServerPacket {
    let ready = u8::from(map_id == 4 && trainer_id != 0);
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
fn test_safeguards_room_code_create_join_and_midgame_block() {
    let test_dir = "target/test_saves_safeguards";
    let _ = fs::remove_dir_all(test_dir);
    let mut manager = RoomManager::new(Duration::from_secs(5), 2, false, test_dir);

    let host_addr: SocketAddr = "127.0.0.1:51000".parse().unwrap();
    let second_addr: SocketAddr = "127.0.0.1:52000".parse().unwrap();

    // 1. Invalid room code (<6 alphanumeric chars) is rejected with ERR_INVALID_CODE
    let bad_code = make_packet(
        [b'A', b'B', b'C', 0, 0, 0],
        ROOM_ACTION_CREATE,
        0x1111,
        AUTO_PLAYER_ID,
        0,
        2,
        0,
        0,
        0,
        0,
        0,
        0,
    );
    assert_eq!(
        manager.handle_client_packet(host_addr, bad_code).unwrap_err()[12],
        ROOM_STATUS_ERR_INVALID_CODE
    );

    // 2. Joining a nonexistent room fails with ERR_NOT_FOUND
    let ghost_join = make_packet(
        *b"GHOST1",
        ROOM_ACTION_JOIN,
        0,
        AUTO_PLAYER_ID,
        0,
        2,
        0,
        0,
        0,
        0,
        0,
        0,
    );
    assert_eq!(
        manager
            .handle_client_packet(second_addr, ghost_join)
            .unwrap_err()[12],
        ROOM_STATUS_ERR_NOT_FOUND
    );

    // 3. Host creates room "SAFE01" at Title Screen (Map 0) with party_size = 2
    let create_ok = make_packet(
        *b"SAFE01",
        ROOM_ACTION_CREATE,
        0xAAAA_BBBB,
        AUTO_PLAYER_ID,
        0,
        2,
        0,
        0,
        0,
        0,
        0,
        0,
    );
    assert_eq!(manager.handle_client_packet(host_addr, create_ok), Ok(0));

    // 4. Clicking CREATE again with a new token (or from another client) ALWAYS fails with ERR_ALREADY_EXISTS
    let dup_create_same_addr = make_packet(
        *b"SAFE01",
        ROOM_ACTION_CREATE,
        0xCCCC_DDDD,
        AUTO_PLAYER_ID,
        0,
        2,
        0,
        0,
        0,
        0,
        0,
        0,
    );
    assert_eq!(
        manager
            .handle_client_packet(host_addr, dup_create_same_addr)
            .unwrap_err()[12],
        ROOM_STATUS_ERR_ALREADY_EXISTS
    );

    let dup_create_other_addr = make_packet(
        *b"SAFE01",
        ROOM_ACTION_CREATE,
        0xAAAA_BBBB,
        AUTO_PLAYER_ID,
        0,
        2,
        0,
        0,
        0,
        0,
        0,
        0,
    );
    assert_eq!(
        manager
            .handle_client_packet(second_addr, dup_create_other_addr)
            .unwrap_err()[12],
        ROOM_STATUS_ERR_ALREADY_EXISTS
    );

    // 5. Attempting to join an unstarted room with a mid-game save (Map 28, outside Bedroom) is rejected
    let midgame_join = make_packet(
        *b"SAFE01",
        ROOM_ACTION_JOIN,
        0,
        AUTO_PLAYER_ID,
        0x2222,
        2,
        28, // Violet City instead of Map 4 Bedroom
        10,
        10,
        158,
        5,
        0,
    );
    assert_eq!(
        manager
            .handle_client_packet(second_addr, midgame_join)
            .unwrap_err()[12],
        ROOM_STATUS_ERR_MIDGAME_SAVE
    );

    let _ = fs::remove_dir_all(test_dir);
}

#[test]
fn test_auto_bedroom_lock_and_offline_progression_block() {
    let mut lobby = LobbyState::with_room(
        *b"JOHTO1",
        0x1234_5678,
        None,
        Duration::from_millis(60),
        2,
        false,
        None,
    );

    let p0_addr: SocketAddr = "127.0.0.1:50000".parse().unwrap();
    let p1_addr: SocketAddr = "127.0.0.1:50001".parse().unwrap();
    let p2_stranger: SocketAddr = "127.0.0.1:50002".parse().unwrap();

    // 1. P0 and P1 connect at Title Screen (Map 0, OTID 0) -> Campaign does NOT start yet
    assert_eq!(
        lobby.handle_client_packet(
            p0_addr,
            make_packet(*b"JOHTO1", ROOM_ACTION_CREATE, 0x1234_5678, 0, 0, 2, 0, 0, 0, 0, 0, 0)
        ),
        Ok(0)
    );
    assert_eq!(
        lobby.handle_client_packet(
            p1_addr,
            make_packet(*b"JOHTO1", ROOM_ACTION_JOIN, 0, 1, 0, 2, 0, 0, 0, 0, 0, 0)
        ),
        Ok(1)
    );
    assert!(!lobby.is_campaign_started());

    // 2. P0 finishes Oak's intro and enters Bedroom (Map 4, OTID 0x1111_1111), P1 still with Oak (Map 0)
    assert_eq!(
        lobby.handle_client_packet(
            p0_addr,
            make_packet(*b"JOHTO1", ROOM_ACTION_SYNC, 0x1234_5678, 0, 0x1111_1111, 2, 4, 5, 5, 0, 2, 0)
        ),
        Ok(0)
    );
    assert!(!lobby.is_campaign_started());

    // 3. P1 finishes Oak's intro and enters Bedroom (Map 4, OTID 0x2222_2222) -> Automatically locks & starts!
    assert_eq!(
        lobby.handle_client_packet(
            p1_addr,
            make_packet(*b"JOHTO1", ROOM_ACTION_SYNC, 0, 1, 0x2222_2222, 2, 4, 5, 5, 0, 2, 0)
        ),
        Ok(1)
    );
    assert!(lobby.is_campaign_started());

    // 4. Stranger with unknown OTID tries joining after lock -> Rejected with ERR_LOCKED
    assert_eq!(
        lobby.handle_client_packet(
            p2_stranger,
            make_packet(*b"JOHTO1", ROOM_ACTION_JOIN, 0, AUTO_PLAYER_ID, 0x9999_9999, 2, 4, 5, 5, 0, 2, 0)
        ),
        Err(ROOM_STATUS_ERR_LOCKED)
    );

    // 5. Both walk out of house to Violet Gym (Map 28) -> Gym Gate UNLOCKED (1)
    let _ = lobby.handle_client_packet(
        p0_addr,
        make_packet(*b"JOHTO1", ROOM_ACTION_SYNC, 0x1234_5678, 0, 0x1111_1111, 2, 28, 10, 10, 155, 5, 0),
    );
    let _ = lobby.handle_client_packet(
        p1_addr,
        make_packet(*b"JOHTO1", ROOM_ACTION_SYNC, 0, 1, 0x2222_2222, 2, 28, 11, 10, 158, 5, 0),
    );
    let batch_online = lobby.build_broadcast_batch();
    let gates_offset = 16 + (5 * 32) + (5 * 24);
    assert_eq!(batch_online[0].1[gates_offset + 4], 1);

    // 6. P1 disconnects -> Gym Gate locks (0)
    sleep(Duration::from_millis(80));
    lobby.prune_stale_clients();
    let batch_offline = lobby.build_broadcast_batch();
    assert_eq!(batch_offline[0].1[gates_offset + 4], 0);

    // 7. P1 reconnects at Title Screen (Map 0, OTID 0) and then loads save (OTID 0x2222_2222)
    let p1_new_addr: SocketAddr = "127.0.0.1:61234".parse().unwrap();
    assert_eq!(
        lobby.handle_client_packet(
            p1_new_addr,
            make_packet(*b"JOHTO1", ROOM_ACTION_JOIN, 0, AUTO_PLAYER_ID, 0, 2, 0, 0, 0, 0, 0, 0)
        ),
        Ok(1)
    );
    assert_eq!(
        lobby.handle_client_packet(
            p1_new_addr,
            make_packet(*b"JOHTO1", ROOM_ACTION_SYNC, 0, AUTO_PLAYER_ID, 0x2222_2222, 2, 28, 11, 10, 158, 5, 0)
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
            *b"TEST01",
            ROOM_ACTION_JOIN,
            0,
            AUTO_PLAYER_ID,
            0, // Title screen before bedroom lock
            6,
            0,
            0,
            0,
            0,
            0,
            0,
        );
        assert_eq!(lobby.handle_client_packet(addr, pkt), Ok(i as u8));
    }

    let overflow: SocketAddr = "127.0.0.1:59999".parse().unwrap();
    let overflow_pkt = make_packet(
        *b"TEST01",
        ROOM_ACTION_JOIN,
        0,
        AUTO_PLAYER_ID,
        0,
        6,
        0,
        0,
        0,
        0,
        0,
        0,
    );
    assert_eq!(
        lobby.handle_client_packet(overflow, overflow_pkt),
        Err(ROOM_STATUS_ERR_FULL)
    );
}
