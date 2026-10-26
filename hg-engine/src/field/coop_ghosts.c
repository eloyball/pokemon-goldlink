#include "config.h"
#include "types.h"

#include "constants/maps.h"

#include "map_events_internal.h"
#include "pokemon.h"
#include "save.h"
#include "task.h"
#include "coop_sync.h"

// ---------------------------------------------------------------------------
// Overlay 131 (OVERLAY_FIELD_EXTENSION): co-op field code.
//
// Loaded together with the field overlay, so everything here may assume the
// field map is running. Overlay 129 reaches this code only through the
// header below, which src/field/linker.ld pins at COOP_FIELD_ENTRY_ADDR.
//
// Milestone 2 Phase 0 probes (docs/MILESTONE2_PLAN.md, section 9):
//   1. renderer budget: free dynamic texture / model / sequence slots
//   2. the 129 -> 131 call itself (diag.field_entry_calls)
//   3. one test ghost driven from the bridge (flags, held movement, saves)
//   4. bridge height (test ghost spawned at the player's render height)
// plus the DEVELOPMENT-ONLY debug tools (docs/DEBUG_TOOLS.md): warp presets
// and bike / surf / fishing toggles, compiled only with COOP_DEBUG_TOOLS.
//
// It also builds and publishes the local player's telemetry every field
// frame (Coop_PublishLocalPlayerState below).
//
// Overlay 131 is loaded without BSS init, so no state lives in statics: the
// test ghost is looked up in the map object list every frame, and the command
// handshake and movement tracking live in gCoopCommBlock.
// ---------------------------------------------------------------------------

// pret asm/unk_02062108.s labels (rom.ld).
BOOL LONG_CALL MapObject_AreBitsSetForMovementScriptInit(LocalMapObject *object);
void LONG_CALL MapObject_SetHeldMovement(LocalMapObject *object, u32 movement);
BOOL LONG_CALL MapObject_ClearHeldMovementIfActive(LocalMapObject *object);

// pret map object flag bits (include/map_object.h).
#define COOP_MOFLAG_ACTIVE          (1u << 0)
#define COOP_MOFLAG_SINGLE_MOVEMENT (1u << 1)
#define COOP_MOFLAG_KEEP            (1u << 10) // survives zone changes in the matrix
#define COOP_MOFLAG_UNK13           (1u << 13) // set with KEEP on player / follower objects
#define COOP_MOFLAG_NO_COLLISION    (1u << 18) // skipped by sub_02060BFC
#define COOP_MOFLAG_NO_INTERACT     (1u << 19) // skipped by sub_0203DC64 (A button)
#define COOP_MOFLAG_UNK25           (1u << 25) // excluded by GetFirstActiveObjectByID

// pret constants/movements.h; MapObject_SetHeldMovement accepts IDs < 0x71.
#define COOP_MOVEMENT_STEP_UP        12 // + direction: up, down, left, right
#define COOP_MOVEMENT_JUMP_NEAR_FAST 52 // + direction (asm/macros/movement.inc)
#define COOP_MOVEMENT_EMOTE_EXCLAIM  75

#if COOP_DEBUG_TOOLS
// pret constants/global_fieldmap.h
#define COOP_PLAYER_STATE_WALKING      0
#define COOP_PLAYER_STATE_CYCLING      1
#define COOP_PLAYER_STATE_SURFING      2
#define COOP_PLAYER_TRANSITION_WALKING (1u << 0)
#define COOP_PLAYER_TRANSITION_CYCLING (1u << 1)
#define COOP_PLAYER_TRANSITION_SURFING (1u << 2)
#define COOP_DIR_SOUTH                 1
#define COOP_HEAP_ID_FIELD1            4 // pret constants/heap.h
#define COOP_ROD_TYPE_OLD              0 // pret include/overlay_01.h

