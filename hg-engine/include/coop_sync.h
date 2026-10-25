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
//            bridge_hb_last_seen, bridge_stale_frames, limit_* (static), diag,
//            debug_result, player_state
//   Bridge : assigned_player_id, active_players_mask, server_tick,
//            remote_players, remote_followers, gates, is_connected_to_room,
//            heartbeat_bridge, debug_cmd, debug_seq, debug_arg
// ---------------------------------------------------------------------------

#define COOP_MAGIC           0x434F4F50 // bytes in RAM: "POOC"
// Version of this mailbox layout (independent of the UDP wire version).
// Must match coop_offsets.json "mailbox_version" and the bridge.
#define COOP_MAILBOX_VERSION 5
#define COOP_MAX_PLAYERS      6
#define COOP_REMOTE_SLOTS     (COOP_MAX_PLAYERS - 1)
#define COOP_PLAYER_ID_NONE   0xFF

// Players spawn in the bedroom (player's house 2F) after New Game.
#define COOP_BEDROOM_MAP_ID 64

// The bedroom's only exit: a stairs warp on (3, 4) that fires while the
// player stands on it facing west and holds Left (pret zone_event
// 061_T20R0202 warp + FieldSystem_CheckMapTransition). Until the adventure
// starts, standing there facing the stairs shows a message and steps the
// player back east to (4, 4).
#define COOP_BEDROOM_STAIRS_X 3
#define COOP_BEDROOM_STAIRS_Z 4

// Map object IDs reserved for remote players' ghosts (0xE0-0xE4) and their
// followers (0xE5-0xE9). Vanilla maps use IDs <= 50; 249-255 are reserved
// by vanilla scripts (photo subject, daycare, apricorn, partner, player).
#define COOP_GHOST_ID_FIRST    0xE0
#define COOP_GHOST_ID_LAST     0xE9
#define COOP_IS_GHOST_ID(id)   ((id) >= COOP_GHOST_ID_FIRST && (id) <= COOP_GHOST_ID_LAST)
// Phase 0 probe: one test ghost spawned from the bridge's diagnostics card.
#define COOP_TEST_GHOST_ID     COOP_GHOST_ID_FIRST

// Overlay 131 (OVERLAY_FIELD_EXTENSION, src/field/) is loaded together with
// the field overlay. Overlay 129 links before it, so it cannot call 131 by
// symbol: src/field/linker.ld pins this header at the start of Overlay 131
// and Coop_MainSysTask calls through it.
#define COOP_FIELD_ENTRY_ADDR  0x023C8000
#define COOP_FIELD_ENTRY_MAGIC 0x31464F43 // "COF1"

struct FieldSystem;
struct CoopFieldEntryHeader
{
    u32 magic;
    void (*entry)(struct FieldSystem *fsys); // runs once per frame while the field map is active
};

// DEVELOPMENT ONLY (docs/DEBUG_TOOLS.md): the bridge's debug commands below
// (test ghost, warp, bike / surf / fishing). Set to 0 before shipping: the
// ROM then compiles the handlers out and acknowledges every command with
// COOP_DEBUG_RESULT_DISABLED. The mailbox fields stay so the layout never
// changes between builds.
#define COOP_DEBUG_TOOLS 1

// Bridge -> ROM debug commands (diagnostics card). The bridge writes
// debug_cmd and debug_arg, then bumps debug_seq; the ROM answers with
// debug_result and sets diag.debug_ack = debug_seq.
enum CoopDebugCommand
{
    COOP_DEBUG_NONE = 0,
    COOP_DEBUG_SPAWN_TEST_GHOST = 1, // in front of the player, same sprite and height
    COOP_DEBUG_STEP_TEST_GHOST = 2,  // one step in the player's facing direction
    COOP_DEBUG_EMOTE_TEST_GHOST = 3, // "!" emote
    COOP_DEBUG_REMOVE_TEST_GHOST = 4,
    COOP_DEBUG_WARP = 5,             // debug_arg = enum CoopWarpPreset
    COOP_DEBUG_TOGGLE_BIKE = 6,      // walking <-> cycling
    COOP_DEBUG_SURF = 7,             // start surfing onto the faced water tile
    COOP_DEBUG_FISH = 8,             // cast the Old Rod into the faced water
};

enum CoopDebugResult
{
    COOP_DEBUG_RESULT_NONE = 0,
    COOP_DEBUG_RESULT_OK = 1,
    COOP_DEBUG_RESULT_NO_TARGET = 2,  // ghost commands without a test ghost
    COOP_DEBUG_RESULT_NOT_NOW = 3,    // wrong player state (e.g. surf while cycling)
    COOP_DEBUG_RESULT_NO_WATER = 4,   // surf / fish not facing surfable water
    COOP_DEBUG_RESULT_BAD_ARG = 5,    // unknown command or warp preset
    COOP_DEBUG_RESULT_FAILED = 6,     // no free map object or heap memory
    COOP_DEBUG_RESULT_DISABLED = 7,   // built with COOP_DEBUG_TOOLS 0
};

