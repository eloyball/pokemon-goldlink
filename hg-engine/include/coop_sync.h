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
//            debug_result, start_save_state, save_counter, track_*
//   Bridge : assigned_player_id, active_players_mask, server_tick,
//            remote_players, gates, is_connected_to_room, room_rejection,
//            heartbeat_bridge, debug_cmd, debug_seq, debug_arg,
//            save_slot_ready, reset_request
// ---------------------------------------------------------------------------

#define COOP_MAGIC           0x434F4F50 // bytes in RAM: "POOC"
// Version of this mailbox layout (independent of the UDP wire version).
// Must match coop_offsets.json "mailbox_version" and the bridge.
#define COOP_MAILBOX_VERSION 6
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

// What a player is doing (CoopPlayerState.activity). Ghosts of players who
// are not in the field stay frozen where they stood (docs/MILESTONE2_PLAN.md).
enum CoopActivity
{
    COOP_ACTIVITY_NONE = 0,       // title screen, intro, not booted
    COOP_ACTIVITY_FIELD = 1,      // walking around the overworld
    COOP_ACTIVITY_BATTLE = 2,
    COOP_ACTIVITY_MENU = 3,       // bag, party, Pokedex or another app over the field
    COOP_ACTIVITY_TRANSITION = 4, // warp, map load
};

// Whether the loaded save may join a room that has not started yet
// (CoopPlayerState.save_status; docs/MILESTONE2_PLAN.md section 7.1).
enum CoopSaveStatus
{
    COOP_SAVE_UNKNOWN = 0,     // not in the overworld yet
    COOP_SAVE_FRESH = 1,       // in the bedroom with an empty party
    COOP_SAVE_IN_PROGRESS = 2, // anything else
};

// 64-byte per-player state, follower included. Used for local_player (ROM)
// and each remote slot (relay data, untrusted: read through
// Coop_ReadRemotePlayer). Must match coop-relay/src/protocol.rs PlayerState.
struct CoopPlayerState
{
    u16 map_id;              // 0x00
    u8  matrix_id;           // 0x02 MapMatrix.matrix_id; 0 = the outdoor world
    u8  avatar_state;        // 0x03 pret PLAYER_STATE_* (walking, cycling, surfing...)
    s16 x;                   // 0x04 tile X
    s16 z;                   // 0x06 tile Z
    s16 height;              // 0x08 render height (position vector Y, world units; tile height = height / 8)
    u8  direction;           // 0x0A DIR_*
    u8  gender;              // 0x0B 0 boy, 1 girl
    u16 sprite_id;           // 0x0C actual overworld sprite
    u8  move_action;         // 0x0E last held movement action (pret constants/movements.h); 0xFF = none since the map loaded
    u8  move_seq;            // 0x0F bumped each time a new action starts
    u8  is_active;           // 0x10 1 = in a running field map
    u8  client_ready;        // 0x11 1 = in bedroom, valid OTID, empty party
    u8  save_status;         // 0x12 enum CoopSaveStatus
    u8  activity;            // 0x13 enum CoopActivity
    u32 trainer_id;          // 0x14
    u16 name[8];             // 0x18 HG charset, 0xFFFF-terminated
    u16 follower_species;    // 0x28 0 = no follower out
    u16 follower_sprite_id;  // 0x2A
    u8  follower_form;       // 0x2C
    u8  follower_shiny;      // 0x2D
    u8  follower_gender;     // 0x2E
    u8  follower_visible;    // 0x2F
    s16 follower_x;          // 0x30
    s16 follower_z;          // 0x32
    s16 follower_height;     // 0x34
    u8  follower_direction;  // 0x36
    u8  follower_move_action; // 0x37
    u8  follower_move_seq;   // 0x38
    u8  pad39[3];            // 0x39
    u8  avatar_id;           // 0x3C custom player sprite set (later milestone; 0 = vanilla)
    u8  pad3D[3];            // 0x3D
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
    u32 room_uid;              // 0x0C random per room, never reused (keys per-room saves)
    u8  pad[8];                // 0x10
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
    struct CoopPlayerState remote_players[COOP_REMOTE_SLOTS];   // 0x050
    struct CoopProgressionGates gates;                          // 0x190
    u8  is_connected_to_room;                                   // 0x1A8
    u8  is_boot_done;                                           // 0x1A9
    u8  was_in_active_session;                                  // 0x1AA
    u8  disconnect_state;                                       // 0x1AB enum CoopDisconnectState
    u16 badge_mask_local;                                       // 0x1AC
    u8  room_rejection;                                         // 0x1AE bridge: relay ROOM_STATUS_ERR_* that turned this save away
    u8  start_save_state;                                       // 0x1AF ROM: adventure-start save progress (Phase 1.5)
    u32 heartbeat_bridge;                                       // 0x1B0
    u32 local_player_seq;                                       // 0x1B4 odd while local_player is being written
    u32 bridge_hb_last_seen;                                    // 0x1B8
    u16 bridge_stale_frames;                                    // 0x1BC
    u16 pad1BE;                                                 // 0x1BE
    // Validation limits published by the ROM so the bridge can bounds-check
    // relay data without duplicating game constants.
    u16 limit_map_id;                                           // 0x1C0 exclusive: map_id < limit
    u16 limit_species;                                          // 0x1C2 inclusive: species <= limit
    struct CoopDiagnostics diag;                                // 0x1C4 ROM
    u8  debug_cmd;                                              // 0x1DC bridge: enum CoopDebugCommand
    u8  debug_seq;                                              // 0x1DD bridge: bumped per command
    u8  debug_arg;                                              // 0x1DE bridge: command argument
    u8  debug_result;                                           // 0x1DF ROM: enum CoopDebugResult
    u32 save_counter;                                           // 0x1E0 ROM: bumped after every completed save write (Phase 1.5)
    u8  save_slot_ready;                                        // 0x1E4 bridge: the room's save is in SRAM (Phase 1.5)
    u8  reset_request;                                          // 0x1E5 bridge: soft-reset at the title screen (Phase 1.5)
    u8  track_player_step;                                      // 0x1E6 ROM private: last movement step seen (move_seq)
    u8  track_follower_step;                                    // 0x1E7 ROM private
    u8  reserved[0x200 - 0x1E8];                                // 0x1E8..0x1FF
};