// Debug-only vanilla functions (rom.ld, "Co-op DEBUG TOOLS only").
void *LONG_CALL sub_020538C0(FieldSystem *fsys, u32 mapId, int warpId, int x, int y, int direction);
void LONG_CALL Field_PlayerAvatar_OrrTransitionFlags(FIELD_PLAYER_AVATAR *avatar, u32 flags);
void LONG_CALL Field_PlayerAvatar_ApplyTransitionFlags(FIELD_PLAYER_AVATAR *avatar);
BOOL LONG_CALL Field_PlayerCanSurfOnTile(FIELD_PLAYER_AVATAR *avatar, int currentTile, int facingTile);
void *LONG_CALL CreateFishingRodTaskEnv(FieldSystem *fsys, u32 heapId, int rod);
BOOL LONG_CALL Task_OverworldFish(TaskManager *taskman);
#endif

#define COOP_RENDER_OFFSET 0x18 // MapObjectRender inline in MapObjectManager

// Render header of ccd include/field/map_object_render_internal.h. Only the
// counters and ID lists are read. Free slots hold 0xFF in the model and
// sequence lists and 0xFFFF in the texture list (ccd IdList_Fill calls).
struct CoopMapObjectRender
{
    u32 unused00;
    u32 objectCount;
    u32 residentModelCount;
    u32 dynamicModelCount;
    u32 residentSequenceCount;
    u32 dynamicSequenceCount;
    u32 residentTextureCount;
    u32 dynamicTextureCount;
    u32 modelIds[8];
    u32 sequenceIds[8];
    u32 textureIds[32];
};

_Static_assert(sizeof(LocalMapObject) == 0x12C, "LocalMapObject must be 0x12C bytes (pret map_object.h)");
_Static_assert(__builtin_offsetof(LocalMapObject, xCurr) == 0x64, "xCurr");
_Static_assert(__builtin_offsetof(LocalMapObject, hCurr) == 0x68, "hCurr");
_Static_assert(__builtin_offsetof(LocalMapObject, yCurr) == 0x6C, "yCurr (tile Z)");
_Static_assert(__builtin_offsetof(LocalMapObject, posVec) == 0x70, "posVec");
_Static_assert(__builtin_offsetof(struct CoopMapObjectRender, modelIds) == 0x20, "modelIds");
_Static_assert(__builtin_offsetof(struct CoopMapObjectRender, sequenceIds) == 0x40, "sequenceIds");
_Static_assert(__builtin_offsetof(struct CoopMapObjectRender, textureIds) == 0x60, "textureIds");

static void Coop_FieldEntry(FieldSystem *fsys);

__attribute__((section(".coop_field_entry"), used))
const struct CoopFieldEntryHeader gCoopFieldEntryHeader = {
    .magic = COOP_FIELD_ENTRY_MAGIC,
    .entry = Coop_FieldEntry,
};

// ---------------------------------------------------------------------------
// Probe 1: renderer budget
// ---------------------------------------------------------------------------

#define COOP_FREE_MODEL_ID    0xFF
#define COOP_FREE_SEQUENCE_ID 0xFF
#define COOP_FREE_TEXTURE_ID  0xFFFF

static u8 Coop_CountFreeSlots(const u32 *ids, u32 first, u32 count, u32 freeId)
{
    u32 i;
    u8 free = 0;

    for (i = first; i < first + count; i++)
    {
        if (ids[i] == freeId)
            free++;
    }
    return free;
}

