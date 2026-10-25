use std::fs;
use std::net::SocketAddr;
use std::thread::sleep;
use std::time::Duration;

use crate::lobby::{CampaignLoadError, LobbyState, Rejection, RoomManager};
use crate::protocol::{
    AUTO_PLAYER_ID, COOP_MAGIC, ClientToServerPacket, MAX_PLAYERS, PROTOCOL_VERSION, PlayerState,
    ROOM_ACTION_CREATE, ROOM_ACTION_JOIN, ROOM_ACTION_LEAVE, ROOM_ACTION_SYNC,
    ROOM_STATUS_ERR_ALREADY_EXISTS, ROOM_STATUS_ERR_FULL, ROOM_STATUS_ERR_INVALID_CODE,
    ROOM_STATUS_ERR_LOCKED, ROOM_STATUS_ERR_NOT_FOUND, ROOM_STATUS_ERR_RATE_LIMITED,
};

/// Sends like a real client that already completed the cookie handshake.
fn send(
    manager: &mut RoomManager,
    addr: SocketAddr,
    mut pkt: ClientToServerPacket,
) -> Result<u8, u8> {
    pkt.cookie = manager.cookie_for(addr);
    manager
        .handle_client_packet(addr, pkt)
        .map_err(|rejection| match rejection {
            Rejection::Status(status) => status,
            Rejection::Challenge(_) => panic!("valid cookie was challenged"),
        })
}

/// Test identity of a bridge install: tied to the OTID once a save is loaded,
/// a shared title-screen value before that (title-screen guests are anonymous).
fn secret_for(trainer_id: u32) -> u64 {
    if trainer_id == 0 {
        0x7171_7171_0000_0000
    } else {
        0x5EC0_0000_0000_0000 | u64::from(trainer_id)
    }
}

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
    // Readiness is computed by the ROM; the relay just relays local_player.client_ready.
    let ready = u8::from(trainer_id != 0);
    ClientToServerPacket {
        magic: COOP_MAGIC,
        version: PROTOCOL_VERSION,
        room_code,
        room_action,
        requested_player_id: player_id,
        reserved_14: 0,
        target_party_size,
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
            pad: [0u8; 4],
        },
        cookie: 0,
        player_secret: secret_for(trainer_id),
    }
}

#[test]
fn test_safeguards_room_code_create_join_and_open_admission() {
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
        send(&mut manager, host_addr, bad_code).unwrap_err(),
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
        send(&mut manager, second_addr, ghost_join).unwrap_err(),
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
    assert_eq!(send(&mut manager, host_addr, create_ok), Ok(0));

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
        send(&mut manager, host_addr, dup_create_same_addr).unwrap_err(),
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
        send(&mut manager, second_addr, dup_create_other_addr).unwrap_err(),
        ROOM_STATUS_ERR_ALREADY_EXISTS
    );

    // 5. Joining an unstarted room with a mid-game save (Map 28) is now ALLOWED:
    //    the relay is game-agnostic and trusts the client's client_ready wire flag
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
    assert_eq!(send(&mut manager, second_addr, midgame_join), Ok(1));

    let _ = fs::remove_dir_all(test_dir);
}