#define COOP_ASSERT_OFFSET(type, field, off) \
    _Static_assert(__builtin_offsetof(struct type, field) == (off), #type "." #field " must be at " #off)

_Static_assert(sizeof(struct CoopPlayerState) == 64, "CoopPlayerState must be 64 bytes");
_Static_assert(sizeof(struct CoopProgressionGates) == 24, "CoopProgressionGates must be 24 bytes");
_Static_assert(sizeof(struct CoopCommBlock) == 512, "CoopCommBlock must be 512 bytes");
_Static_assert(sizeof(struct CoopDiagnostics) == 24, "CoopDiagnostics must be 24 bytes");

COOP_ASSERT_OFFSET(CoopPlayerState, matrix_id, 0x02);
COOP_ASSERT_OFFSET(CoopPlayerState, height, 0x08);
COOP_ASSERT_OFFSET(CoopPlayerState, sprite_id, 0x0C);
COOP_ASSERT_OFFSET(CoopPlayerState, move_seq, 0x0F);
COOP_ASSERT_OFFSET(CoopPlayerState, activity, 0x13);
COOP_ASSERT_OFFSET(CoopPlayerState, trainer_id, 0x14);
COOP_ASSERT_OFFSET(CoopPlayerState, name, 0x18);
COOP_ASSERT_OFFSET(CoopPlayerState, follower_species, 0x28);
COOP_ASSERT_OFFSET(CoopPlayerState, follower_x, 0x30);
COOP_ASSERT_OFFSET(CoopPlayerState, follower_move_seq, 0x38);
COOP_ASSERT_OFFSET(CoopPlayerState, avatar_id, 0x3C);
COOP_ASSERT_OFFSET(CoopProgressionGates, flag_campaign_started, 0x08);
COOP_ASSERT_OFFSET(CoopProgressionGates, room_uid, 0x0C);
COOP_ASSERT_OFFSET(CoopCommBlock, assigned_player_id, 0x006);
COOP_ASSERT_OFFSET(CoopCommBlock, heartbeat_rom, 0x008);
COOP_ASSERT_OFFSET(CoopCommBlock, server_tick, 0x00C);
COOP_ASSERT_OFFSET(CoopCommBlock, local_player, 0x010);
COOP_ASSERT_OFFSET(CoopCommBlock, remote_players, 0x050);
COOP_ASSERT_OFFSET(CoopCommBlock, gates, 0x190);
COOP_ASSERT_OFFSET(CoopCommBlock, is_connected_to_room, 0x1A8);
COOP_ASSERT_OFFSET(CoopCommBlock, disconnect_state, 0x1AB);
COOP_ASSERT_OFFSET(CoopCommBlock, badge_mask_local, 0x1AC);
COOP_ASSERT_OFFSET(CoopCommBlock, room_rejection, 0x1AE);
COOP_ASSERT_OFFSET(CoopCommBlock, start_save_state, 0x1AF);
COOP_ASSERT_OFFSET(CoopCommBlock, heartbeat_bridge, 0x1B0);
COOP_ASSERT_OFFSET(CoopCommBlock, local_player_seq, 0x1B4);
COOP_ASSERT_OFFSET(CoopCommBlock, bridge_hb_last_seen, 0x1B8);
COOP_ASSERT_OFFSET(CoopCommBlock, bridge_stale_frames, 0x1BC);
COOP_ASSERT_OFFSET(CoopCommBlock, limit_map_id, 0x1C0);
COOP_ASSERT_OFFSET(CoopCommBlock, limit_species, 0x1C2);
COOP_ASSERT_OFFSET(CoopCommBlock, diag, 0x1C4);
COOP_ASSERT_OFFSET(CoopCommBlock, debug_cmd, 0x1DC);
COOP_ASSERT_OFFSET(CoopCommBlock, debug_result, 0x1DF);
COOP_ASSERT_OFFSET(CoopCommBlock, save_counter, 0x1E0);
COOP_ASSERT_OFFSET(CoopCommBlock, save_slot_ready, 0x1E4);
COOP_ASSERT_OFFSET(CoopCommBlock, reset_request, 0x1E5);
COOP_ASSERT_OFFSET(CoopCommBlock, track_player_step, 0x1E6);
COOP_ASSERT_OFFSET(CoopCommBlock, reserved, 0x1E8);
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

// Validated copy of relay-provided data for ROM consumers (ghosts and their
// followers). Returns FALSE, leaving *out zeroed, when the slot is out of
// range, empty, or holds out-of-bounds values; the follower fields are
// zeroed alone when only they are invalid. A player outside the field
// (is_active 0: battle, menu) is still returned, so the ghost can freeze.
// Never read remote_players directly: the bridge and relay are untrusted.
BOOL LONG_CALL Coop_ReadRemotePlayer(u32 remoteSlot, struct CoopPlayerState *out);

// 60 Hz background SysTask (created once in SaveData_New): heartbeat,
// bridge watchdog, local player telemetry and the mid-game disconnect latch.
void Coop_MainSysTask(struct SysTask *task, void *param);

#endif // COOP_SYNC_H
