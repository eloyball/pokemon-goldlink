// SPDX-License-Identifier: CC-BY-NC-SA-4.0 OR MIT
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
#define COOP_MAILBOX_VERSION 7
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

// Map object IDs reserved for remote players' ghosts (0xE0-0xE4, ghost of
// remote slot k = 0xE0 + k) and their followers (0xE5-0xE9), plus the debug
// test ghost (0xEA). Vanilla maps use IDs <= 50; 249-255 are reserved by
// vanilla scripts (photo subject, daycare, apricorn, partner, player).
#define COOP_GHOST_ID_FIRST    0xE0
#define COOP_FOLLOWER_ID_FIRST 0xE5
#define COOP_TEST_GHOST_ID     0xEA // debug tools (diagnostics card)
#define COOP_IS_GHOST_ID(id)   ((id) >= COOP_GHOST_ID_FIRST && (id) <= COOP_TEST_GHOST_ID)

// Overlay 131 (OVERLAY_FIELD_EXTENSION, src/field/) is loaded together with
// the field overlay. Overlay 129 links before it, so it cannot call 131 by
// symbol: src/field/linker.ld pins this header at the start of Overlay 131
// and Coop_MainSysTask calls through it.
#define COOP_FIELD_ENTRY_ADDR  0x023C4000 // src/field/linker.ld asserts it
#define COOP_FIELD_ENTRY_MAGIC 0x31464F43 // "COF1"

struct FieldSystem;
struct CoopChallengeSync;
struct CoopWirelessWork;
struct CoopLinkBlock;
struct BattleSetup;
struct CoopFieldEntryHeader
{
    u32 magic;
    void (*entry)(struct FieldSystem *fsys); // runs once per frame while the field map is active
    u32 (*wildLevel)(u32 level);             // 0x08: Kanto wild levels (src/field/coop_wild.c), called by asm/other_hook.s
    struct CoopChallengeSync *challenge;     // 0x0C: Milestone 5 sync block (coop_challenge.h), read by the bridge
    u32 (*giftSpecies)(u32 value);           // 0x10: Milestone 5 gift randomizer, called by GiveMon (src/pokemon.c)
    u32 (*roamerSpecies)(u32 idx, u32 vanilla); // 0x14: Milestone 5 roamer randomizer (src/field_roamer.c)
    u32 (*playerSprite)(u32 state, u32 gender); // 0x18: the local player's sprite in its avatar (Coop_PlayerAvatar_GetSprite)
    void (*stampAvatar)(struct BattleSetup *setup); // 0x1C: Coop_StampBattleAvatar; unused since the boss setup moved to Overlay 131 (2026-11-06), kept for the layout
    u32 (*ballItem)(struct FieldSystem *fsys, u32 item); // 0x20: Milestone 6 random items (Coop_ScrCmd_CopyVar)
    void (*linkScan)(struct CoopWirelessWork *work, struct CoopLinkBlock *blk); // 0x24: the virtual link's scan report (src/field/coop_link_field.c)
};

// Custom avatars in battle (docs/CUSTOM_SPRITES_PLAN.md Phase 5): a player's
// avatar ID rides in their Trainer.loseMessage (Frontier-only; zeroed by
// BattleSetup_New) as { magic, avatar ID }; link battles exchange whole
// Trainer structs, so every game learns every player's avatar
// (src/battle/coop_avatar_battle.c reads it).
#define COOP_AVATAR_CARRIER_OFFSET 0x2C
#define COOP_AVATAR_CARRIER_MAGIC  0xC0A7 // never a valid easy-chat sentence type
void Coop_StampBattleAvatar(struct BattleSetup *setup);
_Static_assert(__builtin_offsetof(struct CoopFieldEntryHeader, challenge) == 0x0C,
               "the bridge reads the challenge pointer at 0x0C (Mailbox.FieldEntryChallenge)");