static void Coop_ProbeRender(MapObjectMan *manager)
{
    volatile struct CoopDiagnostics *diag = &gCoopCommBlock.diag;
    const struct CoopMapObjectRender *render = (const void *)((const u8 *)manager + COOP_RENDER_OFFSET);

    // MapObjectManager_InitRender sets ACTIVE once the renderer exists.
    // Sanity-check the counts before indexing the ID lists.
    if (!(manager->flags & COOP_MOFLAG_ACTIVE)
        || render->residentModelCount + render->dynamicModelCount > 8
        || render->residentSequenceCount + render->dynamicSequenceCount > 8
        || render->residentTextureCount + render->dynamicTextureCount > 32)
    {
        diag->render_ready = 0;
        return;
    }

    diag->tex_resident = (u8)render->residentTextureCount;
    diag->tex_dynamic = (u8)render->dynamicTextureCount;
    diag->tex_dynamic_free = Coop_CountFreeSlots(render->textureIds, render->residentTextureCount, render->dynamicTextureCount, COOP_FREE_TEXTURE_ID);
    diag->model_dynamic = (u8)render->dynamicModelCount;
    diag->model_dynamic_free = Coop_CountFreeSlots(render->modelIds, render->residentModelCount, render->dynamicModelCount, COOP_FREE_MODEL_ID);
    diag->seq_dynamic = (u8)render->dynamicSequenceCount;
    diag->seq_dynamic_free = Coop_CountFreeSlots(render->sequenceIds, render->residentSequenceCount, render->dynamicSequenceCount, COOP_FREE_SEQUENCE_ID);
    diag->map_objects_active = (u8)manager->active_count;
    diag->render_ready = 1;
}

// ---------------------------------------------------------------------------
// Probes 3 and 4: test ghost
// ---------------------------------------------------------------------------

// pret MapObjectManager_GetFirstActiveObjectByID, walked here instead of
// trusting the unused rom.ld mapping.
static LocalMapObject *Coop_FindObjectById(MapObjectMan *manager, u32 id)
{
    LocalMapObject *object = manager->objects;
    u32 i;

    for (i = 0; i < manager->object_count; i++, object++)
    {
        if ((object->flags & COOP_MOFLAG_ACTIVE) && !(object->flags & COOP_MOFLAG_UNK25) && (u32)object->id == id)
            return object;
    }
    return NULL;
}

static s16 Coop_RenderHeight(const LocalMapObject *object)
{
    return (s16)((s32)object->posVec[1] >> 12); // fx32 -> world units
}

#if COOP_DEBUG_TOOLS
// Spawning and driving the test ghost is a debug tool; reporting it (below)
// stays so the diagnostics card works in every build.

static void Coop_DirectionDelta(u32 direction, s32 *dx, s32 *dz)
{
    *dx = 0;
    *dz = 0;
    switch (direction)
    {
    case 0: *dz = -1; break; // DIR_NORTH
    case 1: *dz = 1;  break; // DIR_SOUTH
    case 2: *dx = -1; break; // DIR_WEST
    case 3: *dx = 1;  break; // DIR_EAST
    }
}

// Ready for a new held movement (pret ScrCmd_ApplyMovement preconditions).
static BOOL Coop_GhostIsIdle(LocalMapObject *ghost)
{
    return MapObject_ClearHeldMovementIfActive(ghost) && MapObject_AreBitsSetForMovementScriptInit(ghost);
}

static BOOL Coop_SpawnTestGhost(MapObjectMan *manager, LocalMapObject *player)
{
    LocalMapObject *ghost;
    s32 dx, dz;
    u32 direction = (u32)player->curFacing;

    if (manager->active_count + 4 >= manager->object_count)
        return FALSE; // keep headroom for vanilla objects

    Coop_DirectionDelta(direction, &dx, &dz);
    // Same sprite as the local player: no extra texture / model / sequence.
    ghost = CreateSpecialFieldObject(manager, (u32)(player->xCurr + dx), (u32)(player->yCurr + dz),
                                     direction, (u32)player->gfxId, 0, MAP_NOTHING);
    if (ghost == NULL)
        return FALSE;

    ghost->id = COOP_TEST_GHOST_ID;
    MapObject_SetBits(ghost, COOP_MOFLAG_KEEP | COOP_MOFLAG_UNK13 | COOP_MOFLAG_NO_COLLISION | COOP_MOFLAG_NO_INTERACT);

    // Probe 4: MapObject_Create snaps to the ground; copy the player's height
    // so a ghost spawned on a bridge starts on the bridge.
    ghost->hPrev = player->hCurr;
    ghost->hCurr = player->hCurr;
    ghost->posVec[1] = player->posVec[1];
    return TRUE;
}

