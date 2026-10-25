#ifndef COOP_SYNC_H
#define COOP_SYNC_H

#include "types.h"

// ---------------------------------------------------------------------------
// Shared mailbox between the ROM (Overlay 129) and the BizHawk coop-bridge.
//
// This header is the single source of truth for the layout. It must stay
// byte-for-byte identical to:
//   - coop-bridge/src/CoopBridgeForm.cs (MailboxLayout)
//   - coop-relay/src/protocol.rs        (PlayerState / FollowerState / gates)
// All fields are little-endian. Every offset is pinned by a _Static_assert
// below, so a layout change fails the build instead of corrupting RAM.
//
// Write ownership (never write a field you do not own):
//   ROM    : heartbeat_rom, local_player, local_player_seq, is_boot_done,
//            was_in_active_session, disconnect_state, badge_mask_local,
//            bridge_hb_last_seen, bridge_stale_frames, limit_* (static)
//   Bridge : assigned_player_id, active_players_mask, server_tick,
//            remote_players, remote_followers, gates, is_connected_to_room,
//            heartbeat_bridge
// ---------------------------------------------------------------------------

#define COOP_MAGIC           0x434F4F50 // bytes in RAM: "POOC"
// Version of this mailbox layout (independent of the UDP wire version).
// Must match coop_offsets.json "mailbox_version" and the bridge.
#define COOP_MAILBOX_VERSION 3
#define COOP_MAX_PLAYERS      6
#define COOP_REMOTE_SLOTS     (COOP_MAX_PLAYERS - 1)
#define COOP_PLAYER_ID_NONE   0xFF

// Players spawn in the bedroom (player's house 2F) after New Game.
#define COOP_BEDROOM_MAP_ID 64

// The bridge bumps heartbeat_bridge once per emulated frame. If the ROM sees
// no change for this many of its own main-loop ticks, the bridge is treated
// as gone (window closed, crashed, or a stale savestate was loaded).
#define COOP_BRIDGE_TIMEOUT_FRAMES 120

enum CoopDisconnectState
{
    COOP_DISCONNECT_NONE = 0,    // connected, or never joined a session
    COOP_DISCONNECT_PENDING = 1, // link dropped mid-session; waiting for a safe overworld frame
    COOP_DISCONNECT_RUNNING = 2, // save-and-return field task is running
};

// 32-byte per-player state. Used for local_player and each remote slot.
struct CoopPlayerState
{
    u16 map_id;          // 0x00
    s16 x;               // 0x02 tile X
    s16 y;               // 0x04 tile Z (the overworld's "y")
    s16 prev_x;          // 0x06
    s16 prev_y;          // 0x08
    u8  direction;       // 0x0A
    u8  movement_type;   // 0x0B
    u16 sprite_id;       // 0x0C
    u8  animation_frame; // 0x0E
    u8  is_active;       // 0x0F 1 = standing in a running field map
    u16 lead_species;    // 0x10
    u16 lead_form;       // 0x12
    u8  lead_is_shiny;   // 0x14
    u8  lead_gender;     // 0x15
    u8  map_npc_count;   // 0x16 (not populated yet)
    u8  client_ready;    // 0x17 1 = in bedroom, valid OTID, empty party
    u32 trainer_id;      // 0x18
    u8  pad[4];          // 0x1C
};

// 24-byte remote follower state (relay -> ROM).
struct CoopFollowerState
{
    u16 species;    // 0x00
    u16 form;       // 0x02
    u8  is_shiny;   // 0x04
    u8  gender;     // 0x05
    s16 x;          // 0x06
    s16 y;          // 0x08
    u8  direction;  // 0x0A
    u8  is_visible; // 0x0B
    u8  pad[12];    // 0x0C
};

// 24-byte progression gates block (relay -> ROM).
struct CoopProgressionGates
{
    u16 badge_mask_local_echo; // 0x00
    u16 badge_mask_synced;     // 0x02
    u8  flag_coop_sync_ready;  // 0x04
    u8  current_checkpoint;    // 0x05
    u8  assigned_monolocke;    // 0x06
    u8  nuzlocke_failed;       // 0x07
    u8  flag_campaign_started; // 0x08
    u8  expected_party_size;   // 0x09
    u8  online_party_count;    // 0x0A
    u8  ready_party_count;     // 0x0B
    u8  pad[12];               // 0x0C
};