// DEVELOPMENT ONLY (docs/DEBUG_TOOLS.md): the bridge's debug commands below
// (test ghost, warp, bike / surf / fishing). Set to 0 before shipping: the
// ROM then compiles the handlers out and acknowledges every command with
// COOP_DEBUG_RESULT_DISABLED. The mailbox fields stay so the layout never
// changes between builds.
//
// Release builds (scripts/package_release.sh) put a coop_build_config.h that
// defines COOP_DEBUG_TOOLS 0 on the compiler's include path (C_INCLUDE_PATH);
// a normal build finds none and keeps the tools.
#if __has_include(<coop_build_config.h>)
#include <coop_build_config.h>
#endif
#ifndef COOP_DEBUG_TOOLS
#define COOP_DEBUG_TOOLS 1
#endif

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
    COOP_DEBUG_MON_EXPORT = 9,       // Milestone 3 probe A: party slot debug_arg -> gCoopDebugMonBuffer
    COOP_DEBUG_MON_IMPORT = 10,      // Milestone 3 probe A: validate gCoopDebugMonBuffer, add it to the party
    COOP_DEBUG_ARM_TEAM_BATTLE = 11, // Milestone 3 probe E: next trainer battle becomes player + AI partner vs the trainer twice
    COOP_DEBUG_LINK = 12,            // Milestone 3 Phase 3: debug_arg = enum CoopDebugLinkOp (vanilla CommState sessions)
    COOP_DEBUG_LINK_STATUS = 13,     // result COOP_DEBUG_LINK_STATUS_BASE | COOP_DEBUG_LINK_* flags; debug_arg: LinkSync value
    COOP_DEBUG_GIVE_TEST_MON = 14,   // a shiny test Pokemon (debug_arg 1 Haunter, 2 Onix + Metal Coat: trade evolvers; 3 / 4 the lead for the boss probe: level 40 Golem, Rock Slide / level 5 Magikarp, Splash; 5 / 6 the same, alone in the party; 7 a level 24 Geodude with Rock Slide, one EXP point short of 25, in place of the first; 8 a fainted Rattata added to the party; 9 Milestone 4 Phase 0: gCoopDebugMonBuffer's species / held_item / level, alone in the party; 10 rebalance §23: a level 27 Geodude with four moves, one EXP point short of 28 (Stealth Rock), in place of the first; 11 Milestone 6: like 9 but added to the party, Rock Slide first, the ability in mon[0..1]), then saves
    COOP_DEBUG_WILD_BATTLE = 15,     // §20 EXP Share probe: a wild battle against a level debug_arg Rattata
    COOP_DEBUG_GIVE_HMS = 16,        // §21 HM probe: debug_arg bit 0 the eight HMs, bit 1 all sixteen badges, bit 2 the eight Kanto badges, bit 3 FLAG_GAME_CLEAR
    COOP_DEBUG_ASK_HM = 17,          // §21 HM probe: result 0x80 | script slot << 3 | overworld slot (7: none) for HM debug_arg (0-7, the party menu's order)
    COOP_DEBUG_WILD_SPECIES = 18,    // Milestone 4 Phase 0: a wild battle against gCoopDebugMonBuffer.species (species | form << 11) at level debug_arg
    COOP_DEBUG_REBALANCE = 19,       // Milestone 4 Phase 1: debug_arg 0 = the lead's evolution with gCoopDebugMonBuffer.held_item (0: by level up) -> buffer.species (0: none); 1-4 = rebuild a department store list (Goldenrod 4F / 5F, Celadon 3F / 4F) from the save
    COOP_DEBUG_CHALLENGE = 20,       // Milestone 5 Phase 2: debug_arg 0-3 creates roamer n (Save_CreateRoamerByID) -> buffer.species = its species, buffer.level = SpeciesToRoamerIdx(species); 4 gives an Eevee through GiveMon (a script gift) -> buffer.species = the new last party member's species | form << 11; 5 / 6 the lead's ability set to Drought with / without the random-ability lock, then ResetPartyPokemonAbility -> buffer.species = the ability left; Phase 3: 7 gives three Master Balls; 7 / 8 -> buffer.held_item = Master Balls in the bag, party_count; 8 also buffer.species = the last party member's RESERVED_114 bits, level = its nickname flag; 9 deposits the last party member in box 1; Phase 4: 10 -> buffer.species = the level cap variable, level = GetLevelCap(), mon[0..3] = the lead's EXP; 11 -> buffer.species = 1 if a trade would refuse the last party member (Monolocke); Phase 5: 12 marks every party Pokemon dead, 13 every PC Pokemon outside the Graveyard; 14 (any time, read only) -> buffer.species = save state | spirit_flags << 8, held_item = anchor_map, level = living party count, party_count, mon[0..13] = u16 anchor_x, anchor_z, return_map, return_x, return_z, prebattle_map, deaths, mon[14] = the published challenge_flags, mon[15] = field_busy, mon[16] = 1 while the spirit task roams; Milestone 6: 21 puts the save back before Elm's starter (FLAG_GOT_STARTER clear, VAR_SCENE_ELMS_LAB 0, the starter flags reset), 22 -> mon[0..5] = the starter offer for trainer ID buffer.held_item under the save's rules; 23 starts Lyra's catching tutorial battle; 24 -> buffer.species = how many of item buffer.held_item the bag holds
    COOP_DEBUG_HEAP_FREE = 21,       // custom avatars (KNOWN_RISKS.md #80): mon[0..3] = the field heap's free bytes (u32; HEAP_ID_FIELD1), read only, any time in the field
};

enum CoopDebugLinkOp
{
    COOP_DEBUG_LINK_PARENT = 1,   // CommState_StartParentSession (single battle)
    COOP_DEBUG_LINK_CHILD = 2,    // CommState_StartChildSession (single battle)
    COOP_DEBUG_LINK_CONNECT = 3,  // CommState_ConnectToSlot(0): the first parent the scan listed
    COOP_DEBUG_LINK_SYNC = 4,     // LinkSync_StartSync(0x42)
    COOP_DEBUG_LINK_SHUTDOWN = 5, // CommState_RequestShutdown
    COOP_DEBUG_LINK_BATTLE_PARENT = 6, // Phase 4: the battle task as the parent (beacon id 1), no session request
    COOP_DEBUG_LINK_BATTLE_CHILD = 7,  // Phase 4: the battle task as the child
    COOP_DEBUG_LINK_REQUEST_BATTLE = 8, // §19: a real Battle request (the session task) to remote slot op - 8 (8..12)
    COOP_DEBUG_LINK_REQUEST_TRADE = 13, // stress probe: a real Trade request to remote slot op - 13 (13..17)
    COOP_DEBUG_LINK_REQUEST_REVIVE = 18, // Milestone 5 Phase 5: a Revive request to remote slot op - 18 (18..22), as the spirit's menu sends it
};