// Warp presets: vanilla fly landing spots (ccd src/spawn_warp.c) chosen to
// test dense maps, zone borders, bridges and water. Must match the bridge.
enum CoopWarpPreset
{
    COOP_WARP_NEW_BARK = 0,  // New Bark Town: zone border to Route 29, water to the east
    COOP_WARP_CHERRYGROVE,   // Cherrygrove City: beach (surf / fish)
    COOP_WARP_VIOLET,        // Violet City: zone borders to Routes 31, 32, 36
    COOP_WARP_GOLDENROD,     // Goldenrod City: dense map
    COOP_WARP_ECRUTEAK,      // Ecruteak City: dense map
    COOP_WARP_OLIVINE,       // Olivine City: harbor water
    COOP_WARP_ROUTE_40,      // Route 40: beach and sea
    COOP_WARP_CERULEAN,      // Cerulean City: Nugget Bridge (Route 24) to the north
    COOP_WARP_LAVENDER,      // Lavender Town: Route 12 Silence Bridge to the south
    COOP_WARP_CELADON,       // Celadon City: dense map
    COOP_WARP_COUNT
};

// Why Coop_MainSysTask did or did not call the Overlay 131 entry this frame.
enum CoopFieldEntryStatus
{
    COOP_FIELD_ENTRY_NOT_IN_FIELD = 0, // title screen, intro, battle, menu, transition
    COOP_FIELD_ENTRY_NO_OVERLAY = 1,   // field running but Overlay 131 not loaded
    COOP_FIELD_ENTRY_BAD_HEADER = 2,   // Overlay 131 loaded without our header
    COOP_FIELD_ENTRY_CALLED = 3,
};

enum CoopTestGhostState
{
    COOP_TEST_GHOST_NONE = 0,
    COOP_TEST_GHOST_IDLE = 1,
    COOP_TEST_GHOST_MOVING = 2,
};

// ROM-owned diagnostics written by the Overlay 131 field entry
// (docs/MILESTONE2_PLAN.md, section 9).
struct CoopDiagnostics
{
    u32 field_entry_calls;  // 0x00 frames the Overlay 131 entry ran (pauses outside the field)
    u8  render_ready;       // 0x04 1 = renderer counters below are valid
    u8  tex_resident;       // 0x05 resident texture slots (preloaded sprites + 3)
    u8  tex_dynamic;        // 0x06 dynamic texture slots
    u8  tex_dynamic_free;   // 0x07
    u8  model_dynamic;      // 0x08
    u8  model_dynamic_free; // 0x09
    u8  seq_dynamic;        // 0x0A
    u8  seq_dynamic_free;   // 0x0B
    u8  map_objects_active; // 0x0C live map objects (capacity 64)
    u8  test_ghost_state;   // 0x0D enum CoopTestGhostState
    u8  debug_ack;          // 0x0E last debug_seq processed
    u8  field_entry_status; // 0x0F enum CoopFieldEntryStatus (written by Overlay 129)
    s16 test_ghost_x;       // 0x10 tile coordinates of the test ghost
    s16 test_ghost_z;       // 0x12
    s16 test_ghost_y;       // 0x14 render height (position vector Y, world units)
    s16 player_y;           // 0x16 local player's render height, for comparison
}; // 24 bytes

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
    struct CoopDiagnostics diag;                                // 0x17C ROM
    u8  debug_cmd;                                              // 0x194 bridge: enum CoopDebugCommand
    u8  debug_seq;                                              // 0x195 bridge: bumped per command
    u8  debug_arg;                                              // 0x196 bridge: command argument
    u8  debug_result;                                           // 0x197 ROM: enum CoopDebugResult
    u8  player_state;                                           // 0x198 ROM: pret PLAYER_STATE_* (field only)
    u8  pad199;                                                 // 0x199
    u8  reserved[0x200 - 0x19A];                                // 0x19A..0x1FF
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
COOP_ASSERT_OFFSET(CoopCommBlock, diag, 0x17C);
COOP_ASSERT_OFFSET(CoopCommBlock, debug_cmd, 0x194);
COOP_ASSERT_OFFSET(CoopCommBlock, debug_seq, 0x195);
COOP_ASSERT_OFFSET(CoopCommBlock, debug_arg, 0x196);
COOP_ASSERT_OFFSET(CoopCommBlock, debug_result, 0x197);
COOP_ASSERT_OFFSET(CoopCommBlock, player_state, 0x198);
COOP_ASSERT_OFFSET(CoopCommBlock, reserved, 0x19A);
_Static_assert(sizeof(struct CoopDiagnostics) == 24, "CoopDiagnostics must be 24 bytes");
COOP_ASSERT_OFFSET(CoopDiagnostics, test_ghost_x, 0x10);
COOP_ASSERT_OFFSET(CoopDiagnostics, player_y, 0x16);

extern volatile struct CoopCommBlock gCoopCommBlock;

struct SysTask;
struct TaskManager;

// Field tasks (Overlay 129, also called from Overlay 131). While one runs,
// vanilla blocks player input like an NPC script; only one can exist
// (fsys->taskman must be NULL). Coop_FieldSystem_CreateTask takes an env
// owned by the task; Coop_StartFieldTask allocates a zeroed one the task
// must free. Both return NULL / FALSE when the field heap is full.
struct TaskManager *LONG_CALL Coop_FieldSystem_CreateTask(struct FieldSystem *fsys, BOOL (*func)(struct TaskManager *), void *env);
BOOL LONG_CALL Coop_StartFieldTask(struct FieldSystem *fsys, BOOL (*func)(struct TaskManager *), u32 envSize);

BOOL LONG_CALL Coop_IsConnectedToRoom(void);
// TRUE once the relay locked the roster (everyone was ready in the bedroom).
BOOL LONG_CALL Coop_IsAdventureStarted(void);
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