// ---------------------------------------------------------------------------
// DEVELOPMENT ONLY: field debug tools (docs/DEBUG_TOOLS.md)
// ---------------------------------------------------------------------------

struct CoopWarpTarget
{
    u16 mapId;
    u16 x;
    u16 z;
};

// Vanilla fly landing spots (ccd src/spawn_warp.c sSpawnPoints fly columns;
// Route 40 is the Frontier spawn's special warp), in world tile coordinates.
// hg-engine names maps by their internal codes; pret names in comments.
static const struct CoopWarpTarget sCoopWarpTargets[COOP_WARP_COUNT] = {
    [COOP_WARP_NEW_BARK]    = { MAP_T20, 695,  397 }, // MAP_NEW_BARK
    [COOP_WARP_CHERRYGROVE] = { MAP_T21, 564,  392 }, // MAP_CHERRYGROVE
    [COOP_WARP_VIOLET]      = { MAP_T22, 497,  272 }, // MAP_VIOLET
    [COOP_WARP_GOLDENROD]   = { MAP_T25, 352,  369 }, // MAP_GOLDENROD
    [COOP_WARP_ECRUTEAK]    = { MAP_T27, 397,  184 }, // MAP_ECRUTEAK
    [COOP_WARP_OLIVINE]     = { MAP_T26, 272,  258 }, // MAP_OLIVINE
    [COOP_WARP_ROUTE_40]    = { MAP_W40, 237,  267 }, // MAP_ROUTE_40
    [COOP_WARP_CERULEAN]    = { MAP_T04, 1309, 132 }, // MAP_CERULEAN
    [COOP_WARP_LAVENDER]    = { MAP_T05, 1418, 235 }, // MAP_LAVENDER
    [COOP_WARP_CELADON]     = { MAP_T07, 1231, 238 }, // MAP_CELADON
};

// Same check as using Surf / a rod from the menu (Field_PlayerCanSurfOnTile
// is what both the Surf field move and ItemCheckUseFunc_FishingRod test).
static BOOL Coop_IsFacingSurfableWater(FieldSystem *fsys, LocalMapObject *player)
{
    s32 dx, dz;
    u8 current, facing;

    Coop_DirectionDelta((u32)player->curFacing, &dx, &dz);
    current = GetMetatileBehaviorAt(fsys, player->xCurr, player->yCurr);
    facing = GetMetatileBehaviorAt(fsys, player->xCurr + dx, player->yCurr + dz);
    return Field_PlayerCanSurfOnTile(fsys->playerAvatar, current, facing);
}

// Task_Surf (ccd player_avatar_actions.c) without the party cut-in and cry,
// so it works without a Pokemon: mount (surf sprite + bobbing mount under
// the player), then jump onto the faced water tile. Dismounting is vanilla:
// walk into the shore.
static BOOL Coop_Task_DebugSurf(TaskManager *taskman)
{
    FieldSystem *fsys = taskman->fieldSystem;
    FIELD_PLAYER_AVATAR *avatar = fsys->playerAvatar;
    LocalMapObject *player = avatar->mapObject;

    switch (taskman->state)
    {
    case 0:
        if (MapObject_IsSingleMovementActive(player))
            break; // finish a step in progress first
        Field_PlayerAvatar_OrrTransitionFlags(avatar, COOP_PLAYER_TRANSITION_SURFING);
        Field_PlayerAvatar_ApplyTransitionFlags(avatar);
        taskman->state++;
        break;
    case 1:
        if (!MapObject_AreBitsSetForMovementScriptInit(player))
            break;
        MapObject_SetHeldMovement(player, COOP_MOVEMENT_JUMP_NEAR_FAST + (u32)player->curFacing);
        taskman->state++;
        break;
    case 2:
        if (!MapObject_ClearHeldMovementIfActive(player))
            break;
        sys_FreeMemoryEz(taskman->env);
        return TRUE;
    }
    return FALSE;
}