#define COOP_DEBUG_LINK_STATUS_BASE 0x40
#define COOP_DEBUG_LINK_EXISTS      0x01 // CommState_Exists
#define COOP_DEBUG_LINK_LINKED      0x02 // CommState_IsLinked
#define COOP_DEBUG_LINK_SYNCED      0x04 // LinkSync_IsSynced(debug_arg)
#define COOP_DEBUG_LINK_SYNC_VALUE  0x42

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
    COOP_DEBUG_RESULT_REJECTED = 8,   // Pokemon import: failed validation (checksum, species, level, egg)
};

// The game's PartyPokemon size. hg-engine's struct is 0xF0 (a u64 aligns it):
// copy this many bytes, never sizeof, and use Party_GetMonByIndex.
#define COOP_PARTY_MON_SIZE 0xEC

#if COOP_DEBUG_TOOLS
// DEVELOPMENT ONLY (docs/DEBUG_TOOLS.md; Milestone 3 Phase 0 probe A): one
// Pokemon in transit, exactly as the party stores it (encrypted, 236 bytes),
// plus what the ROM read from it (decrypted reads). Resident (Overlay 129
// .data); probes find it through build/rom_gen.ld.
struct CoopDebugMonBuffer
{
    u8  mon[COOP_PARTY_MON_SIZE];
    u16 species;
    u16 held_item;
    u8  level;
    u8  checksum_failed;
    u8  is_egg;
    u8  party_count; // after the command
    u8  team_battle_armed; // one-shot, cleared when used: 1 probe E (multi with an AI partner); 2 Milestone 4 Phase 0: the trainer's first Pokemon becomes species holding held_item; 3 Phase 3: the trainer scales like a Kanto leader (+2 per Kanto badge); 4 Milestone 5 Phase 3: the next wild Pokemon becomes species at level (Coop_ChallengeWildMon); 5 the same, shiny; 6 the slot rolled species (before the dupes clause)
    u8  team_battle_used;  // probe E: set when a battle was converted
    u8  pad[2];            // pad[0]: battle results; pad[1]: the Pokemon this player brought to a 2v2
    u8  auto_pick[3];      // probes: a 2v2's three (party slots, 1-based) instead of the party screen; 0: none
    u8  pad2;
    // Milestone 5 (m5_probe.lua wild, at +0xFC): what the last wild hook saw and did. Here,
    // resident, because Overlay 131's data is reloaded after every battle.
    u16 wild_original;     // species | form << 11 from the slot or script
    u16 wild_result;       // what joined the battle
    u16 wild_map;
    u8  wild_kind;         // 0 regular, 1 static, 2 safari / contest
    u8  wild_task;         // a field task or script was running (statics need one)
    u16 wild_ability;      // the random ability given (0 none)
    u16 wild_count;        // wild hooks seen since boot
    // Custom avatars Phase 6 (vs_intro probe): the PvP intro's faces (NARC a/1/0/9 palette
    // members) as armed, and how many intros finished since boot.
    u16 vs_player;
    u16 vs_opponent;
    u16 vs_count;
    // Milestone 6 §5.1 (m5_probe.lua items, at +0x10E): the last item ball's pick.
    u16 ball_vanilla;      // the ball's own item
    u16 ball_result;       // what it gave
    u16 ball_key;          // its event flag (the script number without a talked-to object)
    u16 ball_count;        // item-ball picks since boot
    // Milestone 6 §5.2 (m5_probe.lua alreadymet, at +0x116): the last HP-box icon drawn.
    u16 hp_icon_kind;      // 0 none, 1 caught, 2 already met here
    u16 hp_icon_count;     // icons drawn since boot
    u32 hp_icon_addr;      // the VRAM address of its tile
    // Link battle rework phase 0 (link_e2e_probe.lua, at +0x120): the battle script's waits for
    // the link queue (waitmessage) in link battles since boot, and the script steps they took.
    u16 link_waits;
    u8  link_wait_open;    // a wait in progress
    u8  pad_link;
    u32 link_wait_steps;
};
_Static_assert(__builtin_offsetof(struct CoopDebugMonBuffer, wild_original) == 0xFC, "m5_probe.lua reads the wild log at +0xFC");
_Static_assert(__builtin_offsetof(struct CoopDebugMonBuffer, ball_vanilla) == 0x10E, "m5_probe.lua reads the ball log at +0x10E");
_Static_assert(__builtin_offsetof(struct CoopDebugMonBuffer, hp_icon_kind) == 0x116, "m5_probe.lua reads the icon log at +0x116");
_Static_assert(__builtin_offsetof(struct CoopDebugMonBuffer, hp_icon_addr) == 0x11C, "m5_probe.lua reads the icon address at +0x11C");
_Static_assert(__builtin_offsetof(struct CoopDebugMonBuffer, link_waits) == 0x120, "link_e2e_probe.lua reads the wait counters at +0x120");
extern struct CoopDebugMonBuffer gCoopDebugMonBuffer;
struct BATTLE_PARAM;
void Coop_DebugTeamBattle(struct BATTLE_PARAM *bp, int num); // Overlay 131, called by hg-engine MakeTrainerPokemonParty
#endif

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
    COOP_WARP_VIOLET_GYM,    // Violet Gym, two tiles in front of Falkner: gym gate
    COOP_WARP_LANCE,         // the Champion's room, at its door: Lance as a team boss (rebalance plan §21)
    COOP_WARP_ELMS_LAB,      // Elm's lab, below the starter machine (Milestone 6 §3.2 probe; face north)
    COOP_WARP_R29_ITEM,      // Route 29, below its Potion ball (Milestone 6 §5.1 probe; face north)
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

