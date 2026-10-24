#ifndef COOP_SYNC_H
#define COOP_SYNC_H

#include "types.h"

// 32-byte per-player state block (occupies 0x010..0x02F of CoopCommBlock).
// Must stay byte-for-byte identical to CoopBridgeForm.cs / protocol.rs.
struct CoopPlayerState
{
    u16 map_id;
    s16 x;
    s16 y;
    s16 prev_x;
    s16 prev_y;
    u8  direction;
    u8  movement_type;
    u16 sprite_id;
    u8  animation_frame;
    u8  is_active;
    u16 lead_species;
    u16 lead_form;
    u8  lead_is_shiny;
    u8  lead_gender;
    u8  map_npc_count;
    u8  client_ready;
    u32 trainer_id;
    u8  target_party_size;
    u8  pad[3];
};

// 512-byte shared mailbox between the ROM and the BizHawk coop-bridge.
// Offsets are frozen; do not reorder fields.
struct CoopCommBlock
{
    u32 magic;                       // 0x000 0x434F4F50 ("COOP")
    u16 version;                     // 0x004 protocol version (1)
    u8  assigned_player_id;          // 0x006 0xFF = unassigned, else 0..5
    u8  active_players_mask;         // 0x007 bitmask of connected slots
    u32 heartbeat_rom;               // 0x008 last ROM-side heartbeat tick
    u32 server_tick;                  // 0x00C relay server tick
    struct CoopPlayerState local_player; // 0x010..0x02F (32 bytes)
    u8  server_payload[280];         // 0x030..0x147 opaque relay payload
    u16 badge_mask_local;            // 0x148 local 16-bit badge mask
    u8  server_payload_tail[22];     // 0x14A..0x15F opaque relay payload tail
    volatile u8 is_connected_to_room; // 0x160 1 = in a room, 0 = disconnected
    volatile u8 is_boot_done;          // 0x161 startup gate armed (1 = boot complete)
    u8  reserved[158];                 // 0x162..0x1FF pad to exactly 512 bytes
};

_Static_assert(sizeof(struct CoopCommBlock) == 512, "CoopCommBlock must be 512 bytes");

extern volatile struct CoopCommBlock gCoopCommBlock;

BOOL LONG_CALL Coop_IsConnectedToRoom(void);
void LONG_CALL Coop_ShowDisconnectedGateAndReset(void);
void LONG_CALL Coop_CheckStartupGate(void);

// Forward declaration; the full type is defined in pokemon.h.
struct FieldSystem;
// Publish the local player's overworld state into gCoopCommBlock at
// 30-60 Hz for the BizHawk bridge / relay. Called once per overworld frame.
void LONG_CALL Coop_UpdateLocalPlayerState(struct FieldSystem *fsys);

#endif // COOP_SYNC_H