// Field-state commands run only between events: no field task (script,
// menu exit, warp) and no message box, with the player standing still.
// Returns FALSE while they must wait.
static BOOL Coop_CanRunFieldDebugCommand(FieldSystem *fsys, LocalMapObject *player)
{
    return fsys->taskman == NULL && !fsys->textbox_open && !MapObject_IsSingleMovementActive(player);
}

static u8 Coop_DebugWarp(FieldSystem *fsys, u8 preset)
{
    const struct CoopWarpTarget *target;

    if (preset >= COOP_WARP_COUNT)
        return COOP_DEBUG_RESULT_BAD_ARG;
    target = &sCoopWarpTargets[preset];
    // Fades out, unloads the field (and this overlay), loads the map and sets
    // the player walking; the task runs from the next frame on.
    sub_020538C0(fsys, target->mapId, -1, target->x, target->z, COOP_DIR_SOUTH);
    return COOP_DEBUG_RESULT_OK;
}

// What the Bicycle item does (Task_MountOrDismountBicycle), minus the bike
// music and follower reactions.
static u8 Coop_DebugToggleBike(FieldSystem *fsys)
{
    FIELD_PLAYER_AVATAR *avatar = fsys->playerAvatar;

    if (avatar->state == COOP_PLAYER_STATE_CYCLING)
        Field_PlayerAvatar_OrrTransitionFlags(avatar, COOP_PLAYER_TRANSITION_WALKING);
    else if (avatar->state == COOP_PLAYER_STATE_WALKING)
        Field_PlayerAvatar_OrrTransitionFlags(avatar, COOP_PLAYER_TRANSITION_CYCLING);
    else
        return COOP_DEBUG_RESULT_NOT_NOW;
    Field_PlayerAvatar_ApplyTransitionFlags(avatar);
    return COOP_DEBUG_RESULT_OK;
}

static u8 Coop_DebugSurf(FieldSystem *fsys, LocalMapObject *player)
{
    if (fsys->playerAvatar->state != COOP_PLAYER_STATE_WALKING)
        return COOP_DEBUG_RESULT_NOT_NOW;
    if (!Coop_IsFacingSurfableWater(fsys, player))
        return COOP_DEBUG_RESULT_NO_WATER;
    if (!Coop_StartFieldTask(fsys, Coop_Task_DebugSurf, sizeof(u32)))
        return COOP_DEBUG_RESULT_FAILED;
    return COOP_DEBUG_RESULT_OK;
}

// The registered Old Rod (ItemFieldUseFunc_OldRod). A bite is a real
// encounter: reeling it in starts a wild battle, which needs a party.
static u8 Coop_DebugFish(FieldSystem *fsys, LocalMapObject *player)
{
    void *env;
    u32 state = (u32)fsys->playerAvatar->state;

    if (state != COOP_PLAYER_STATE_WALKING && state != COOP_PLAYER_STATE_SURFING)
        return COOP_DEBUG_RESULT_NOT_NOW;
    if (!Coop_IsFacingSurfableWater(fsys, player))
        return COOP_DEBUG_RESULT_NO_WATER;
    env = CreateFishingRodTaskEnv(fsys, COOP_HEAP_ID_FIELD1, COOP_ROD_TYPE_OLD);
    if (env == NULL)
        return COOP_DEBUG_RESULT_FAILED;
    if (Coop_FieldSystem_CreateTask(fsys, Task_OverworldFish, env) == NULL)
    {
        sys_FreeMemoryEz(env);
        return COOP_DEBUG_RESULT_FAILED;
    }
    return COOP_DEBUG_RESULT_OK;
}