// The adventure-start save (docs/MILESTONE2_PLAN.md 7.2): when the relay
// locks the roster, everyone still in the bedroom with a fresh save is told
// the adventure begins and the game is saved, so the room is tied to a save
// that exists on disk. Resets with the mailbox on a soft reset; a player who
// comes back to the bedroom with a still-fresh save just saves again.
enum CoopStartSaveState
{
    COOP_START_SAVE_NONE = 0,
    COOP_START_SAVE_RUNNING = 1,
    COOP_START_SAVE_DONE = 2,
};

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
    COOP_ACTIVITY_EVENT = 5,      // Milestone 6 §6.1: in the field, a script runs (talking, a sign, a cutscene)
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
    u8  boss_wait;           // 0x39 team boss this player waits at: sGatedBosses index + 1; 0 = none (Phase 5)
    u8  badge_mask[2];       // 0x3A the player's sixteen badges (little endian; bits 8-15 Kanto): a team's badge-scaled bosses use the lower count (rebalance plan §11)
    u8  avatar_id;           // 0x3C custom player sprite set (later milestone; 0 = vanilla)
    u8  anim;                // 0x3D map object animation (pret MapObject_SetAnimation): pose frames (fishing cast / bite / reel...)
    u8  field_busy;          // 0x3E 1 while a field task or message runs (NPC talk, start menu, co-op menus)
    u8  challenge_flags;     // 0x3F Milestone 5: COOP_PLAYER_FLAG_* (coop_challenge.h)
};

// Player-to-player sessions (docs/MILESTONE3_PLAN.md, Phase 1). Must match
// coop-relay/src/protocol.rs SESSION_*. The ROM latches one operation in
// CoopCommBlock.session_out (bumping its seq last); the relay applies each
// seq once and reports the outcome in the gates, bumping session_seq.
enum CoopSessionOp
{
    COOP_SESSION_OP_NONE = 0,
    COOP_SESSION_OP_REQUEST = 1,
    COOP_SESSION_OP_ACCEPT = 2,
    COOP_SESSION_OP_DECLINE = 3,
    COOP_SESSION_OP_CANCEL = 4, // withdraw my own request
};

enum CoopSessionKind
{
    COOP_SESSION_KIND_NONE = 0,
    COOP_SESSION_KIND_TRADE = 1,
    COOP_SESSION_KIND_BATTLE = 2,
    COOP_SESSION_KIND_TEAM_UP = 3,
    COOP_SESSION_KIND_LEAVE_TEAM = 4, // one-sided: no answer needed, the relay applies it at once
    // Two teams battle (2v2): the relay turns a BATTLE request between two
    // teamed players into this one and asks the other three. For the
    // requester's teammate, session_reason holds the opponent's remote slot.
    COOP_SESSION_KIND_TEAM_BATTLE = 5,
    // Milestone 5 Ghost Mode: give a Pokemon to a spirit. No answer: the
    // relay opens a one-sided trade at once (COOP_TRADE_FLAG_REVIVE).
    COOP_SESSION_KIND_REVIVE = 6,
};

enum CoopSessionState
{
    COOP_SESSION_NONE = 0,
    COOP_SESSION_OUTGOING = 1, // my request, waiting for the answer
    COOP_SESSION_INCOMING = 2, // someone asks me
    COOP_SESSION_ACCEPTED = 3,
    COOP_SESSION_DECLINED = 4,
    COOP_SESSION_CANCELLED = 5, // withdrawn, or the other side left
    COOP_SESSION_TIMEOUT = 6,
    COOP_SESSION_REFUSED = 7,   // turned down by the relay (session_reason)
    COOP_SESSION_NOTICE = 8,    // to the ex-teammate: the peer left our team (kind LEAVE_TEAM)
    COOP_SESSION_STATE_MAX = 8,
};

enum CoopSessionReason
{
    COOP_SESSION_REASON_NONE = 0,
    COOP_SESSION_REASON_NOT_HERE = 1,
    COOP_SESSION_REASON_BUSY = 2,
    COOP_SESSION_REASON_TEAM_RULE = 3,      // a team only battles a team, never its own teammate
    COOP_SESSION_REASON_ALREADY_TEAMED = 4,
    COOP_SESSION_REASON_INVALID = 5,
    COOP_SESSION_REASON_TEAMMATE_AWAY = 6,  // a team battle needs all four players here
};

#define COOP_SLOT_NONE        0xFF // team_partner_slot / session_peer_slot: nobody
#define COOP_TEAMED_SELF_BIT  0x80 // teamed_mask: this player is in a team (bits 0-4: remote slots)

// 8-byte latched session operation (ROM -> bridge -> relay, client packet 112..120).
struct CoopSessionOut
{
    u8  seq;            // 0x00 bumped last, once the rest is written
    u8  op;             // 0x01 enum CoopSessionOp
    u8  kind;           // 0x02 enum CoopSessionKind (REQUEST only)
    u8  pad;            // 0x03
    u32 target_trainer; // 0x04 trainer ID of the player asked (REQUEST only)
};