// 512-byte mailbox. Offsets are frozen; never reorder fields.
// remote_players[k] holds player id k if k < assigned_player_id, else k + 1.
struct CoopCommBlock
{
    u32 magic;                                                  // 0x000
    u16 version;                                                // 0x004
    u8  assigned_player_id;                                     // 0x006
    u8  active_players_mask;                                    // 0x007
    u32 heartbeat_rom;                                          // 0x008
    u32 server_tick;                                            // 0x00C
    struct CoopPlayerState local_player;                        // 0x010
    struct CoopPlayerState remote_players[COOP_REMOTE_SLOTS];   // 0x030
    struct CoopFollowerState remote_followers[COOP_REMOTE_SLOTS]; // 0x0D0
    struct CoopProgressionGates gates;                          // 0x148
    u8  is_connected_to_room;                                   // 0x160
    u8  is_boot_done;                                           // 0x161
    u8  was_in_active_session;                                  // 0x162
    u8  disconnect_state;                                       // 0x163 enum CoopDisconnectState
    u16 badge_mask_local;                                       // 0x164
    u16 pad166;                                                 // 0x166
    u32 heartbeat_bridge;                                       // 0x168
    u32 local_player_seq;                                       // 0x16C odd while local_player is being written
    u32 bridge_hb_last_seen;                                    // 0x170
    u16 bridge_stale_frames;                                    // 0x174
    u16 pad176;                                                 // 0x176
    // Validation limits published by the ROM so the bridge can bounds-check
    // relay data without duplicating game constants.
    u16 limit_map_id;                                           // 0x178 exclusive: map_id < limit
    u16 limit_species;                                          // 0x17A inclusive: species <= limit
    u8  reserved[0x200 - 0x17C];                                // 0x17C..0x1FF
};

#define COOP_ASSERT_OFFSET(type, field, off) \
    _Static_assert(__builtin_offsetof(struct type, field) == (off), #type "." #field " must be at " #off)

_Static_assert(sizeof(struct CoopPlayerState) == 32, "CoopPlayerState must be 32 bytes");
_Static_assert(sizeof(struct CoopFollowerState) == 24, "CoopFollowerState must be 24 bytes");
_Static_assert(sizeof(struct CoopProgressionGates) == 24, "CoopProgressionGates must be 24 bytes");
_Static_assert(sizeof(struct CoopCommBlock) == 512, "CoopCommBlock must be 512 bytes");

COOP_ASSERT_OFFSET(CoopPlayerState, sprite_id, 0x0C);
COOP_ASSERT_OFFSET(CoopPlayerState, lead_species, 0x10);
COOP_ASSERT_OFFSET(CoopPlayerState, client_ready, 0x17);
COOP_ASSERT_OFFSET(CoopPlayerState, trainer_id, 0x18);
COOP_ASSERT_OFFSET(CoopFollowerState, x, 0x06);
COOP_ASSERT_OFFSET(CoopProgressionGates, flag_campaign_started, 0x08);
COOP_ASSERT_OFFSET(CoopCommBlock, assigned_player_id, 0x006);
COOP_ASSERT_OFFSET(CoopCommBlock, heartbeat_rom, 0x008);
COOP_ASSERT_OFFSET(CoopCommBlock, server_tick, 0x00C);
COOP_ASSERT_OFFSET(CoopCommBlock, local_player, 0x010);
COOP_ASSERT_OFFSET(CoopCommBlock, remote_players, 0x030);
COOP_ASSERT_OFFSET(CoopCommBlock, remote_followers, 0x0D0);
COOP_ASSERT_OFFSET(CoopCommBlock, gates, 0x148);
COOP_ASSERT_OFFSET(CoopCommBlock, is_connected_to_room, 0x160);
COOP_ASSERT_OFFSET(CoopCommBlock, disconnect_state, 0x163);
COOP_ASSERT_OFFSET(CoopCommBlock, badge_mask_local, 0x164);
COOP_ASSERT_OFFSET(CoopCommBlock, heartbeat_bridge, 0x168);
COOP_ASSERT_OFFSET(CoopCommBlock, local_player_seq, 0x16C);
COOP_ASSERT_OFFSET(CoopCommBlock, bridge_hb_last_seen, 0x170);
COOP_ASSERT_OFFSET(CoopCommBlock, bridge_stale_frames, 0x174);
COOP_ASSERT_OFFSET(CoopCommBlock, limit_map_id, 0x178);
COOP_ASSERT_OFFSET(CoopCommBlock, limit_species, 0x17A);
COOP_ASSERT_OFFSET(CoopCommBlock, reserved, 0x17C);

extern volatile struct CoopCommBlock gCoopCommBlock;

struct SysTask;

BOOL LONG_CALL Coop_IsConnectedToRoom(void);
void LONG_CALL Coop_CheckStartupGate(void);

// Validated copies of relay-provided data for ROM consumers (ghosts,
// followers). Return FALSE, leaving *out zeroed, when the slot is out of
// range, inactive, or holds out-of-bounds values. Never read
// remote_players / remote_followers directly: the bridge and relay are
// untrusted input.
BOOL LONG_CALL Coop_ReadRemotePlayer(u32 remoteSlot, struct CoopPlayerState *out);
BOOL LONG_CALL Coop_ReadRemoteFollower(u32 remoteSlot, struct CoopFollowerState *out);

// 60 Hz background SysTask (created once in SaveData_New): heartbeat,
// bridge watchdog, local player telemetry and the mid-game disconnect latch.
void Coop_MainSysTask(struct SysTask *task, void *param);

#endif // COOP_SYNC_H