static u8 Coop_DebugTestGhost(MapObjectMan *manager, LocalMapObject *player, u8 cmd, BOOL *wait)
{
    LocalMapObject *ghost = Coop_FindObjectById(manager, COOP_TEST_GHOST_ID);

    if (cmd == COOP_DEBUG_SPAWN_TEST_GHOST)
    {
        if (ghost != NULL)
            return COOP_DEBUG_RESULT_OK;
        return Coop_SpawnTestGhost(manager, player) ? COOP_DEBUG_RESULT_OK : COOP_DEBUG_RESULT_FAILED;
    }
    if (ghost == NULL)
        return COOP_DEBUG_RESULT_NO_TARGET;
    if (!Coop_GhostIsIdle(ghost))
    {
        *wait = TRUE; // busy: try again next frame
        return COOP_DEBUG_RESULT_NONE;
    }
    if (cmd == COOP_DEBUG_STEP_TEST_GHOST)
        // One tile in the player's facing direction; the step action also
        // turns the ghost. Held movements ignore collision.
        MapObject_SetHeldMovement(ghost, COOP_MOVEMENT_STEP_UP + (u32)player->curFacing);
    else if (cmd == COOP_DEBUG_EMOTE_TEST_GHOST)
        MapObject_SetHeldMovement(ghost, COOP_MOVEMENT_EMOTE_EXCLAIM);
    else
        DeleteMapObject(ghost);
    return COOP_DEBUG_RESULT_OK;
}
#endif // COOP_DEBUG_TOOLS

// A command that cannot run yet (busy ghost, event in progress, player
// mid-step) is retried every frame; a newer command from the bridge
// replaces a waiting one.
static void Coop_ProcessDebugCommand(FieldSystem *fsys, MapObjectMan *manager, LocalMapObject *player)
{
    volatile struct CoopCommBlock *mb = &gCoopCommBlock;
    u8 seq = mb->debug_seq;
    u8 cmd = mb->debug_cmd;
    u8 result;

    if (seq == mb->diag.debug_ack)
        return;

#if COOP_DEBUG_TOOLS
    {
        BOOL wait = FALSE;

        switch (cmd)
        {
        case COOP_DEBUG_SPAWN_TEST_GHOST:
        case COOP_DEBUG_STEP_TEST_GHOST:
        case COOP_DEBUG_EMOTE_TEST_GHOST:
        case COOP_DEBUG_REMOVE_TEST_GHOST:
            result = Coop_DebugTestGhost(manager, player, cmd, &wait);
            break;
        case COOP_DEBUG_WARP:
        case COOP_DEBUG_TOGGLE_BIKE:
        case COOP_DEBUG_SURF:
        case COOP_DEBUG_FISH:
            if (!Coop_CanRunFieldDebugCommand(fsys, player))
                return;
            if (cmd == COOP_DEBUG_WARP)
                result = Coop_DebugWarp(fsys, mb->debug_arg);
            else if (cmd == COOP_DEBUG_TOGGLE_BIKE)
                result = Coop_DebugToggleBike(fsys);
            else if (cmd == COOP_DEBUG_SURF)
                result = Coop_DebugSurf(fsys, player);
            else
                result = Coop_DebugFish(fsys, player);
            break;
        default:
            result = COOP_DEBUG_RESULT_BAD_ARG;
            break;
        }
        if (wait)
            return;
    }
#else
    (void)fsys;
    (void)manager;
    (void)player;
    (void)cmd;
    result = COOP_DEBUG_RESULT_DISABLED;
#endif
    mb->debug_result = result;
    mb->diag.debug_ack = seq;
}