// Trading (docs/MILESTONE3_PLAN.md, Phase 2). While a trade task runs, the
// ROM owns a CoopTradeExchange on the field heap and publishes its address in
// CoopCommBlock.exchange_ptr. The bridge appends `out` to every client packet
// (bytes 120..364) and copies the relay's per-recipient trailer (server packet
// bytes 360..612) into `in`, or clears `in` when the relay sent none. Must
// match coop-relay/src/protocol.rs TRADE_*.
#define COOP_EXCHANGE_MAGIC   0x31585443 // "CTX1"
#define COOP_EXCHANGE_VERSION 1

enum CoopTradeOutState
{
    COOP_TRADE_OUT_NONE = 0,      // block open, nothing chosen yet
    COOP_TRADE_OUT_PICKING = 1,
    COOP_TRADE_OUT_OFFERED = 2,   // mon holds my offer (offer_id)
    COOP_TRADE_OUT_CONFIRMED = 3, // I accept the partner's offer confirm_id
    COOP_TRADE_OUT_CANCEL = 4,    // withdraw, or acknowledge an abort
    COOP_TRADE_OUT_DONE = 5,      // trade_id applied and saved
};

enum CoopTradeInState
{
    COOP_TRADE_IN_NONE = 0,       // the relay holds no trade for me
    COOP_TRADE_IN_WAITING = 1,    // trade open, the partner has not offered
    COOP_TRADE_IN_OFFERED = 2,    // mon holds the partner's offer (partner_offer_id)
    COOP_TRADE_IN_COMMIT = 3,     // swap now: give give_pid, receive mon (also after a crash)
    COOP_TRADE_IN_ABORTED = 4,    // reason
    COOP_TRADE_IN_STATE_MAX = 4,
};

// CoopTradeIn.flags: a revive (Milestone 5 Ghost Mode) is one-sided: the
// donor gives, the spirit (RECEIVER) only takes (docs/MILESTONE5_PLAN.md §5.5.4).
#define COOP_TRADE_FLAG_REVIVE   0x01
#define COOP_TRADE_FLAG_RECEIVER 0x02

enum CoopTradeAbortReason
{
    COOP_TRADE_ABORT_NONE = 0,
    COOP_TRADE_ABORT_CANCELLED = 1, // the partner cancelled (or rejected my Pokemon)
    COOP_TRADE_ABORT_LEFT = 2,      // the partner disconnected
    COOP_TRADE_ABORT_TIMEOUT = 3,
};

struct CoopTradeOut // ROM -> relay, 244 bytes
{
    u8  state;      // 0x00 enum CoopTradeOutState
    u8  offer_id;   // 0x01 bumped for every new offer
    u8  confirm_id; // 0x02 CONFIRMED: the partner_offer_id I accept
    u8  pad;        // 0x03
    u32 trade_id;   // 0x04 DONE: the commit applied
    u8  mon[COOP_PARTY_MON_SIZE]; // 0x08 my offer, encrypted as in the party
};

struct CoopTradeIn // relay -> ROM, 252 bytes
{
    u8  state;            // 0x00 enum CoopTradeInState
    u8  reason;           // 0x01 enum CoopTradeAbortReason
    u8  partner_offer_id; // 0x02
    u8  my_offer_id;      // 0x03 the offer of mine the relay holds
    u8  partner_confirmed;// 0x04 the partner accepted my current offer
    u8  peer_slot;        // 0x05 the partner's remote slot (COOP_SLOT_NONE: offline)
    u8  flags;            // 0x06 COOP_TRADE_FLAG_* (Milestone 5)
    u8  pad;              // 0x07
    u32 trade_id;         // 0x08 COMMIT
    u32 give_pid;         // 0x0C COMMIT: personality of the Pokemon I give
    u8  mon[COOP_PARTY_MON_SIZE]; // 0x10 OFFERED: the partner's offer; COMMIT: what I receive
};

struct CoopTradeExchange
{
    u32 magic;               // 0x000 COOP_EXCHANGE_MAGIC
    u16 version;             // 0x004 COOP_EXCHANGE_VERSION
    u16 size;                // 0x006 sizeof(struct CoopTradeExchange)
    struct CoopTradeOut out; // 0x008 written by the ROM
    struct CoopTradeIn in;   // 0x0FC written by the bridge
};

// Virtual link (docs/MILESTONE3_PLAN.md §15). While a local-wireless comm
// session exists, src/coop_link.c runs it over the network: the block below
// sits in the wireless manager's work buffer (the WM system buffer, unused)
// and is published in CoopCommBlock.exchange_ptr (trades use it too, never
// at the same time; the magic tells them apart). Single writer per field;
// the bridge writes between frames only.
//
//   out ring  ROM -> bridge: my messages (head: ROM, tail: bridge)
//   in ring   bridge -> ROM: the others' messages (head: bridge, tail: ROM)
//   mine      ROM: my beacon while I am a parent
//   seen      bridge: the beacons of the other players in the room
#define COOP_LINK_MAGIC       0x314B4C43 // "CLK1"
#define COOP_LINK_VERSION     1
#define COOP_LINK_DATA_MAX    0xC0 // a parent frame (192 bytes, port 14)
#define COOP_LINK_BEACON_MAX  0x70 // WMGameInfo user data (112 bytes)
#define COOP_LINK_OUT_SLOTS   4
#define COOP_LINK_IN_SLOTS    10
#define COOP_LINK_SEEN_SLOTS  COOP_REMOTE_SLOTS