#[test]
fn test_auto_start_lock_and_offline_progression_block() {
    let mut lobby = LobbyState::with_room(
        *b"JOHTO1",
        0x1234_5678,
        None,
        Duration::from_millis(60),
        2,
        false,
        None,
    )
    .unwrap();

    let p0_addr: SocketAddr = "127.0.0.1:50000".parse().unwrap();
    let p1_addr: SocketAddr = "127.0.0.1:50001".parse().unwrap();
    let p2_stranger: SocketAddr = "127.0.0.1:50002".parse().unwrap();

    // 1. P0 and P1 connect at Title Screen (Map 0, OTID 0) -> Campaign does NOT start yet
    assert_eq!(
        lobby.handle_client_packet(
            p0_addr,
            make_packet(
                *b"JOHTO1",
                ROOM_ACTION_CREATE,
                0x1234_5678,
                0,
                0,
                2,
                0,
                0,
                0,
                0,
                0,
                0
            )
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

    // 2. P0 loads save (Map 4, OTID 0x1111_1111) and reports client_ready=1; P1 still at Title
    assert_eq!(
        lobby.handle_client_packet(
            p0_addr,
            make_packet(
                *b"JOHTO1",
                ROOM_ACTION_SYNC,
                0x1234_5678,
                0,
                0x1111_1111,
                2,
                4,
                5,
                5,
                0,
                2,
                0
            )
        ),
        Ok(0)
    );
    assert!(!lobby.is_campaign_started());

    // 3. P1 loads save (Map 4, OTID 0x2222_2222) and reports client_ready=1 -> Automatically locks & starts!
    assert_eq!(
        lobby.handle_client_packet(
            p1_addr,
            make_packet(
                *b"JOHTO1",
                ROOM_ACTION_SYNC,
                0,
                1,
                0x2222_2222,
                2,
                4,
                5,
                5,
                0,
                2,
                0
            )
        ),
        Ok(1)
    );
    assert!(lobby.is_campaign_started());

    // 4. Stranger with unknown OTID tries joining after lock -> Rejected with ERR_LOCKED
    assert_eq!(
        lobby.handle_client_packet(
            p2_stranger,
            make_packet(
                *b"JOHTO1",
                ROOM_ACTION_JOIN,
                0,
                AUTO_PLAYER_ID,
                0x9999_9999,
                2,
                4,
                5,
                5,
                0,
                2,
                0
            )
        ),
        Err(ROOM_STATUS_ERR_LOCKED)
    );

    // 5. Both walk out of house to Violet Gym (Map 28) -> Gym Gate UNLOCKED (1)
    let _ = lobby.handle_client_packet(
        p0_addr,
        make_packet(
            *b"JOHTO1",
            ROOM_ACTION_SYNC,
            0x1234_5678,
            0,
            0x1111_1111,
            2,
            28,
            10,
            10,
            155,
            5,
            0,
        ),
    );
    let _ = lobby.handle_client_packet(
        p1_addr,
        make_packet(
            *b"JOHTO1",
            ROOM_ACTION_SYNC,
            0,
            1,
            0x2222_2222,
            2,
            28,
            11,
            10,
            158,
            5,
            0,
        ),
    );
    let batch_online = lobby.build_broadcast_batch();
    let gates_offset = 16 + (5 * 32) + (5 * 24);
    assert_eq!(batch_online[0].1[gates_offset + 4], 1);

    // 6. P1 goes stale (80ms > 60ms timeout) and is pruned -> progress gate locks (0)
    sleep(Duration::from_millis(80));
    let _ = lobby.handle_client_packet(
        p0_addr,
        make_packet(
            *b"JOHTO1",
            ROOM_ACTION_SYNC,
            0x1234_5678,
            0,
            0x1111_1111,
            2,
            28,
            10,
            10,
            155,
            5,
            0,
        ),
    );
    lobby.prune_stale_clients();
    let batch_offline = lobby.build_broadcast_batch();
    assert_eq!(batch_offline[0].1[gates_offset + 4], 0);

    // 7. P1 reconnects at Title Screen (OTID 0) and then loads save (Map 28 -> OTID 0x2222_2222)
    let p1_new_addr: SocketAddr = "127.0.0.1:61234".parse().unwrap();
    assert_eq!(
        lobby.handle_client_packet(
            p1_new_addr,
            make_packet(
                *b"JOHTO1",
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
                0
            )
        ),
        Ok(1)
    );
    assert_eq!(
        lobby.handle_client_packet(
            p1_new_addr,
            make_packet(
                *b"JOHTO1",
                ROOM_ACTION_SYNC,
                0,
                AUTO_PLAYER_ID,
                0x2222_2222,
                2,
                28,
                11,
                10,
                158,
                5,
                0
            )
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

#[test]
fn test_inactive_player_is_hidden_from_others() {
    let mut lobby = LobbyState::new(Duration::from_secs(5), 2, false);
    let p0: SocketAddr = "127.0.0.1:50100".parse().unwrap();
    let p1: SocketAddr = "127.0.0.1:50101".parse().unwrap();

    let a = make_packet(
        *b"TEST01",
        ROOM_ACTION_JOIN,
        0,
        0,
        0x1,
        2,
        28,
        5,
        5,
        155,
        0,
        0,
    );
    let mut b = make_packet(
        *b"TEST01",
        ROOM_ACTION_JOIN,
        0,
        1,
        0x2,
        2,
        28,
        6,
        5,
        158,
        0,
        0,
    );
    assert_eq!(lobby.handle_client_packet(p0, a), Ok(0));
    assert_eq!(lobby.handle_client_packet(p1, b), Ok(1));

    // Same map, both active: P0 sees P1 (remote slot 0 = player 1) and its follower.
    let batch = lobby.build_broadcast_batch();
    let p0_pkt = &batch.iter().find(|(addr, _)| *addr == p0).unwrap().1;
    assert_eq!(p0_pkt[16 + 0x0F], 1);
    let follower_visible = 16 + 5 * 32 + 0x0B;
    assert_eq!(p0_pkt[follower_visible], 1);

    // P1 enters a battle: the ROM reports is_active = 0 and the relay must not override it.
    b.local_player.is_active = 0;
    b.sequence = 2;
    assert_eq!(lobby.handle_client_packet(p1, b), Ok(1));
    let batch = lobby.build_broadcast_batch();
    let p0_pkt = &batch.iter().find(|(addr, _)| *addr == p0).unwrap().1;
    assert_eq!(p0_pkt[16 + 0x0F], 0);
    assert_eq!(p0_pkt[follower_visible], 0);
}

#[test]
fn test_idle_rooms_are_closed_and_room_cap_enforced() {
    let test_dir = "target/test_saves_idle";
    let _ = fs::remove_dir_all(test_dir);
    let mut manager = RoomManager::new(Duration::from_millis(20), 2, false, test_dir)
        .with_limits(Duration::from_millis(40), 1);
    let host: SocketAddr = "127.0.0.1:51100".parse().unwrap();

    let create = make_packet(
        *b"IDLE01",
        ROOM_ACTION_CREATE,
        0xABCD,
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
    assert_eq!(send(&mut manager, host, create), Ok(0));

    // Room cap of 1: a second room is refused while the first is alive.
    let other = make_packet(
        *b"IDLE02",
        ROOM_ACTION_CREATE,
        0xBCDE,
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
    assert_eq!(send(&mut manager, host, other), Err(ROOM_STATUS_ERR_FULL));

    // The host times out, then the empty room idles out of memory.
    sleep(Duration::from_millis(30));
    let _ = manager.tick_all_rooms();
    sleep(Duration::from_millis(50));
    let _ = manager.tick_all_rooms();
    assert_eq!(manager.room_count(), 0);

    let other = make_packet(
        *b"IDLE02",
        ROOM_ACTION_CREATE,
        0xBCDE,
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
    assert_eq!(send(&mut manager, host, other), Ok(0));
    let _ = fs::remove_dir_all(test_dir);
}

#[test]
fn test_corrupt_campaign_file_is_refused_not_reset() {
    let test_dir = "target/test_saves_corrupt";
    let _ = fs::remove_dir_all(test_dir);
    fs::create_dir_all(test_dir).unwrap();
    fs::write(format!("{test_dir}/BROKE1.json"), "{ not json").unwrap();

    assert!(matches!(
        LobbyState::with_room(
            *b"BROKE1",
            0,
            None,
            Duration::from_secs(5),
            2,
            false,
            Some(test_dir.into())
        ),
        Err(CampaignLoadError(_))
    ));

    let mut manager = RoomManager::new(Duration::from_secs(5), 2, false, test_dir);
    let addr: SocketAddr = "127.0.0.1:51200".parse().unwrap();
    let join = make_packet(
        *b"BROKE1",
        ROOM_ACTION_JOIN,
        0,
        AUTO_PLAYER_ID,
        0x9999,
        2,
        4,
        0,
        0,
        0,
        0,
        0,
    );
    assert_eq!(send(&mut manager, addr, join), Err(ROOM_STATUS_ERR_LOCKED));
    assert_eq!(manager.room_count(), 0);
    let _ = fs::remove_dir_all(test_dir);
}

#[test]
fn test_started_campaign_is_saved_atomically_and_restored() {
    let test_dir = "target/test_saves_atomic";
    let _ = fs::remove_dir_all(test_dir);
    let mut manager = RoomManager::new(Duration::from_secs(5), 2, false, test_dir);
    let p0: SocketAddr = "127.0.0.1:51300".parse().unwrap();
    let p1: SocketAddr = "127.0.0.1:51301".parse().unwrap();

    assert_eq!(
        send(
            &mut manager,
            p0,
            make_packet(
                *b"SAVE01",
                ROOM_ACTION_CREATE,
                0x77,
                0,
                0x1111,
                2,
                64,
                3,
                3,
                0,
                0,
                0
            )
        ),
        Ok(0)
    );
    assert_eq!(
        send(
            &mut manager,
            p1,
            make_packet(
                *b"SAVE01",
                ROOM_ACTION_JOIN,
                0,
                1,
                0x2222,
                2,
                64,
                4,
                3,
                0,
                0,
                0
            )
        ),
        Ok(1)
    );

    let path = format!("{test_dir}/SAVE01.json");
    assert!(fs::metadata(&path).is_ok());
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
    assert!(fs::metadata(format!("{path}.tmp")).is_err());
    let restored = LobbyState::with_room(
        *b"SAVE01",
        0,
        None,
        Duration::from_secs(5),
        2,
        false,
        Some(test_dir.into()),
    )
    .unwrap();
    assert!(restored.is_campaign_started());
    assert_eq!(restored.expected_party_size(), 2);
    let _ = fs::remove_dir_all(test_dir);
}

fn started_two_player_campaign(
    test_dir: &str,
    code: [u8; 6],
) -> (RoomManager, SocketAddr, SocketAddr) {
    let _ = fs::remove_dir_all(test_dir);
    let mut manager = RoomManager::new(Duration::from_millis(30), 2, false, test_dir);
    let p0: SocketAddr = "127.0.0.1:52000".parse().unwrap();
    let p1: SocketAddr = "127.0.0.1:52001".parse().unwrap();
    assert_eq!(
        send(
            &mut manager,
            p0,
            make_packet(
                code,
                ROOM_ACTION_CREATE,
                0x42,
                0,
                0x1111,
                2,
                64,
                3,
                3,
                0,
                0,
                0
            )
        ),
        Ok(0)
    );
    assert_eq!(
        send(
            &mut manager,
            p1,
            make_packet(code, ROOM_ACTION_JOIN, 0, 1, 0x2222, 2, 64, 4, 3, 0, 0, 0)
        ),
        Ok(1)
    );
    (manager, p0, p1)
}

#[test]
fn test_cookie_handshake_is_required_and_bound_to_address() {
    let test_dir = "target/test_saves_cookie";
    let _ = fs::remove_dir_all(test_dir);
    let mut manager = RoomManager::new(Duration::from_secs(5), 2, false, test_dir);
    let real: SocketAddr = "127.0.0.1:53000".parse().unwrap();
    let spoofed: SocketAddr = "198.51.100.7:40000".parse().unwrap();

    // No cookie: challenged, no room created.
    let create = make_packet(
        *b"COOK01",
        ROOM_ACTION_CREATE,
        0x9,
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
    let Err(Rejection::Challenge(cookie)) = manager.handle_client_packet(real, create) else {
        panic!("expected a challenge");
    };
    assert_eq!(manager.room_count(), 0);

    // A cookie issued to one address is useless from another (spoofed) one.
    let mut forged = create;
    forged.cookie = cookie;
    assert!(matches!(
        manager.handle_client_packet(spoofed, forged),
        Err(Rejection::Challenge(_))
    ));
    assert_eq!(manager.room_count(), 0);

    // Echoing the cookie from the real address completes the handshake.
    let mut echoed = create;
    echoed.cookie = cookie;
    assert_eq!(manager.handle_client_packet(real, echoed), Ok(0));
    assert_eq!(manager.room_count(), 1);
    let _ = fs::remove_dir_all(test_dir);
}

#[test]
fn test_spoofed_leave_cannot_kick_a_player() {
    let (mut manager, p0, _p1) = started_two_player_campaign("target/test_saves_leave", *b"LEAVE1");
    let mut leave = make_packet(
        *b"LEAVE1",
        ROOM_ACTION_LEAVE,
        0,
        0,
        0x1111,
        2,
        64,
        3,
        3,
        0,
        0,
        0,
    );
    leave.cookie = 0;
    assert!(matches!(
        manager.handle_client_packet(p0, leave),
        Err(Rejection::Challenge(_))
    ));
    let batch = manager.tick_all_rooms();
    assert_eq!(batch.iter().filter(|(addr, _)| *addr == p0).count(), 1);
    let _ = fs::remove_dir_all("target/test_saves_leave");
}

#[test]
fn test_failed_admissions_are_rate_limited_per_ip() {
    let test_dir = "target/test_saves_ratelimit";
    let mut manager = RoomManager::new(Duration::from_secs(5), 2, false, test_dir);
    let guesser: SocketAddr = "127.0.0.9:54000".parse().unwrap();
    let neighbour: SocketAddr = "127.0.0.10:54000".parse().unwrap();

    for i in 0..10u8 {
        let code = [b'G', b'U', b'E', b'S', b'S', b'0' + i];
        let join = make_packet(
            code,
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
            send(&mut manager, guesser, join),
            Err(ROOM_STATUS_ERR_NOT_FOUND)
        );
    }
    let join = make_packet(
        *b"GUESSX",
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
        send(&mut manager, guesser, join),
        Err(ROOM_STATUS_ERR_RATE_LIMITED)
    );
    // Other IPs are unaffected.
    assert_eq!(
        send(&mut manager, neighbour, join),
        Err(ROOM_STATUS_ERR_NOT_FOUND)
    );
}

#[test]
fn test_one_ip_cannot_host_unlimited_rooms() {
    let test_dir = "target/test_saves_hostcap";
    let _ = fs::remove_dir_all(test_dir);
    let mut manager = RoomManager::new(Duration::from_secs(5), 2, false, test_dir);
    for i in 0..4u16 {
        let addr: SocketAddr = format!("127.0.0.20:{}", 55000 + i).parse().unwrap();
        let code = [b'H', b'O', b'S', b'T', b'0', b'0' + i as u8];
        let create = make_packet(
            code,
            ROOM_ACTION_CREATE,
            0x100 + u32::from(i),
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
        assert_eq!(send(&mut manager, addr, create), Ok(0));
    }
    let addr: SocketAddr = "127.0.0.20:55010".parse().unwrap();
    let create = make_packet(
        *b"HOST05",
        ROOM_ACTION_CREATE,
        0x200,
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
    assert_eq!(send(&mut manager, addr, create), Err(ROOM_STATUS_ERR_FULL));
    let _ = fs::remove_dir_all(test_dir);
}

#[test]
fn test_known_otid_without_matching_secret_cannot_take_a_slot() {
    let test_dir = "target/test_saves_impersonate";
    let (mut manager, _p0, _p1) = started_two_player_campaign(test_dir, *b"IMPER1");
    // P1 goes offline.
    sleep(Duration::from_millis(50));
    let _ = manager.tick_all_rooms();

    // P0 saw P1's OTID (0x2222) in broadcasts and tries to claim it from another socket.
    let thief: SocketAddr = "127.0.0.1:52999".parse().unwrap();
    let mut claim = make_packet(
        *b"IMPER1",
        ROOM_ACTION_SYNC,
        0,
        AUTO_PLAYER_ID,
        0x2222,
        2,
        64,
        4,
        3,
        0,
        0,
        0,
    );
    claim.player_secret = 0xBAD;
    assert_eq!(
        send(&mut manager, thief, claim),
        Err(ROOM_STATUS_ERR_LOCKED)
    );

    // The real P1 (same bridge secret) is accepted from a new socket.
    let p1_new: SocketAddr = "127.0.0.1:52555".parse().unwrap();
    let rejoin = make_packet(
        *b"IMPER1",
        ROOM_ACTION_SYNC,
        0,
        AUTO_PLAYER_ID,
        0x2222,
        2,
        64,
        4,
        3,
        0,
        0,
        0,
    );
    assert_eq!(send(&mut manager, p1_new, rejoin), Ok(1));
    let _ = fs::remove_dir_all(test_dir);
}

#[test]
fn test_title_screen_guest_is_displaced_by_slot_owner() {
    let test_dir = "target/test_saves_guest";
    let (mut manager, p0, _p1) = started_two_player_campaign(test_dir, *b"GUEST1");
    sleep(Duration::from_millis(50));
    let _ = manager.tick_all_rooms(); // both time out

    // P0 keeps playing; an anonymous title-screen guest borrows the free slot 1.
    let keep_alive = make_packet(
        *b"GUEST1",
        ROOM_ACTION_SYNC,
        0,
        0,
        0x1111,
        2,
        64,
        3,
        3,
        0,
        0,
        0,
    );
    assert_eq!(send(&mut manager, p0, keep_alive), Ok(0));
    let guest: SocketAddr = "127.0.0.1:52777".parse().unwrap();
    let mut title = make_packet(
        *b"GUEST1",
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
    title.player_secret = 0x6E57;
    assert_eq!(send(&mut manager, guest, title), Ok(1));

    // The owner of slot 1 arrives at the title screen (secret only) and gets it back.
    let p1_new: SocketAddr = "127.0.0.1:52888".parse().unwrap();
    let mut owner_title = make_packet(
        *b"GUEST1",
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
    owner_title.player_secret = secret_for(0x2222);
    assert_eq!(send(&mut manager, p1_new, owner_title), Ok(1));

    // No free roster slot is left for the guest.
    title.sequence = 2;
    assert_eq!(
        send(&mut manager, guest, title),
        Err(ROOM_STATUS_ERR_LOCKED)
    );
    let _ = fs::remove_dir_all(test_dir);
}

#[test]
fn test_legacy_roster_without_secret_binds_first_claimant() {
    let test_dir = "target/test_saves_legacy";
    let _ = fs::remove_dir_all(test_dir);
    fs::create_dir_all(test_dir).unwrap();
    fs::write(
        format!("{test_dir}/OLDIE1.json"),
        r#"{"room_code":"OLDIE1","host_token":1,"campaign_started":true,"expected_party_size":2,
            "roster":[{"slot_id":0,"trainer_id":4369,"last_badge_mask":0,"assigned_monolocke":18},
                      {"slot_id":1,"trainer_id":8738,"last_badge_mask":0,"assigned_monolocke":18}]}"#,
    )
    .unwrap();
    let mut manager = RoomManager::new(Duration::from_secs(5), 2, false, test_dir);
    let first: SocketAddr = "127.0.0.1:56000".parse().unwrap();
    let second: SocketAddr = "127.0.0.1:56001".parse().unwrap();

    let claim = make_packet(
        *b"OLDIE1",
        ROOM_ACTION_JOIN,
        0,
        AUTO_PLAYER_ID,
        0x1111,
        2,
        64,
        3,
        3,
        0,
        0,
        0,
    );
    assert_eq!(send(&mut manager, first, claim), Ok(0));
    let saved = fs::read_to_string(format!("{test_dir}/OLDIE1.json")).unwrap();
    assert!(saved.contains(&format!("\"player_secret\": {}", secret_for(0x1111))));

    let mut other = claim;
    other.player_secret = 0xD1FF;
    assert_eq!(
        send(&mut manager, second, other),
        Err(ROOM_STATUS_ERR_LOCKED)
    );
    let _ = fs::remove_dir_all(test_dir);
}