static void Coop_ReportTestGhost(MapObjectMan *manager, LocalMapObject *player)
{
    volatile struct CoopDiagnostics *diag = &gCoopCommBlock.diag;
    LocalMapObject *ghost = Coop_FindObjectById(manager, COOP_TEST_GHOST_ID);

    diag->player_y = Coop_RenderHeight(player);
    if (ghost == NULL)
    {
        diag->test_ghost_state = COOP_TEST_GHOST_NONE;
        return;
    }
    diag->test_ghost_state = (ghost->flags & COOP_MOFLAG_SINGLE_MOVEMENT) ? COOP_TEST_GHOST_MOVING : COOP_TEST_GHOST_IDLE;
    diag->test_ghost_x = (s16)ghost->xCurr;
    diag->test_ghost_z = (s16)ghost->yCurr;
    diag->test_ghost_y = Coop_RenderHeight(ghost);
}

// ---------------------------------------------------------------------------
// Local player telemetry (mailbox local_player; docs/MILESTONE2_PLAN.md 5.2)
// ---------------------------------------------------------------------------

struct Party *LONG_CALL SaveArray_Party_Get(void *saveData);

#define COOP_MATRIX_ID_OFFSET 2 // pret MapMatrix: width, height, matrix_id

// A new held movement started since the last frame: the action changed, the
// object moved to another tile (steps update the tile when they begin), or
// the action's step counter went back (MapObject_SetHeldMovement resets it).
// A repeat of the same in-place action (walking into a wall) can go unseen;
// that only costs a cosmetic bump on other screens.
static u8 Coop_NextMoveSeq(const LocalMapObject *object, BOOL lastValid, s16 lastX, s16 lastZ, u8 lastAction,
                           u8 lastSeq, volatile u8 *lastStep)
{
    u8 step = (u8)object->movementStep;
    BOOL started = lastValid
        && ((u8)object->movementCmd != lastAction
            || (s16)object->xCurr != lastX || (s16)object->yCurr != lastZ
            || step < *lastStep);

    *lastStep = step;
    return started ? (u8)(lastSeq + 1) : lastSeq;
}

static s16 Coop_ObjectHeight(const LocalMapObject *object)
{
    return (s16)((s32)object->posVec[1] >> 12); // fx32 -> world units
}

static void Coop_BuildFollower(FieldSystem *fsys, struct CoopPlayerState *ps, const struct CoopPlayerState *last, BOOL lastValid)
{
    const FollowMon *follow = &fsys->followMon;
    const LocalMapObject *object = follow->mapObject;

    if (!follow->active || object == NULL || !(object->flags & COOP_MOFLAG_ACTIVE))
        return;
    ps->follower_species = (u16)follow->species;
    ps->follower_sprite_id = (u16)object->gfxId;
    ps->follower_form = (u8)follow->forme;
    ps->follower_shiny = follow->shiny ? 1 : 0;
    ps->follower_gender = follow->gender;
    ps->follower_visible = 1;
    ps->follower_x = (s16)object->xCurr;
    ps->follower_z = (s16)object->yCurr;
    ps->follower_height = Coop_ObjectHeight(object);
    ps->follower_direction = (u8)object->curFacing;
    ps->follower_move_action = (u8)object->movementCmd;
    ps->follower_move_seq = Coop_NextMoveSeq(object, lastValid && last->follower_visible,
                                             last->follower_x, last->follower_z, last->follower_move_action,
                                             last->follower_move_seq, &gCoopCommBlock.track_follower_step);
}