enum CoopLinkMsgType
{
    COOP_LINK_MSG_NONE = 0,
    COOP_LINK_MSG_DATA = 1,    // a piece of a comm command stream (src/coop_link.c): out: to every other
                               // member of the link group; in: aid = sender
    COOP_LINK_MSG_CONNECT = 2, // out (child): data[0] = the parent's player id, data[1..4] = connect data;
                               // in (parent): aid = the child's aid, data[0..3] = connect data
    COOP_LINK_MSG_ACCEPT = 3,  // out (parent): aid = the child; in (child): aid = mine
    COOP_LINK_MSG_REFUSE = 4,  // out (parent): aid = the child; in (child)
    COOP_LINK_MSG_BYE = 5,     // out: I left; in: aid = who left
};

// gCoopCommBlock.track_link_battle: a co-op link battle and how it ended.
enum CoopLinkBattleState
{
    COOP_LINK_BATTLE_NONE = 0,
    COOP_LINK_BATTLE_RUNNING = 1, // set by the battle task right before the battle
    COOP_LINK_BATTLE_LOST = 2,    // the link dropped: the watchdog ended the battle (a draw)
};

enum CoopLinkRole
{
    COOP_LINK_ROLE_NONE = 0,
    COOP_LINK_ROLE_PARENT = 1,
    COOP_LINK_ROLE_CHILD = 2,
};

struct CoopLinkMsg
{
    u8  type;                     // 0x00 enum CoopLinkMsgType
    u8  aid;                      // 0x01
    u16 length;                   // 0x02 bytes of data used
    u8  data[COOP_LINK_DATA_MAX]; // 0x04
};

struct CoopLinkBeacon
{
    u32 ggid;                       // 0x00
    u8  length;                     // 0x04 bytes of data (0: none)
    u8  entry;                      // 0x05 the parent accepts children
    u8  player;                     // 0x06 seen: the parent's player id
    u8  seq;                        // 0x07 bumped on every change
    u8  data[COOP_LINK_BEACON_MAX]; // 0x08 the game's beacon (CommBeacon)
};

struct CoopLinkBlock
{
    u32 magic;                                  // 0x000 COOP_LINK_MAGIC
    u16 version;                                // 0x004 COOP_LINK_VERSION
    u16 size;                                   // 0x006 sizeof(struct CoopLinkBlock)
    u8  role;                                   // 0x008 ROM: enum CoopLinkRole
    u8  aid;                                    // 0x009 ROM: my aid (0: parent)
    u8  target;                                 // 0x00A ROM: child: the parent's player id (0xFF none)
    u8  linked;                                 // 0x00B ROM: 1 while connected (parent: a child joined)
    u8  out_head;                               // 0x00C ROM
    u8  out_tail;                               // 0x00D bridge
    u8  in_head;                                // 0x00E bridge
    u8  in_tail;                                // 0x00F ROM
    struct CoopLinkBeacon mine;                 // 0x010 ROM
    struct CoopLinkBeacon seen[COOP_LINK_SEEN_SLOTS]; // 0x088 bridge
    struct CoopLinkMsg out[COOP_LINK_OUT_SLOTS];      // 0x2E0 ROM
    struct CoopLinkMsg in[COOP_LINK_IN_SLOTS];        // 0x5F0 bridge
    // 0xD98: ROM private (src/coop_link.c)
    u32 pending;                                // 0xD98 callbacks due next frame
    u8  unused_d9c[17];                         // 0xD9C (was the radio's send callbacks)
    u8  scan_next;                              // 0xDAD next seen slot the fake scan reports
    u16 timer;                                  // 0xDAE child: frames waiting for an answer; parent: beacon events
};

