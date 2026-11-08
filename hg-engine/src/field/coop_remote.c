#include "config.h"
#include "types.h"
#include "constants/maps.h"
#include "constants/species.h"
#include "coop_sync.h"

// Overlay 131 since Milestone 5 Phase 0 (docs/MILESTONE5_PLAN.md §11.3): only field code
// (ghosts, sessions, trades, team bosses) reads remote players, so this left the resident
// Overlay 129 (src/coop_sync.c) to make room.

// ---------------------------------------------------------------------------
// Remote data (untrusted: written by the bridge from relay packets)
// ---------------------------------------------------------------------------

static BOOL Coop_IsValidDirection(u8 direction)
{
    return direction <= 3; // DIR_NORTH, DIR_SOUTH, DIR_WEST, DIR_EAST
}

static void Coop_Zero(void *dst, u32 size)
{
    u8 *bytes = dst;
    u32 i;

    for (i = 0; i < size; i++)
        bytes[i] = 0;
}

// Snapshot copy: the bridge only writes between frames, but copying first
// guarantees the checks below apply to exactly the bytes the caller uses.
static void Coop_CopyFromMailbox(void *dst, const volatile void *src, u32 size)
{
    u8 *d = dst;
    const volatile u8 *s = src;
    u32 i;

    for (i = 0; i < size; i++)
        d[i] = s[i];
}

#define COOP_MAX_HELD_MOVEMENT 0x71 // MapObject_SetHeldMovement asserts below this
#define COOP_MOVE_ACTION_NONE  0xFF // no action since the map loaded

static BOOL Coop_IsValidMoveAction(u8 action)
{
    return action < COOP_MAX_HELD_MOVEMENT || action == COOP_MOVE_ACTION_NONE;
}
#define COOP_MAX_PLAYER_STATE  14   // pret PLAYER_STATE_ROCKET_SAVING
#define COOP_MAX_ANIMATION     15   // player pose animations are small indexes

BOOL LONG_CALL Coop_ReadRemotePlayer(u32 remoteSlot, struct CoopPlayerState *out)
{
    Coop_Zero(out, sizeof(*out));
    if (remoteSlot >= COOP_REMOTE_SLOTS)
        return FALSE;

    Coop_CopyFromMailbox(out, &gCoopCommBlock.remote_players[remoteSlot], sizeof(*out));
    if (out->trainer_id == 0 // empty slot, or still at the title screen
        || out->map_id == 0 || out->map_id >= MAP_ID_MAX
        || !Coop_IsValidDirection(out->direction)
        || out->avatar_state > COOP_MAX_PLAYER_STATE
        || !Coop_IsValidMoveAction(out->move_action)
        || out->is_active > 1 || out->client_ready > 1 || out->gender > 1
        || out->save_status > COOP_SAVE_IN_PROGRESS
        || out->activity > COOP_ACTIVITY_EVENT)
    {
        Coop_Zero(out, sizeof(*out));
        return FALSE;
    }
    if (out->anim > COOP_MAX_ANIMATION)
        out->anim = 0; // cosmetic: show the pose's first animation instead of dropping the player
    if (out->field_busy > 1)
        out->field_busy = 1;
    if (out->follower_visible > 1 || out->follower_shiny > 1 || out->follower_gender > 2
        || out->follower_species > MAX_SPECIES_INCLUDING_FORMS
        || (out->follower_visible && out->follower_species == 0)
        || !Coop_IsValidDirection(out->follower_direction)
        || !Coop_IsValidMoveAction(out->follower_move_action))
    {
        // Show the player without a follower rather than not at all.
        Coop_Zero(&out->follower_species, __builtin_offsetof(struct CoopPlayerState, avatar_id)
                                              - __builtin_offsetof(struct CoopPlayerState, follower_species));
    }
    return TRUE;
}