static void Coop_BuildLocalPlayerState(FieldSystem *fsys, LocalMapObject *player, struct CoopPlayerState *ps, u16 *badgeMask)
{
    struct CoopPlayerState last;
    struct PlayerProfile *profile;
    struct Party *party;
    u32 partyCount = 0;
    BOOL lastValid;
    u32 i;

    // The previous snapshot drives move_seq; it only counts while we stayed
    // in the field (a map load or battle starts the tracking over).
    __builtin_memcpy(&last, (const void *)&gCoopCommBlock.local_player, sizeof(last));
    lastValid = last.is_active == 1 && last.map_id == (u16)fsys->location->mapId;

    ps->map_id = (u16)fsys->location->mapId;
    if (fsys->map_matrix != NULL)
        ps->matrix_id = ((const u8 *)fsys->map_matrix)[COOP_MATRIX_ID_OFFSET];
    ps->avatar_state = (u8)fsys->playerAvatar->state;
    ps->x = (s16)player->xCurr;
    ps->z = (s16)player->yCurr;
    ps->height = Coop_ObjectHeight(player);
    ps->direction = (u8)player->curFacing;
    ps->gender = (u8)fsys->playerAvatar->gender;
    ps->sprite_id = (u16)player->gfxId;
    ps->move_action = (u8)player->movementCmd;
    ps->move_seq = Coop_NextMoveSeq(player, lastValid, last.x, last.z, last.move_action, last.move_seq,
                                    &gCoopCommBlock.track_player_step);
    ps->is_active = 1;
    ps->activity = COOP_ACTIVITY_FIELD;

    profile = Sav2_PlayerData_GetProfileAddr(fsys->savedata);
    if (profile != NULL)
    {
        ps->trainer_id = profile->id;
        for (i = 0; i < 8; i++)
            ps->name[i] = profile->name[i];
        for (i = 0; i < 16; i++)
        {
            if (PlayerProfile_TestBadgeFlag(profile, (s32)i) == TRUE)
                *badgeMask |= (u16)(1u << i);
        }
    }

    party = SaveArray_Party_Get(fsys->savedata);
    if (party != NULL)
        partyCount = (u32)party->count;

    // Lobby readiness and save admission live here, next to the game state
    // they depend on.
    ps->client_ready = (ps->map_id == COOP_BEDROOM_MAP_ID && ps->trainer_id != 0 && partyCount == 0) ? 1 : 0;
    ps->save_status = (ps->map_id == COOP_BEDROOM_MAP_ID && partyCount == 0) ? COOP_SAVE_FRESH : COOP_SAVE_IN_PROGRESS;

    Coop_BuildFollower(fsys, ps, &last, lastValid);
}

// Publishes a complete snapshot under a sequence lock: local_player_seq is
// odd while the copy is in progress, so the bridge can reject a torn read
// (possible on lag frames, when a frame boundary lands mid-copy).
static void Coop_PublishLocalPlayerState(FieldSystem *fsys, LocalMapObject *player)
{
    volatile u32 *dst = (volatile u32 *)&gCoopCommBlock.local_player;
    struct CoopPlayerState ps;
    u32 words[sizeof(ps) / sizeof(u32)];
    u16 badgeMask = 0;
    u32 i;

    __builtin_memset(&ps, 0, sizeof(ps));
    Coop_BuildLocalPlayerState(fsys, player, &ps, &badgeMask);

    // memcpy, not a (const u32 *) cast of ps: reading the struct through an
    // incompatible pointer type is undefined behaviour (strict aliasing).
    __builtin_memcpy(words, &ps, sizeof(words));

    gCoopCommBlock.local_player_seq++;
    for (i = 0; i < sizeof(words) / sizeof(u32); i++)
        dst[i] = words[i];
    gCoopCommBlock.badge_mask_local = badgeMask;
    gCoopCommBlock.local_player_seq++;
}

// ---------------------------------------------------------------------------
// Entry (probe 2): called by Coop_MainSysTask (Overlay 129) once per frame
// while the field map is active and this overlay is loaded.
// ---------------------------------------------------------------------------

static void Coop_FieldEntry(FieldSystem *fsys)
{
    MapObjectMan *manager = fsys->mapObjectMan;
    LocalMapObject *player;

    gCoopCommBlock.diag.field_entry_calls++;
    if (manager == NULL || fsys->playerAvatar == NULL || fsys->playerAvatar->mapObject == NULL)
        return;
    player = fsys->playerAvatar->mapObject;

    if (fsys->location == NULL || fsys->savedata == NULL)
        return;
    Coop_PublishLocalPlayerState(fsys, player);
    Coop_ProbeRender(manager);
    Coop_ProcessDebugCommand(fsys, manager, player);
    Coop_ReportTestGhost(manager, player);
}