// 24-byte progression gates block (relay -> ROM).
struct CoopProgressionGates
{
    u16 badge_mask_local_echo; // 0x00
    u16 badge_mask_synced;     // 0x02
    u8  flag_coop_sync_ready;  // 0x04
    u8  current_checkpoint;    // 0x05
    u8  assigned_monolocke;    // 0x06
    u8  challenge_room_state;  // 0x07 Milestone 5: COOP_ROOM_* bits (coop_challenge.h)
    u8  flag_campaign_started; // 0x08
    u8  expected_party_size;   // 0x09
    u8  online_party_count;    // 0x0A
    u8  ready_party_count;     // 0x0B
    u32 room_uid;              // 0x0C random per room, never reused (keys per-room saves)
    u8  team_partner_slot;     // 0x10 my teammate's remote slot; COOP_SLOT_NONE: none or offline
    u8  teamed_mask;           // 0x11 bit per remote slot in a team; COOP_TEAMED_SELF_BIT: me
    u8  session_state;         // 0x12 enum CoopSessionState
    u8  session_kind;          // 0x13 enum CoopSessionKind
    u8  session_peer_slot;     // 0x14 the other side's remote slot; COOP_SLOT_NONE: gone
    u8  session_seq;           // 0x15 bumped on every change of the session fields
    u8  session_reason;        // 0x16 enum CoopSessionReason (REFUSED)
    u8  trade_pending;         // 0x17 1: the relay holds a trade for me (open, to apply, or an abort to read)
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
    u8  start_save_state;                                       // 0x1AF ROM: enum CoopStartSaveState
    u32 heartbeat_bridge;                                       // 0x1B0
    u32 local_player_seq;                                       // 0x1B4 odd while local_player is being written
    u32 bridge_hb_last_seen;                                    // 0x1B8
    u16 bridge_stale_frames;                                    // 0x1BC
    u8  local_avatar;                                           // 0x1BE bridge: this player's avatar (docs/CUSTOM_SPRITES_PLAN.md; 0 the save's own)
    u8  pad1BF;                                                 // 0x1BF
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
    u8  track_field_idle;                                       // 0x1E8 ROM private: no field task / message last frame (ghost talk)
    u8  track_follower_seq[COOP_REMOTE_SLOTS];                  // 0x1E9 ROM private: last follower move_seq each follower ghost handled
    u8  track_follower_shiny;                                   // 0x1EE ROM private: bit per slot, shiny sequence applied to its follower ghost
    u8  track_field_hidden;                                     // 0x1EF ROM private: the field was covered (menu, battle, app) since the last field tick
    struct CoopSessionOut session_out;                          // 0x1F0 ROM: latched session operation (Milestone 3)
    u8  track_session_answered;                                 // 0x1F8 ROM private: session_seq of the pending request already prompted (0: none pending)
    u8  track_link_battle;                                      // 0x1F9 ROM private: enum CoopLinkBattleState (Phase 4)
    u8  track_link_lost_frames;                                 // 0x1FA ROM private: frames since the link of a battle was lost
    u8  track_boss_wait;                                        // 0x1FB ROM private: published as local_player.boss_wait
    u32 exchange_ptr;                                           // 0x1FC ROM: address of the open CoopTradeExchange, 0 when none
};

#define COOP_ASSERT_OFFSET(type, field, off) \
    _Static_assert(__builtin_offsetof(struct type, field) == (off), #type "." #field " must be at " #off)

_Static_assert(sizeof(struct CoopPlayerState) == 64, "CoopPlayerState must be 64 bytes");
_Static_assert(sizeof(struct CoopProgressionGates) == 24, "CoopProgressionGates must be 24 bytes");
_Static_assert(sizeof(struct CoopCommBlock) == 512, "CoopCommBlock must be 512 bytes");
_Static_assert(sizeof(struct CoopDiagnostics) == 24, "CoopDiagnostics must be 24 bytes");
_Static_assert(sizeof(struct CoopSessionOut) == 8, "CoopSessionOut must be 8 bytes");
_Static_assert(sizeof(struct CoopTradeOut) == 244, "CoopTradeOut must be 244 bytes");
_Static_assert(sizeof(struct CoopTradeIn) == 252, "CoopTradeIn must be 252 bytes");
_Static_assert(sizeof(struct CoopTradeExchange) == 0x1F8, "CoopTradeExchange must be 504 bytes");

COOP_ASSERT_OFFSET(CoopPlayerState, matrix_id, 0x02);
COOP_ASSERT_OFFSET(CoopPlayerState, height, 0x08);
COOP_ASSERT_OFFSET(CoopPlayerState, sprite_id, 0x0C);
COOP_ASSERT_OFFSET(CoopPlayerState, boss_wait, 0x39);
COOP_ASSERT_OFFSET(CoopPlayerState, move_seq, 0x0F);
COOP_ASSERT_OFFSET(CoopPlayerState, activity, 0x13);
COOP_ASSERT_OFFSET(CoopPlayerState, trainer_id, 0x14);
COOP_ASSERT_OFFSET(CoopPlayerState, name, 0x18);
COOP_ASSERT_OFFSET(CoopPlayerState, follower_species, 0x28);
COOP_ASSERT_OFFSET(CoopPlayerState, follower_x, 0x30);
COOP_ASSERT_OFFSET(CoopPlayerState, follower_move_seq, 0x38);
COOP_ASSERT_OFFSET(CoopPlayerState, avatar_id, 0x3C);
COOP_ASSERT_OFFSET(CoopPlayerState, anim, 0x3D);
COOP_ASSERT_OFFSET(CoopPlayerState, field_busy, 0x3E);
COOP_ASSERT_OFFSET(CoopProgressionGates, flag_campaign_started, 0x08);
COOP_ASSERT_OFFSET(CoopProgressionGates, room_uid, 0x0C);
COOP_ASSERT_OFFSET(CoopProgressionGates, team_partner_slot, 0x10);
COOP_ASSERT_OFFSET(CoopProgressionGates, session_state, 0x12);
COOP_ASSERT_OFFSET(CoopProgressionGates, session_seq, 0x15);
COOP_ASSERT_OFFSET(CoopProgressionGates, session_reason, 0x16);
COOP_ASSERT_OFFSET(CoopSessionOut, target_trainer, 0x04);
COOP_ASSERT_OFFSET(CoopProgressionGates, trade_pending, 0x17);
COOP_ASSERT_OFFSET(CoopTradeOut, mon, 0x08);
COOP_ASSERT_OFFSET(CoopTradeIn, trade_id, 0x08);
COOP_ASSERT_OFFSET(CoopTradeIn, mon, 0x10);
COOP_ASSERT_OFFSET(CoopTradeExchange, out, 0x008);
COOP_ASSERT_OFFSET(CoopTradeExchange, in, 0x0FC);
_Static_assert(sizeof(struct CoopLinkMsg) == 0xC4, "CoopLinkMsg must be 196 bytes");
_Static_assert(sizeof(struct CoopLinkBeacon) == 0x78, "CoopLinkBeacon must be 120 bytes");
_Static_assert(sizeof(struct CoopLinkBlock) == 0xDB0, "CoopLinkBlock must be 0xDB0 bytes");
COOP_ASSERT_OFFSET(CoopLinkBlock, mine, 0x010);
COOP_ASSERT_OFFSET(CoopLinkBlock, seen, 0x088);
COOP_ASSERT_OFFSET(CoopLinkBlock, out, 0x2E0);
COOP_ASSERT_OFFSET(CoopLinkBlock, in, 0x5F0);
COOP_ASSERT_OFFSET(CoopLinkBlock, pending, 0xD98);
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
COOP_ASSERT_OFFSET(CoopCommBlock, local_avatar, 0x1BE);
COOP_ASSERT_OFFSET(CoopCommBlock, limit_map_id, 0x1C0);
COOP_ASSERT_OFFSET(CoopCommBlock, limit_species, 0x1C2);
COOP_ASSERT_OFFSET(CoopCommBlock, diag, 0x1C4);
COOP_ASSERT_OFFSET(CoopCommBlock, debug_cmd, 0x1DC);
COOP_ASSERT_OFFSET(CoopCommBlock, debug_result, 0x1DF);
COOP_ASSERT_OFFSET(CoopCommBlock, save_counter, 0x1E0);
COOP_ASSERT_OFFSET(CoopCommBlock, save_slot_ready, 0x1E4);
COOP_ASSERT_OFFSET(CoopCommBlock, reset_request, 0x1E5);
COOP_ASSERT_OFFSET(CoopCommBlock, track_player_step, 0x1E6);
COOP_ASSERT_OFFSET(CoopCommBlock, track_field_idle, 0x1E8);
COOP_ASSERT_OFFSET(CoopCommBlock, track_follower_seq, 0x1E9);
COOP_ASSERT_OFFSET(CoopCommBlock, track_follower_shiny, 0x1EE);
COOP_ASSERT_OFFSET(CoopCommBlock, track_field_hidden, 0x1EF);
COOP_ASSERT_OFFSET(CoopCommBlock, session_out, 0x1F0);
COOP_ASSERT_OFFSET(CoopCommBlock, track_session_answered, 0x1F8);
COOP_ASSERT_OFFSET(CoopCommBlock, track_link_battle, 0x1F9);
COOP_ASSERT_OFFSET(CoopCommBlock, track_link_lost_frames, 0x1FA);
COOP_ASSERT_OFFSET(CoopCommBlock, track_boss_wait, 0x1FB);
COOP_ASSERT_OFFSET(CoopCommBlock, exchange_ptr, 0x1FC);
COOP_ASSERT_OFFSET(CoopDiagnostics, test_ghost_x, 0x10);
COOP_ASSERT_OFFSET(CoopDiagnostics, player_y, 0x16);

extern volatile struct CoopCommBlock gCoopCommBlock;

struct SysTask;
struct TaskManager;

// Field tasks (Overlay 131, src/field/coop_field_tasks.c). While one runs,
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

// Talking to a ghost (Overlay 131 detects the A press): "It's NAME!" plus a
// second page when the player is in a battle or a menu (activity, enum
// CoopActivity). name is HG-encoded, up to 8 chars, 0xFFFF-terminated when
// shorter. FALSE when a field task could not be started.
BOOL LONG_CALL Coop_StartTalkDialog(struct FieldSystem *fsys, const u16 *name, u32 activity);

// TRUE while the location banner is showing; co-op field tasks wait for it.
BOOL LONG_CALL Coop_IsMapNameBannerShowing(struct FieldSystem *fsys);

// End of every field frame (Overlay 131): starts the lobby and boss blocks,
// the adventure-start save and the disconnect save as the connection asks.
void LONG_CALL Coop_UpdateFieldConnection(struct FieldSystem *fsys);

// Validated copy of relay-provided data for ROM consumers (ghosts and their
// followers). Returns FALSE, leaving *out zeroed, when the slot is out of
// range, empty, or holds out-of-bounds values; the follower fields are
// zeroed alone when only they are invalid. A player outside the field
// (is_active 0: battle, menu) is still returned, so the ghost can freeze.
// Never read remote_players directly: the bridge and relay are untrusted.
// Overlay 131's pinned header while that overlay is loaded (magic checked), else NULL.
const struct CoopFieldEntryHeader *LONG_CALL Coop_FieldHeader(void);

// Overlay 131 (src/field/coop_remote.c): callable from field code only.
BOOL LONG_CALL Coop_ReadRemotePlayer(u32 remoteSlot, struct CoopPlayerState *out);

// 60 Hz background SysTask (created once in SaveData_New): heartbeat,
// bridge watchdog, local player telemetry and the mid-game disconnect latch.
void Coop_MainSysTask(struct SysTask *task, void *param);
void Coop_LinkSysTask(struct SysTask *task, void *param); // VWait queue: the virtual radio

#endif // COOP_SYNC_H
