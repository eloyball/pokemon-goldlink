#include "config.h"
#include "types.h"

#include "constants/buttons.h"
#include "constants/maps.h"
#include "constants/species.h"

#include "battle.h"
#include "item.h"

#include "map_events_internal.h"
#include "pokemon.h"
#include "save.h"
#include "system.h"
#include "task.h"
#include "coop_sync.h"
#include "coop_session.h"

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
// frame (Coop_PublishLocalPlayerState below) and runs the remote players'
// ghosts (Coop_UpdateGhosts, Coop_CheckGhostTalk; Phases 2 and 3).
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
#define COOP_MOFLAG_HIDDEN          (1u << 9)  // pret MAPOBJECTFLAG_VISIBLE: set = not drawn (follower in its ball)
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
struct Party *LONG_CALL SaveArray_Party_Get(void *saveData);
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
_Static_assert(__builtin_offsetof(FieldSystem, unkD3) == 0xD3, "FieldSystem.textbox_open byte must be 0xD2 (pret field_system.h)");
_Static_assert(__builtin_offsetof(FieldSystem, followMon) == 0xE4, "FieldSystem.followMon must be at 0xE4 (pret field_system.h)");
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

#define COOP_OBJECT_HEADROOM 4 // map object slots always left for vanilla

// A ghost: a plain map object (movement 0, no script) that survives zone
// changes inside the matrix, never collides and never runs a vanilla script.
// MapObject_Create snaps it to the ground, so the height is written after:
// a ghost created on a bridge stays on the bridge. NULL when the object
// list is (nearly) full.
static LocalMapObject *Coop_CreateGhostObject(MapObjectMan *manager, u32 id, s32 x, s32 z, u32 direction,
                                              u32 sprite, s16 height)
{
    LocalMapObject *ghost;

    if (manager->active_count + COOP_OBJECT_HEADROOM >= manager->object_count)
        return NULL;
    ghost = CreateSpecialFieldObject(manager, (u32)x, (u32)z, direction, sprite, 0, MAP_NOTHING);
    if (ghost == NULL)
        return NULL;
    ghost->id = (int)id;
    MapObject_SetBits(ghost, COOP_MOFLAG_KEEP | COOP_MOFLAG_UNK13 | COOP_MOFLAG_NO_COLLISION | COOP_MOFLAG_NO_INTERACT);
    ghost->hPrev = height >> 3; // tile height = render height / 8
    ghost->hCurr = height >> 3;
    ghost->posVec[1] = (u32)((s32)height << 12);
    return ghost;
}

#if COOP_DEBUG_TOOLS
// Spawning and driving the test ghost is a debug tool; reporting it (below)
// stays so the diagnostics card works in every build.

static BOOL Coop_SpawnTestGhost(MapObjectMan *manager, LocalMapObject *player)
{
    s32 dx, dz;
    u32 direction = (u32)player->curFacing;

    Coop_DirectionDelta(direction, &dx, &dz);
    // Same sprite as the local player: no extra texture / model / sequence.
    return Coop_CreateGhostObject(manager, COOP_TEST_GHOST_ID, player->xCurr + dx, player->yCurr + dz, direction,
                                  (u32)player->gfxId, Coop_RenderHeight(player)) != NULL;
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
    [COOP_WARP_VIOLET_GYM]  = { MAP_T22GYM0101, 15, 6 }, // MAP_VIOLET_GYM (Falkner on 15, 4)
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
    return fsys->taskman == NULL && !fsys->textbox_open && !MapObject_IsSingleMovementActive(player)
        && !Coop_IsMapNameBannerShowing(fsys);
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

// The Bicycle item itself (ItemFieldUseFunc_Bicycle starts vanilla
// Task_MountOrDismountBicycle: sprite, bike music, follower recalled into its
// ball and released again). That task first waits for the follower's
// movement to be paused and unpauses everything when done, as the field
// pauses movement before a registered item's task runs.
static u8 Coop_DebugToggleBike(FieldSystem *fsys)
{
    struct ItemFieldUseData data;
    u32 state = (u32)fsys->playerAvatar->state;

    if (state != COOP_PLAYER_STATE_CYCLING && state != COOP_PLAYER_STATE_WALKING)
        return COOP_DEBUG_RESULT_NOT_NOW;
    __builtin_memset(&data, 0, sizeof(data));
    data.fieldSystem = fsys; // all it reads
    MapObjectMan_PauseAllMovement(fsys->mapObjectMan);
    ItemFieldUseFunc_Bicycle(&data);
    return COOP_DEBUG_RESULT_OK;
}

// pret label (asm/overlay_01_021F1AFC.s): pushes vanilla Task_Surf (cut-in or
// follower hop, cry, follower recalled, mount, jump) onto the running task.
void LONG_CALL CallFieldTask_Surf(TaskManager *taskman, int direction, int partySlot);

// Runs vanilla Surf with the lead Pokemon from a field task of its own.
static BOOL Coop_Task_DebugVanillaSurf(TaskManager *taskman)
{
    FieldSystem *fsys = taskman->fieldSystem;

    switch (taskman->state)
    {
    case 0:
        if (MapObject_IsSingleMovementActive(fsys->playerAvatar->mapObject))
            break; // finish a step in progress first
        taskman->state++;
        CallFieldTask_Surf(taskman, fsys->playerAvatar->mapObject->curFacing, 0);
        break;
    case 1: // Task_Surf finished and returned here
        sys_FreeMemoryEz(taskman->env);
        return TRUE;
    }
    return FALSE;
}

static u8 Coop_DebugSurf(FieldSystem *fsys, LocalMapObject *player)
{
    struct Party *party = SaveArray_Party_Get(fsys->savedata);

    if (fsys->playerAvatar->state != COOP_PLAYER_STATE_WALKING)
        return COOP_DEBUG_RESULT_NOT_NOW;
    if (!Coop_IsFacingSurfableWater(fsys, player))
        return COOP_DEBUG_RESULT_NO_WATER;
    // With a party: vanilla Surf, follower included. Without one (no
    // follower either): the mount alone.
    if (!Coop_StartFieldTask(fsys, (party != NULL && party->count > 0) ? Coop_Task_DebugVanillaSurf : Coop_Task_DebugSurf,
                             sizeof(u32)))
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

// Milestone 3 probe A: Pokemon transfer. The encrypted struct is copied
// byte for byte (its encryption is keyed by its own PID and checksum); the
// receiving side reads it decrypted only to validate it.
// hg-engine's struct PartyPokemon has the right field layout but a u64
// ribbon field makes GCC align it to 8, so sizeof is 0xF0, not the game's
// 0xEC: copy COOP_PARTY_MON_SIZE bytes, never sizeof, and never index
// party->members[] (stride 0xF0); use Party_GetMonByIndex.
_Static_assert(__builtin_offsetof(struct PartyPokemon, party) + sizeof(PARTYONLYMON) == COOP_PARTY_MON_SIZE,
               "party Pokemon data must end at 0xEC");

static void Coop_DebugDescribeMon(struct PartyPokemon *mon, struct Party *party)
{
    volatile struct CoopDebugMonBuffer *buf = &gCoopDebugMonBuffer;

    buf->checksum_failed = (u8)GetMonData(mon, MON_DATA_CHECKSUM_FAILED, NULL);
    buf->species = (u16)GetMonData(mon, MON_DATA_SPECIES, NULL);
    buf->held_item = (u16)GetMonData(mon, MON_DATA_HELD_ITEM, NULL);
    buf->level = (u8)GetMonData(mon, MON_DATA_LEVEL, NULL);
    buf->is_egg = (u8)GetMonData(mon, MON_DATA_IS_EGG, NULL);
    buf->party_count = (u8)party->count;
}

static u8 Coop_DebugMonTransfer(FieldSystem *fsys, u8 cmd, u8 slot)
{
    struct Party *party = SaveArray_Party_Get(fsys->savedata);
    struct PartyPokemon mon;

    if (party == NULL)
        return COOP_DEBUG_RESULT_FAILED;
    if (cmd == COOP_DEBUG_MON_EXPORT)
    {
        if (slot >= party->count)
            return COOP_DEBUG_RESULT_BAD_ARG;
        __builtin_memcpy((void *)gCoopDebugMonBuffer.mon, Party_GetMonByIndex(party, slot), COOP_PARTY_MON_SIZE);
        Coop_DebugDescribeMon(Party_GetMonByIndex(party, slot), party);
        return COOP_DEBUG_RESULT_OK;
    }
    // Import: validate a private copy before it can touch the party.
    __builtin_memset(&mon, 0, sizeof(mon));
    __builtin_memcpy(&mon, (const void *)gCoopDebugMonBuffer.mon, COOP_PARTY_MON_SIZE);
    Coop_DebugDescribeMon(&mon, party);
    if (gCoopDebugMonBuffer.checksum_failed || gCoopDebugMonBuffer.is_egg
        || gCoopDebugMonBuffer.species == 0 || gCoopDebugMonBuffer.species > MAX_SPECIES_INCLUDING_FORMS
        || gCoopDebugMonBuffer.level == 0 || gCoopDebugMonBuffer.level > 100)
        return COOP_DEBUG_RESULT_REJECTED;
    if (party->count >= 6 || !PokeParty_Add(party, &mon))
        return COOP_DEBUG_RESULT_FAILED;
    gCoopDebugMonBuffer.party_count = (u8)party->count;
    return COOP_DEBUG_RESULT_OK;
}

// Milestone 3 probe E: once armed, the next trainer battle, right after its
// opponent party is built, becomes a multi battle with an AI partner: the
// player and a partner (the player's own party standing in for a teammate)
// against the same trainer in both opponent seats. Shows whether a lone
// leader can fill a 2v2 and how vanilla presents it.
void LONG_CALL PokeParty_Init(struct Party *party, int max);

void Coop_DebugTeamBattle(struct BATTLE_PARAM *bp, int num)
{
    struct Party *source;
    int i;

    if (!gCoopDebugMonBuffer.team_battle_armed || num != 1)
        return;
    gCoopDebugMonBuffer.team_battle_armed = 0;
    bp->fight_type = BATTLE_TYPE_TRAINER | BATTLE_TYPE_DOUBLES | BATTLE_TYPE_MULTI | BATTLE_TYPE_AI;

    bp->trainer_id[3] = bp->trainer_id[1];
    bp->trainer_data[3] = bp->trainer_data[1];
    source = bp->poke_party[1];
    PokeParty_Init(bp->poke_party[3], 6);
    for (i = 0; i < source->count; i++)
        PokeParty_Add(bp->poke_party[3], Party_GetMonByIndex(source, i));

    // Vanilla EnemyTrainerSet_Init re-reads fight_type and then builds the
    // other seats' parties from trainer_id too (seen in the probe: the
    // partner seat ended up with the leader's team). A real version must
    // fill seats 2 / 3 after it, or hook later.
    bp->trainer_id[2] = bp->trainer_id[1];
    bp->trainer_data[2] = bp->trainer_data[1];
    source = bp->poke_party[0];
    PokeParty_Init(bp->poke_party[2], 6);
    for (i = 0; i < source->count && i < 3; i++)
        PokeParty_Add(bp->poke_party[2], Party_GetMonByIndex(source, i));
    gCoopDebugMonBuffer.team_battle_used = 1;
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

// Vanilla link sessions over the virtual link (docs/MILESTONE3_PLAN.md §15):
// what the Colosseum does, without its rooms. pret comm_state.c (asm labels
// in pret f90c3a7c0) and link_synchronization.c; docs/LINK_SYMBOLS.md.
#define COOP_COMM_TYPE_SINGLE_BATTLE 1 // ccd constants/comm_type.h

void LONG_CALL CommState_StartParentSession(void *saveData, s8 linkMode, s8 beaconId, void *rules, u32 unused);
void LONG_CALL CommState_StartChildSession(void *saveData, s8 linkMode, s8 beaconId, void *rules, u32 unused);
void LONG_CALL CommState_ConnectToSlot(s8 slot);
void LONG_CALL CommState_RequestShutdown(void);
BOOL LONG_CALL CommState_Exists(void);
BOOL LONG_CALL CommState_IsLinked(void);
void LONG_CALL LinkSync_StartSync(u8 state);
BOOL LONG_CALL LinkSync_IsSynced(u8 state);

static u8 Coop_DebugLink(FieldSystem *fsys, u32 op)
{
    switch (op)
    {
    case COOP_DEBUG_LINK_PARENT:
        CommState_StartParentSession(fsys->savedata, COOP_COMM_TYPE_SINGLE_BATTLE, 0, NULL, 0);
        break;
    case COOP_DEBUG_LINK_CHILD:
        CommState_StartChildSession(fsys->savedata, COOP_COMM_TYPE_SINGLE_BATTLE, 0, NULL, 0);
        break;
    case COOP_DEBUG_LINK_CONNECT:
    case COOP_DEBUG_LINK_SYNC:
    case COOP_DEBUG_LINK_SHUTDOWN:
        if (!CommState_Exists())
            return COOP_DEBUG_RESULT_NOT_NOW;
        if (op == COOP_DEBUG_LINK_CONNECT)
            CommState_ConnectToSlot(0);
        else if (op == COOP_DEBUG_LINK_SYNC)
            LinkSync_StartSync(COOP_DEBUG_LINK_SYNC_VALUE);
        else
            CommState_RequestShutdown();
        break;
    default:
        return COOP_DEBUG_RESULT_BAD_ARG;
    }
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
        case COOP_DEBUG_LINK:
            result = Coop_DebugLink(fsys, mb->debug_arg);
            break;
        case COOP_DEBUG_LINK_STATUS:
            result = COOP_DEBUG_LINK_STATUS_BASE;
            if (CommState_Exists())
            {
                result |= COOP_DEBUG_LINK_EXISTS;
                if (CommState_IsLinked())
                    result |= COOP_DEBUG_LINK_LINKED;
                if (LinkSync_IsSynced(mb->debug_arg))
                    result |= COOP_DEBUG_LINK_SYNCED;
            }
            break;
        case COOP_DEBUG_ARM_TEAM_BATTLE:
            gCoopDebugMonBuffer.team_battle_armed = 1;
            gCoopDebugMonBuffer.team_battle_used = 0;
            result = COOP_DEBUG_RESULT_OK;
            break;
        case COOP_DEBUG_MON_EXPORT:
        case COOP_DEBUG_MON_IMPORT:
            if (!Coop_CanRunFieldDebugCommand(fsys, player))
                return;
            result = Coop_DebugMonTransfer(fsys, cmd, mb->debug_arg);
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
    // Vanilla hides the follower (in its ball) while cycling, surfing and in
    // some maps, without deleting it.
    ps->follower_visible = (object->flags & COOP_MOFLAG_HIDDEN) ? 0 : 1;
    ps->follower_x = (s16)object->xCurr;
    ps->follower_z = (s16)object->yCurr;
    ps->follower_height = Coop_ObjectHeight(object);
    ps->follower_direction = (u8)object->curFacing;
    ps->follower_move_action = (u8)object->movementCmd;
    ps->follower_move_seq = Coop_NextMoveSeq(object, lastValid && last->follower_species != 0,
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
    ps->anim = (u8)player->unkA0; // pret LocalMapObject.animation (MapObject_SetAnimation)
    ps->move_seq = Coop_NextMoveSeq(player, lastValid, last.x, last.z, last.move_action, last.move_seq,
                                    &gCoopCommBlock.track_player_step);
    ps->is_active = 1;
    ps->activity = COOP_ACTIVITY_FIELD;
    // Others see "busy" instead of the session menu when they talk to us.
    ps->field_busy = (fsys->taskman != NULL || fsys->textbox_open) ? 1 : 0;

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
// Player ghosts: radii, bookkeeping, placement (docs/MILESTONE2_PLAN.md 5.4)
// ---------------------------------------------------------------------------

#define COOP_SPRITE_HERO    0  // pret SPRITE_HERO
#define COOP_SPRITE_HEROINE 97 // pret SPRITE_HEROINE

// Radii in tiles around the local player. The DS shows about 16 x 12 tiles,
// so "on screen" is within 8 x 6 (plus a tile of margin). A player walking
// toward us gets their ghost just off screen, and it despawns beyond a
// larger radius; a player who arrives in view (through a door, or by
// connecting) appears like any NPC. An existing ghost never jumps on screen.
#define COOP_SPAWN_RADIUS_X   10
#define COOP_SPAWN_RADIUS_Z   8
#define COOP_DESPAWN_RADIUS_X 13
#define COOP_DESPAWN_RADIUS_Z 10
#define COOP_SCREEN_RADIUS_X  9
#define COOP_SCREEN_RADIUS_Z  7
#define COOP_CATCHUP_MAX      4 // farther behind than this: snap while the ghost is off screen

// pret constants/movements.h groups (+ direction: north, south, west, east).
#define COOP_MOVEMENT_FACE      0

// Per-ghost bookkeeping in the ghost object's own params (movement 0 never
// reads them): survives frames without statics (Overlay 131 has no BSS
// init) and disappears with the object on every map reload.
#define COOP_GHOST_SEQ(ghost)     ((ghost)->param[0]) // last move_seq handled, | 0x100 once set
#define COOP_GHOST_TRAINER(ghost) ((ghost)->param[1]) // trainer the ghost belongs to
#define COOP_GHOST_MOUNT(ghost)   ((ghost)->param[2]) // its surf mount field effect, 0 = none
// Its "!" / "!!" mark: effect pointer | kind (bit 0; effects are 4-aligned),
// 0 = none. initialFacing is only read when an object is set up (pret
// map_object.c sub_0205EC90), before this is written.
#define COOP_GHOST_MARK(ghost)    ((ghost)->facingInit)
#define COOP_SEQ_VALID 0x100

static s32 Coop_Abs(s32 v)
{
    return v < 0 ? -v : v;
}

// Tiles a held movement moves its object, or -1 for actions never replayed
// as movement (delays, warps, emotes, scripted poses, odd-direction jumps).
static s32 Coop_ActionDistance(u32 action)
{
    if (action <= 3)
        return 0; // face
    if (action <= 23)
        return 1; // walk, slower .. faster
    if (action <= 51)
        return 0; // walk / jump on the spot
    if (action <= 55)
        return 1; // jump near (onto water, etc.)
    if (action <= 59)
        return 2; // jump far (ledges)
    if (action >= 76 && action <= 91)
        return 1; // walk / run variants
    if (action >= 96 && action <= 99)
        return 1;
    return -1;
}

static u32 Coop_LocalMatrixId(FieldSystem *fsys)
{
    return fsys->map_matrix != NULL ? ((const u8 *)fsys->map_matrix)[COOP_MATRIX_ID_OFFSET] : 0;
}

// Same place as the local player: the same matrix, and outside the outdoor
// matrix (0) also the same map, because interiors share matrices (every
// Pokemon Center uses one) and their tile coordinates are per map.
static BOOL Coop_IsSamePlace(FieldSystem *fsys, const struct CoopPlayerState *remote)
{
    u32 matrix = Coop_LocalMatrixId(fsys);

    if (remote->matrix_id != matrix)
        return FALSE;
    return matrix == 0 || remote->map_id == (u16)fsys->location->mapId;
}

static BOOL Coop_IsWithin(s32 x, s32 z, const LocalMapObject *player, s32 radiusX, s32 radiusZ)
{
    return Coop_Abs(x - player->xCurr) <= radiusX && Coop_Abs(z - player->yCurr) <= radiusZ;
}

// ---------------------------------------------------------------------------
// Ghost sprites and the renderer budget (plan 3.2, 5.5; Phase 3)
// ---------------------------------------------------------------------------

// Overlay 01 (rom.ld "Player ghost states"; ccd names).
void *LONG_CALL SurfMountEffect_Create(LocalMapObject *object, u32 x, u32 z, u32 direction, BOOL bobbing);
void LONG_CALL FieldEffectManager_DestroyEffect(void *effect);
void LONG_CALL MapObjectRender_ReleaseSpriteResources(MapObjectMan *manager, u32 sprite, LocalMapObject *skip);
void LONG_CALL MapObjectRender_ReleaseUnused(MapObjectMan *manager, void *render);
// Overlay 01 lookup of an overworld tag (rom.ld; hg-engine repoints its table
// to gOWTagToFileNum). NULL for a tag that does not exist.
struct OVERWORLD_TAG *LONG_CALL ObjectEvent_GetGraphicsInfo(u32 spriteId);

// Every player sprite (PlayerAvatar_GetSpriteByStateAndGender, plus the
// map-load variants) with the billboard model and animation sequence it
// needs (ccd gObjectEventGraphicsInfos -> gSpriteBillboardInfos). The texture
// is per sprite. A remote sprite outside this list shows as walking.
struct CoopPlayerSprite
{
    u16 sprite;
    u8 model;
    u8 sequence : 7;
    u8 posed : 1; // animated by MapObject_SetAnimation (tasks), not by movements
};

static const struct CoopPlayerSprite sCoopPlayerSprites[] = {
    { 0, 0, 0x01, 0 },   { 97, 0, 0x01, 0 },  // HERO, HEROINE (walking, running)
    { 21, 0, 0x03, 0 },  { 98, 0, 0x03, 0 },  // CYCLEHERO(INE)
    { 176, 0, 0x04, 1 }, { 177, 0, 0x04, 1 }, // SPHERO(INE): HM pose
    { 178, 0, 0x06, 0 }, { 179, 0, 0x06, 0 }, // SWIMHERO(INE)
    { 180, 0, 0x05, 1 }, { 181, 0, 0x05, 1 }, // WATERHERO(INE): SquirtBottle
    { 188, 6, 0x0A, 1 }, { 189, 6, 0x0A, 1 }, // FISHINGHERO, FISH_HEROINE (idle / cast / bite / reel)
    { 196, 0, 0x0D, 1 }, { 197, 0, 0x0D, 1 }, // POKEHERO(INE): Pokegear
    { 198, 0, 0x0E, 1 }, { 199, 0, 0x0E, 1 }, // SAVEHERO(INE)
    { 200, 0, 0x0F, 1 }, { 201, 0, 0x0F, 1 }, // BANZAIHERO(INE): healing
    { 238, 0, 0x01, 0 }, { 239, 0, 0x01, 0 }, // HERO_2, HEROINE_2
    { 248, 11, 0x17, 1 }, { 249, 11, 0x17, 1 }, // LADDERHERO(INE)
    { 258, 0, 0x01, 0 }, { 259, 0, 0x01, 0 }, // RHERO(INE): Rocket disguise
    { 260, 0, 0x0F, 1 }, { 261, 0, 0x0F, 1 }, // RBANZAIHERO, RBANZAIINE
    { 297, 0, 0x0E, 1 }, { 298, 0, 0x0E, 1 }, // SAVERHERO(INE)
    { 407, 0, 0x01, 0 }, { 408, 0, 0x01, 0 }, // PKTHHERO(INE): Pokeathlon
    { 423, 0, 0x1A, 1 }, { 424, 0, 0x1A, 1 }, // SHAKEHERO(INE): Apricorn shake
};

static const struct CoopPlayerSprite *Coop_FindPlayerSprite(u32 sprite)
{
    u32 i;

    for (i = 0; i < NELEMS(sCoopPlayerSprites); i++)
    {
        if (sCoopPlayerSprites[i].sprite == sprite)
            return &sCoopPlayerSprites[i];
    }
    return NULL;
}

static u32 Coop_WalkingSprite(const struct CoopPlayerState *remote)
{
    return remote->gender ? COOP_SPRITE_HEROINE : COOP_SPRITE_HERO;
}

// The remote's own sprite when it is a player sprite, else walking.
static u32 Coop_WantedSprite(const struct CoopPlayerState *remote)
{
    return Coop_FindPlayerSprite(remote->sprite_id) != NULL ? remote->sprite_id : Coop_WalkingSprite(remote);
}

// Free slots always left for vanilla. One animation (and model) is the room
// the local player's own next pose needs; textures are plentiful. A map often
// holds a second dynamic animation already (an Apricorn tree-like object on
// Route 29), so a reserve of 2 animations left no room for any ghost pose.
// Vanilla loads beyond the reserve are covered by not starting poses during
// events and by the short-budget steps in Coop_UpdateGhosts
// (docs/MILESTONE2_PLAN.md 7.3, docs/KNOWN_RISKS.md).
#define COOP_RESERVE_TEXTURES  2
#define COOP_RESERVE_MODELS    1
#define COOP_RESERVE_SEQUENCES 1

#define COOP_BUDGET_IDS (32 + 2 * COOP_REMOTE_SLOTS)

// One resource kind: the IDs loaded (resident or dynamic) or claimed by a
// ghost whose billboard is still queued, and the dynamic slots left.
struct CoopIdSet
{
    u16 ids[COOP_BUDGET_IDS];
    s32 count;
    s32 free;
};

struct CoopBudget
{
    struct CoopIdSet textures; // texture ID = sprite ID
    struct CoopIdSet models;
    struct CoopIdSet sequences;
};

static void Coop_IdSetInit(struct CoopIdSet *set, const u32 *ids, u32 resident, u32 dynamic, u32 freeId)
{
    u32 i;

    set->count = 0;
    set->free = 0;
    for (i = 0; i < resident + dynamic; i++)
    {
        if (ids[i] == freeId)
            set->free += i >= resident;
        else
            set->ids[set->count++] = (u16)ids[i];
    }
}

static BOOL Coop_IdSetHas(const struct CoopIdSet *set, u32 id)
{
    s32 i;

    for (i = 0; i < set->count; i++)
    {
        if (set->ids[i] == id)
            return TRUE;
    }
    return FALSE;
}

static BOOL Coop_IdSetFits(const struct CoopIdSet *set, u32 id, s32 reserve)
{
    return set->free - (Coop_IdSetHas(set, id) ? 0 : 1) >= reserve;
}

static void Coop_IdSetClaim(struct CoopIdSet *set, u32 id)
{
    if (Coop_IdSetHas(set, id) || set->count >= COOP_BUDGET_IDS)
        return;
    set->ids[set->count++] = (u16)id;
    set->free--;
}

static BOOL Coop_BudgetFits(const struct CoopBudget *budget, u32 sprite)
{
    const struct CoopPlayerSprite *info = Coop_FindPlayerSprite(sprite);

    return info != NULL
        && Coop_IdSetFits(&budget->textures, sprite, COOP_RESERVE_TEXTURES)
        && Coop_IdSetFits(&budget->models, info->model, COOP_RESERVE_MODELS)
        && Coop_IdSetFits(&budget->sequences, info->sequence, COOP_RESERVE_SEQUENCES);
}

static void Coop_BudgetClaim(struct CoopBudget *budget, u32 sprite)
{
    const struct CoopPlayerSprite *info = Coop_FindPlayerSprite(sprite);

    if (info == NULL)
        return;
    Coop_IdSetClaim(&budget->textures, sprite);
    Coop_IdSetClaim(&budget->models, info->model);
    Coop_IdSetClaim(&budget->sequences, info->sequence);
}

// Everything the sprite needs is already loaded or claimed.
static BOOL Coop_BudgetHas(const struct CoopBudget *budget, u32 sprite)
{
    const struct CoopPlayerSprite *info = Coop_FindPlayerSprite(sprite);

    return info != NULL
        && Coop_IdSetHas(&budget->textures, sprite)
        && Coop_IdSetHas(&budget->models, info->model)
        && Coop_IdSetHas(&budget->sequences, info->sequence);
}

// Something else ate into the reserve: give it back.
static BOOL Coop_BudgetShort(const struct CoopBudget *budget)
{
    return budget->textures.free < COOP_RESERVE_TEXTURES
        || budget->models.free < COOP_RESERVE_MODELS
        || budget->sequences.free < COOP_RESERVE_SEQUENCES;
}

// Follower sprites use resident models (0, 10) and the resident sequence
// 0x15 (billboard entries 19 and 20, from the hg-engine overworld table's
// size values), so they only cost their texture.
static BOOL Coop_BudgetFitsTexture(const struct CoopBudget *budget, u32 sprite)
{
    return Coop_IdSetFits(&budget->textures, sprite, COOP_RESERVE_TEXTURES);
}

// What the renderer holds now, plus what existing ghosts hold or wait for
// (a new billboard loads its model and sequence only once its texture is
// in VRAM, a few frames later). Retail never frees a slot it cannot track
// (the insert fails into a silent assert), so a bad header claims nothing
// is free.
static void Coop_BudgetInit(struct CoopBudget *budget, MapObjectMan *manager)
{
    const struct CoopMapObjectRender *render = (const void *)((const u8 *)manager + COOP_RENDER_OFFSET);
    u32 slot;

    if (render->residentTextureCount + render->dynamicTextureCount > 32
        || render->residentModelCount + render->dynamicModelCount > 8
        || render->residentSequenceCount + render->dynamicSequenceCount > 8)
    {
        __builtin_memset(budget, 0, sizeof(*budget));
        return;
    }
    Coop_IdSetInit(&budget->textures, render->textureIds, render->residentTextureCount, render->dynamicTextureCount, COOP_FREE_TEXTURE_ID);
    Coop_IdSetInit(&budget->models, render->modelIds, render->residentModelCount, render->dynamicModelCount, COOP_FREE_MODEL_ID);
    Coop_IdSetInit(&budget->sequences, render->sequenceIds, render->residentSequenceCount, render->dynamicSequenceCount, COOP_FREE_SEQUENCE_ID);
    for (slot = 0; slot < COOP_REMOTE_SLOTS; slot++)
    {
        const LocalMapObject *ghost = Coop_FindObjectById(manager, COOP_GHOST_ID_FIRST + slot);
        const LocalMapObject *follower = Coop_FindObjectById(manager, COOP_FOLLOWER_ID_FIRST + slot);
        if (ghost != NULL)
            Coop_BudgetClaim(budget, (u32)ghost->gfxId);
        if (follower != NULL)
            Coop_IdSetClaim(&budget->textures, (u32)follower->gfxId);
    }
}

// ---------------------------------------------------------------------------
// Player ghosts: reconcile, movement, states (docs/MILESTONE2_PLAN.md 5.4)
// ---------------------------------------------------------------------------

// Where a ghost should be: a remote player, or their follower.
struct CoopGhostTarget
{
    s32 x;
    s32 z;
    u32 direction;
    u32 action;
    u8 seq;
};

static void Coop_PlayerTarget(const struct CoopPlayerState *remote, struct CoopGhostTarget *target)
{
    target->x = remote->x;
    target->z = remote->z;
    target->direction = remote->direction;
    target->action = remote->move_action;
    target->seq = remote->move_seq;
}

static void Coop_FollowerTarget(const struct CoopPlayerState *remote, struct CoopGhostTarget *target)
{
    target->x = remote->follower_x;
    target->z = remote->follower_z;
    target->direction = remote->follower_direction;
    target->action = remote->follower_move_action;
    target->seq = remote->follower_move_seq;
}

// Ticks one tile takes for a 1-tile walk / run action, 0 for anything else
// (ccd map_object_movement_commands.c durations).
static u32 Coop_StepTicks(u32 action)
{
    if (action >= 4 && action <= 23)
        return 32u >> ((action - 4) / 4); // slower 32, slow 16, normal 8, fast 4, faster 2 (bike)
    if (action >= 76 && action <= 79)
        return 6;
    if (action >= 80 && action <= 83)
        return 3;
    if (action >= 84 && action <= 87)
        return 1;
    if (action >= 88 && action <= 91)
        return 4; // run (running-shoes animation)
    if (action >= 96 && action <= 99)
        return 7;
    return 0;
}

// The step action (direction 0) for a speed: the target's own group when
// that is the speed asked for (keeps its animation), else the nearest group
// at least that fast.
static u32 Coop_StepActionForTicks(u32 ticks, u32 targetAction)
{
    if (targetAction != 0xFF && Coop_StepTicks(targetAction) == ticks)
        return targetAction & ~3u;
    if (ticks >= 8)
        return 12; // normal walk
    if (ticks >= 4)
        return 16;
    if (ticks >= 2)
        return 20;
    return 84;
}

static u32 Coop_SetGhostAction(LocalMapObject *ghost, u32 action, u8 seq)
{
    MapObject_SetHeldMovement(ghost, action);
    return COOP_SEQ_VALID | seq;
}

// One decision per idle ghost: replay the target's latest action exactly
// when that lands on the target's tile (steps, runs, bike speed, ledge
// jumps, turns); otherwise step toward it at the target's own speed, one
// speed faster when 2 or more tiles behind (walk -> run -> bike -> fastest),
// so a ghost never falls further behind a cyclist. Held movements ignore
// collision, like the ghost itself. handled is the last move_seq handled
// (| COOP_SEQ_VALID); returns the new one.
static u32 Coop_DriveGhost(LocalMapObject *ghost, const struct CoopGhostTarget *target, u32 handled)
{
    s32 gx = ghost->xCurr, gz = ghost->yCurr;
    s32 dx, dz, distance;
    u32 direction;

    if (!Coop_GhostIsIdle(ghost))
        return handled;

    if (handled != (COOP_SEQ_VALID | target->seq) && target->action != 0xFF)
    {
        distance = Coop_ActionDistance(target->action);
        Coop_DirectionDelta(target->action & 3, &dx, &dz);
        if (distance >= 0 && gx + dx * distance == target->x && gz + dz * distance == target->z)
        {
            // On-the-spot walks and hops (the bike's turn is one) block the
            // ghost for up to 16 ticks while the target already moves on:
            // replay them as an instant turn.
            if (distance == 0 && target->action >= 24)
                return Coop_SetGhostAction(ghost, COOP_MOVEMENT_FACE + (target->action & 3), target->seq);
            return Coop_SetGhostAction(ghost, target->action, target->seq);
        }
    }

    dx = target->x - gx;
    dz = target->z - gz;
    if (dx != 0 || dz != 0)
    {
        u32 ticks = Coop_StepTicks(target->action);

        if (ticks == 0)
            ticks = 4; // the target stopped or turned: catch up at running speed
        if (Coop_Abs(dx) + Coop_Abs(dz) > 1)
            ticks = ticks > 1 ? ticks / 2 : 1;
        if (Coop_Abs(dx) >= Coop_Abs(dz))
            direction = dx < 0 ? 2 : 3; // DIR_WEST / DIR_EAST
        else
            direction = dz < 0 ? 0 : 1; // DIR_NORTH / DIR_SOUTH
        return Coop_SetGhostAction(ghost, Coop_StepActionForTicks(ticks, target->action) + direction, target->seq);
    }
    if ((u32)ghost->curFacing != target->direction)
        return Coop_SetGhostAction(ghost, COOP_MOVEMENT_FACE + target->direction, target->seq);
    return COOP_SEQ_VALID | target->seq; // in sync
}

// Far behind while the ghost itself is off screen: delete, so it is
// recreated on the target's tile next frame. The target may be on screen:
// the ghost then appears there like a player arriving, instead of trailing
// unseen (it never jumps while visible).
static BOOL Coop_ShouldSnap(LocalMapObject *ghost, const LocalMapObject *player, s32 x, s32 z)
{
    return Coop_Abs(x - ghost->xCurr) + Coop_Abs(z - ghost->yCurr) > COOP_CATCHUP_MAX
        && !Coop_IsWithin(ghost->xCurr, ghost->yCurr, player, COOP_SCREEN_RADIUS_X, COOP_SCREEN_RADIUS_Z)
        && Coop_GhostIsIdle(ghost);
}

// Overlay 01 (rom.ld): the "!" / "!!" mark over a map object (ccd
// EmoteMark_Spawn). It pops up once and stays until destroyed; it keeps
// reading its object, so it must go before the object does.
void *LONG_CALL EmoteMark_Spawn(LocalMapObject *object, u32 kind, BOOL playSound);

#define COOP_MARK_EXCLAMATION 0 // ccd EMOTE_MARK_EXCLAMATION: "!"
#define COOP_MARK_DOUBLE      1 // ccd EMOTE_MARK_QUESTION, but it draws "!!" in HGSS (no "?" in this set)
#define COOP_MARK_NONE        2

static void Coop_ClearMark(LocalMapObject *ghost)
{
    u32 mark = (u32)COOP_GHOST_MARK(ghost);

    if (mark != 0)
        FieldEffectManager_DestroyEffect((void *)(mark & ~1u));
    COOP_GHOST_MARK(ghost) = 0;
}

// A remote in a battle or a menu: the ghost stays on its last tile under a
// steady "!!" (battle) or "!" (menu, bag, other app). HGSS has "?" only in
// the emote balloon, which shows one balloon at a time and animates out.
// Map transitions and the field show none.
static void Coop_UpdateMark(LocalMapObject *ghost, const struct CoopPlayerState *remote)
{
    u32 mark = (u32)COOP_GHOST_MARK(ghost);
    u32 kind = COOP_MARK_NONE;
    void *effect;

    if (!remote->is_active && remote->activity == COOP_ACTIVITY_BATTLE)
        kind = COOP_MARK_DOUBLE;
    else if (!remote->is_active && remote->activity == COOP_ACTIVITY_MENU)
        kind = COOP_MARK_EXCLAMATION;
    if (mark != 0 && (mark & 1u) == kind)
        return;
    Coop_ClearMark(ghost);
    if (kind == COOP_MARK_NONE)
        return;
    effect = EmoteMark_Spawn(ghost, kind, FALSE);
    if (effect != NULL)
        COOP_GHOST_MARK(ghost) = (int)((u32)effect | kind);
}

// A pose sprite shows the remote's animation (fishing cast / bite / reel,
// the save pose...), set by tasks there and read by the draw code here.
// Walking, cycling and swimming animations come from the replayed movements.
static void Coop_UpdatePoseAnim(LocalMapObject *ghost, const struct CoopPlayerState *remote)
{
    const struct CoopPlayerSprite *info = Coop_FindPlayerSprite((u32)ghost->gfxId);

    if (info != NULL && info->posed && (u32)ghost->gfxId == (u32)remote->sprite_id && Coop_GhostIsIdle(ghost))
        ghost->unkA0 = remote->anim; // pret LocalMapObject.animation
}

// The mount is a field effect (no map-object renderer slots). It removes
// itself once its object is gone (SurfMountEffect_Update checks ID and map),
// so only a dismount needs this.
static void Coop_UpdateSurfMount(LocalMapObject *ghost, const struct CoopPlayerState *remote)
{
    void *mount = (void *)COOP_GHOST_MOUNT(ghost);

    if (remote->avatar_state == COOP_PLAYER_STATE_SURFING && mount == NULL)
        COOP_GHOST_MOUNT(ghost) = (int)SurfMountEffect_Create(ghost, (u32)ghost->xCurr, (u32)ghost->yCurr,
                                                              (u32)ghost->curFacing, TRUE);
    else if (remote->avatar_state != COOP_PLAYER_STATE_SURFING && mount != NULL)
    {
        FieldEffectManager_DestroyEffect(mount);
        COOP_GHOST_MOUNT(ghost) = 0;
    }
}

// Deletes a ghost or follower and frees the renderer slots only it used, as
// a vanilla sprite change does (deleting alone leaves them loaded until a
// zone change).
static void Coop_RemoveGhost(MapObjectMan *manager, LocalMapObject *ghost)
{
    u32 sprite = (u32)ghost->gfxId;

    // Follower params belong to the follower draw code (param[2] = shiny).
    if ((u32)ghost->id < COOP_FOLLOWER_ID_FIRST)
    {
        if (COOP_GHOST_MOUNT(ghost) != 0)
            FieldEffectManager_DestroyEffect((void *)COOP_GHOST_MOUNT(ghost));
        COOP_GHOST_MOUNT(ghost) = 0;
        Coop_ClearMark(ghost);
    }
    DeleteMapObject(ghost);
    MapObjectRender_ReleaseSpriteResources(manager, sprite, NULL);
}

// The ghost's sprite this frame: the remote's when it fits the budget, else
// walking when that fits; a ghost not walking goes back to walking (when
// that costs nothing) once something else needs the reserve. No new poses
// while a vanilla event runs (allowPoses), so a script adding NPCs keeps
// its room. Changes keep the old billboard on screen until the new one
// loads (vanilla's own state changes do the same).
// Returns FALSE when the ghost is left in a pose the remote no longer has
// and can switch to nothing: the caller respawns it (deleting frees its
// slots exactly, and the spawn picks what fits), so a pose never sticks.
static BOOL Coop_UpdateGhostSprite(struct CoopBudget *budget, LocalMapObject *ghost, const struct CoopPlayerState *remote,
                                   BOOL allowPoses)
{
    u32 current = (u32)ghost->gfxId;
    u32 wanted = Coop_WantedSprite(remote);
    u32 walking = Coop_WalkingSprite(remote);
    u32 target = current;

    if (!allowPoses && wanted != current)
        wanted = walking;
    if (current != wanted && Coop_BudgetFits(budget, wanted))
        target = wanted;
    else if (current != walking && current != wanted && Coop_BudgetFits(budget, walking))
        target = walking; // the new state does not fit
    else if (current != walking && Coop_BudgetShort(budget) && Coop_BudgetHas(budget, walking))
        target = walking; // vanilla needs the room back
    if (target == current)
        return current == wanted || current == walking;
    ChangeMapObjSprite(ghost, target);
    ghost->unkA0 = 0; // a pose's animation index means something else to the new sprite
    Coop_BudgetClaim(budget, target);
    return TRUE;
}

// Reconciles one remote slot with its ghost: create, drive, snap or delete.
// valid: the remote is readable and in the same place.
// The local player just came back from a menu, battle or app: a ghost more
// than a tile from its target is moved there at once (deleted now,
// recreated on the target's tile next tick) while the screen is still
// fading in, instead of visibly racing there.
#define COOP_RETURN_SNAP_MIN 2

static BOOL Coop_ShouldSnapOnReturn(BOOL returned, const LocalMapObject *ghost, s32 x, s32 z)
{
    return returned && Coop_Abs(x - ghost->xCurr) + Coop_Abs(z - ghost->yCurr) >= COOP_RETURN_SNAP_MIN;
}

static void Coop_UpdateGhost(MapObjectMan *manager, LocalMapObject *player, struct CoopBudget *budget, u32 slot,
                             const struct CoopPlayerState *remote, BOOL valid, BOOL allowPoses, BOOL returned)
{
    LocalMapObject *ghost = Coop_FindObjectById(manager, COOP_GHOST_ID_FIRST + slot);
    struct CoopGhostTarget target;
    u32 sprite;

    if (ghost != NULL)
    {
        // Gone, elsewhere, out of range, another player in this slot, or
        // far behind off screen: remove (the next frame may recreate it).
        if (!valid
            || (u32)COOP_GHOST_TRAINER(ghost) != remote->trainer_id
            || !Coop_IsWithin(remote->x, remote->z, player, COOP_DESPAWN_RADIUS_X, COOP_DESPAWN_RADIUS_Z)
            || Coop_ShouldSnap(ghost, player, remote->x, remote->z)
            || Coop_ShouldSnapOnReturn(returned, ghost, remote->x, remote->z)
            || !Coop_UpdateGhostSprite(budget, ghost, remote, allowPoses))
        {
            Coop_RemoveGhost(manager, ghost);
            return;
        }
        Coop_UpdateSurfMount(ghost, remote);
        Coop_UpdateMark(ghost, remote);
        if (remote->is_active)
        {
            Coop_PlayerTarget(remote, &target);
            COOP_GHOST_SEQ(ghost) = (int)Coop_DriveGhost(ghost, &target, (u32)COOP_GHOST_SEQ(ghost));
        }
        Coop_UpdatePoseAnim(ghost, remote);
        return;
    }

    if (!valid || !Coop_IsWithin(remote->x, remote->z, player, COOP_SPAWN_RADIUS_X, COOP_SPAWN_RADIUS_Z))
        return;
    sprite = allowPoses ? Coop_WantedSprite(remote) : Coop_WalkingSprite(remote);
    if (!Coop_BudgetFits(budget, sprite))
    {
        sprite = Coop_WalkingSprite(remote);
        if (!Coop_BudgetFits(budget, sprite))
            return;
    }
    ghost = Coop_CreateGhostObject(manager, COOP_GHOST_ID_FIRST + slot, remote->x, remote->z, remote->direction, sprite,
                                   remote->height);
    if (ghost == NULL)
        return;
    Coop_BudgetClaim(budget, sprite);
    COOP_GHOST_TRAINER(ghost) = (int)remote->trainer_id;
    COOP_GHOST_SEQ(ghost) = COOP_SEQ_VALID | remote->move_seq;
    COOP_GHOST_MOUNT(ghost) = 0;
    COOP_GHOST_MARK(ghost) = 0;
    Coop_UpdateSurfMount(ghost, remote);
    Coop_UpdateMark(ghost, remote);
}

// ---------------------------------------------------------------------------
// Remote followers (plan 3.3, 5.8; Phase 4)
// ---------------------------------------------------------------------------

void Coop_FollowMonBillboard_SetResource(BOOL alternate, LocalMapObject *object, void *billboard);
BOOL LONG_CALL MapObject_IsBillboardPending(LocalMapObject *object);

// Vanilla picks the shiny sequence once per object, on its first draw update
// with a billboard (draw byte 0x17 bit 0), which a recycled object slot can
// skip. So the follower ghost gets it here once its billboard exists (the
// first word of the billboard draw data, pret LocalMapObject.unk108).
static void Coop_ApplyFollowerShiny(LocalMapObject *follower, u32 slot)
{
    void *billboard;

    if (!(follower->param[2] & 1) || (gCoopCommBlock.track_follower_shiny & (1u << slot)))
        return;
    __builtin_memcpy(&billboard, follower->unk108, sizeof(billboard));
    if (billboard == NULL || MapObject_IsBillboardPending(follower))
        return;
    Coop_FollowMonBillboard_SetResource(TRUE, follower, billboard);
    gCoopCommBlock.track_follower_shiny |= (u8)(1u << slot);
}

#define COOP_MON_FEMALE 1 // pret MON_FEMALE

// Overworld tag the remote's follower shows, worked out here from species,
// form and gender with hg-engine's own lookup (what the local follower
// uses), never taken from the network: an unknown tag would break the
// renderer. 0 = none (no follower, in its ball, or no overworld).
static u32 Coop_FollowerSprite(const struct CoopPlayerState *remote)
{
    u32 tag;

    // Vanilla recalls the follower while cycling or surfing; follow the state
    // too, in case the published flag lags behind.
    if (remote->follower_species == 0 || !remote->follower_visible
        || remote->avatar_state == COOP_PLAYER_STATE_CYCLING || remote->avatar_state == COOP_PLAYER_STATE_SURFING)
        return 0;
    tag = get_mon_ow_tag(remote->follower_species, remote->follower_form, remote->follower_gender == COOP_MON_FEMALE);
    return ObjectEvent_GetGraphicsInfo(tag) != NULL ? tag : 0;
}

// A follower ghost: the follower's sprite and draw params (species, shiny;
// no form-size params, vanilla reads those only for its own follower ID),
// moved like a player ghost. Shown only next to its player's ghost, so it
// goes away with it (another trainer in the slot, a snap) and the players
// always get the budget first. Its last handled move_seq lives in the
// mailbox: follower params belong to the follower draw code.
static void Coop_UpdateFollower(MapObjectMan *manager, LocalMapObject *player, struct CoopBudget *budget, u32 slot,
                                const struct CoopPlayerState *remote, BOOL valid, BOOL returned)
{
    LocalMapObject *follower = Coop_FindObjectById(manager, COOP_FOLLOWER_ID_FIRST + slot);
    BOOL hasGhost = Coop_FindObjectById(manager, COOP_GHOST_ID_FIRST + slot) != NULL;
    u32 sprite = (valid && hasGhost) ? Coop_FollowerSprite(remote) : 0;
    struct CoopGhostTarget target;

    if (follower != NULL)
    {
        if (sprite == 0
            || (u32)follower->gfxId != sprite
            || (u32)(follower->param[2] & 1) != remote->follower_shiny
            || !Coop_IsWithin(remote->follower_x, remote->follower_z, player, COOP_DESPAWN_RADIUS_X, COOP_DESPAWN_RADIUS_Z)
            || Coop_ShouldSnap(follower, player, remote->follower_x, remote->follower_z)
            || Coop_ShouldSnapOnReturn(returned, follower, remote->follower_x, remote->follower_z))
        {
            Coop_RemoveGhost(manager, follower);
            return;
        }
        Coop_ApplyFollowerShiny(follower, slot);
        if (remote->is_active)
        {
            Coop_FollowerTarget(remote, &target);
            gCoopCommBlock.track_follower_seq[slot] =
                (u8)Coop_DriveGhost(follower, &target, COOP_SEQ_VALID | gCoopCommBlock.track_follower_seq[slot]);
        }
        return;
    }

    if (sprite == 0
        || !Coop_IsWithin(remote->follower_x, remote->follower_z, player, COOP_SPAWN_RADIUS_X, COOP_SPAWN_RADIUS_Z)
        || !Coop_BudgetFitsTexture(budget, sprite))
        return;
    follower = Coop_CreateGhostObject(manager, COOP_FOLLOWER_ID_FIRST + slot, remote->follower_x, remote->follower_z,
                                      remote->follower_direction, sprite, remote->follower_height);
    if (follower == NULL)
        return;
    Coop_IdSetClaim(&budget->textures, sprite);
    follower->param[0] = remote->follower_species; // pret FOLLOW_MON_PARAM_SPECIES
    follower->param[1] = 0;
    follower->param[2] = remote->follower_shiny ? 1 : 0; // FollowMon_SetObjectShiny
    gCoopCommBlock.track_follower_seq[slot] = remote->follower_move_seq;
    gCoopCommBlock.track_follower_shiny &= (u8)~(1u << slot);
}

// Overlay-01 hook (hooks: ov01_0220553C, ccd MapObject_IsShinyPokemon; the
// whole function is replaced). Vanilla reads the shiny bit only for its own
// follower IDs; remote followers carry it the same way.
BOOL Coop_MapObject_IsShinyPokemon(LocalMapObject *object)
{
    u32 id = (u32)object->id;

    if (id == 0xFD || id == 0xFA || id == 0xFB
        || (id >= COOP_FOLLOWER_ID_FIRST && id < COOP_FOLLOWER_ID_FIRST + COOP_REMOTE_SLOTS))
        return object->param[2] & 1;
    return FALSE;
}

// ccd BillboardResource (include/billboard_internal.h), filled by vanilla;
// gfxSequence (ccd BillboardGfxSequence) is written here.
struct CoopBillboardResource
{
    void *modelFile;
    void *texture;
    const void *ranges;
    const u16 *startFrame;
    const u8 *textureIdx;
    const u8 *plttIdx;
    u32 seqCount;
    u32 texKey;
    u32 tex4x4Key;
    u32 plttKey;
};

_Static_assert(sizeof(struct CoopBillboardResource) == 0x28, "BillboardResource is 0x28 bytes (ccd billboard_internal.h)");

BOOL LONG_CALL MapObjectRender_GetBillboardResource(MapObjectMan *manager, u32 sprite, struct CoopBillboardResource *dest);
const void *LONG_CALL ResourceBank_Get(void *bank, u32 id);
u32 LONG_CALL Facing_GetBillboardStandSequence(u32 direction);
void LONG_CALL BillboardObject_SetResource(void *billboard, struct CoopBillboardResource *resource);
void LONG_CALL BillboardObject_SetSequence(void *billboard, u32 sequence);
void LONG_CALL BillboardObject_SetSequenceFrame(void *billboard, u32 frame);

#define COOP_RENDER_SEQUENCE_BANK  0xFC   // ccd MapObjectRender.sequenceBank
#define COOP_SEQUENCE_FOLLOW_MON   0x15   // ccd BILLBOARD_SEQUENCE_FOLLOW_MON
#define COOP_SEQUENCE_FOLLOW_SHINY 0x16   // ccd BILLBOARD_SEQUENCE_FOLLOW_MON_ALT (shiny palette frames)

// ccd BillboardGfxSequence_Init (C in pret, unmapped): points the sequence at
// its frame table.
static void Coop_InitGfxSequence(const void *data, struct CoopBillboardResource *resource)
{
    const u8 *bytes = data;

    __builtin_memcpy(&resource->seqCount, bytes, sizeof(u32));
    bytes += sizeof(u32);
    resource->startFrame = (const u16 *)bytes;
    bytes += sizeof(u16) * resource->seqCount;
    resource->textureIdx = bytes;
    bytes += resource->seqCount;
    resource->plttIdx = bytes;
}

// Overlay-01 hook (hooks: ov01_02205808, ccd FollowMonBillboard_SetResource;
// the whole function is replaced): the same steps, with the remote follower
// IDs accepted next to vanilla's. The shiny look is the alternate animation
// sequence 0x16 (palette frames), picked when IsShinyPokemon says so.
void Coop_FollowMonBillboard_SetResource(BOOL alternate, LocalMapObject *object, void *billboard)
{
    struct CoopBillboardResource resource;
    MapObjectMan *manager = (MapObjectMan *)object->unkB4; // pret LocalMapObject.manager
    const u8 *render = (const u8 *)manager + COOP_RENDER_OFFSET;
    void *bank;
    u32 id = (u32)object->id;

    MapObjectRender_GetBillboardResource(manager, (u32)object->gfxId, &resource);
    if (id != 0xFD && id != 0xFA && id != 0xFB
        && !(id >= COOP_FOLLOWER_ID_FIRST && id < COOP_FOLLOWER_ID_FIRST + COOP_REMOTE_SLOTS))
        return;
    __builtin_memcpy(&bank, render + COOP_RENDER_SEQUENCE_BANK, sizeof(bank));
    Coop_InitGfxSequence(ResourceBank_Get(bank, alternate ? COOP_SEQUENCE_FOLLOW_SHINY : COOP_SEQUENCE_FOLLOW_MON), &resource);
    BillboardObject_SetResource(billboard, &resource);
    BillboardObject_SetSequence(billboard, Facing_GetBillboardStandSequence((u32)object->curFacing));
    BillboardObject_SetSequenceFrame(billboard, 0);
}

// ---------------------------------------------------------------------------
// Priorities and fallbacks (plan 5.5): nearest teammates first; when vanilla
// needs room back, followers go first (furthest first), then the furthest
// players. At most one removal every 16 ticks (Coop_UpdateGhosts).
// ---------------------------------------------------------------------------

static void Coop_SortByDistance(u8 *order, const struct CoopPlayerState *remotes, const BOOL *valid, const LocalMapObject *player)
{
    s32 distance[COOP_REMOTE_SLOTS];
    u32 i, j;

    for (i = 0; i < COOP_REMOTE_SLOTS; i++)
    {
        distance[i] = valid[i] ? Coop_Abs(remotes[i].x - player->xCurr) + Coop_Abs(remotes[i].z - player->yCurr) : 0x7FFF;
        for (j = i; j > 0 && distance[order[j - 1]] > distance[i]; j--)
            order[j] = order[j - 1];
        order[j] = (u8)i;
    }
}

// Texture or object slots short: drop a follower, else a player ghost.
// Animation or model slots short with every ghost already walking (the
// sprite downgrade has nothing left to give): drop a player ghost.
static BOOL Coop_ShedGhost(MapObjectMan *manager, const struct CoopBudget *budget, const u8 *order, const struct CoopPlayerState *remotes)
{
    BOOL roomShort = budget->textures.free < COOP_RESERVE_TEXTURES
        || manager->active_count + COOP_OBJECT_HEADROOM > manager->object_count;
    BOOL animShort = budget->models.free < COOP_RESERVE_MODELS || budget->sequences.free < COOP_RESERVE_SEQUENCES;
    LocalMapObject *object;
    s32 i;

    if (roomShort)
    {
        for (i = COOP_REMOTE_SLOTS - 1; i >= 0; i--)
        {
            object = Coop_FindObjectById(manager, COOP_FOLLOWER_ID_FIRST + order[i]);
            if (object != NULL)
            {
                Coop_RemoveGhost(manager, object);
                return TRUE;
            }
        }
    }
    else if (animShort)
    {
        for (i = 0; i < COOP_REMOTE_SLOTS; i++)
        {
            object = Coop_FindObjectById(manager, COOP_GHOST_ID_FIRST + i);
            if (object != NULL && (u32)object->gfxId != Coop_WalkingSprite(&remotes[i]))
                return FALSE; // a downgrade will free it
        }
    }
    else
        return FALSE;
    for (i = COOP_REMOTE_SLOTS - 1; i >= 0; i--)
    {
        object = Coop_FindObjectById(manager, COOP_GHOST_ID_FIRST + order[i]);
        if (object != NULL)
        {
            Coop_RemoveGhost(manager, object);
            return TRUE;
        }
    }
    return FALSE;
}

static void Coop_UpdateGhosts(FieldSystem *fsys, MapObjectMan *manager, LocalMapObject *player)
{
    struct CoopPlayerState remotes[COOP_REMOTE_SLOTS];
    BOOL valid[COOP_REMOTE_SLOTS];
    u8 order[COOP_REMOTE_SLOTS];
    struct CoopBudget budget;
    BOOL returned = gCoopCommBlock.track_field_hidden == 1;
    u32 i;

    gCoopCommBlock.track_field_hidden = 0;
    for (i = 0; i < COOP_REMOTE_SLOTS; i++)
        valid[i] = Coop_ReadRemotePlayer(i, &remotes[i]) && Coop_IsSamePlace(fsys, &remotes[i]);
    Coop_SortByDistance(order, remotes, valid, player);

    Coop_BudgetInit(&budget, manager);
    // Short (or objects nearly full): every 16 ticks, first let vanilla free
    // loaded-but-unused slots (what it does on zone changes; it walks every
    // object, hence the rate limit), and only if still short delete a ghost.
    // Pose downgrades (Coop_UpdateGhostSprite) react every frame.
    if ((gCoopCommBlock.diag.field_entry_calls & 15) == 0
        && (Coop_BudgetShort(&budget) || manager->active_count + COOP_OBJECT_HEADROOM > manager->object_count))
    {
        MapObjectRender_ReleaseUnused(manager, (u8 *)manager + COOP_RENDER_OFFSET);
        Coop_BudgetInit(&budget, manager);
        if (Coop_ShedGhost(manager, &budget, order, remotes))
            Coop_BudgetInit(&budget, manager); // slots were freed
    }
    for (i = 0; i < COOP_REMOTE_SLOTS; i++)
        Coop_UpdateGhost(manager, player, &budget, order[i], &remotes[order[i]], valid[order[i]], fsys->taskman == NULL, returned);
    for (i = 0; i < COOP_REMOTE_SLOTS; i++)
        Coop_UpdateFollower(manager, player, &budget, order[i], &remotes[order[i]], valid[order[i]], returned);
}

// ---------------------------------------------------------------------------
// Talking to a ghost (plan 5.7): ghosts carry NO_INTERACT, so vanilla's A
// press finds nothing (or an event behind the ghost, which wins: the field
// runs before this SysTask). Needs a frame with no field task or message
// before this one, so the A that closes a sign does not open a ghost talk.
// ---------------------------------------------------------------------------

static void Coop_CheckGhostTalk(FieldSystem *fsys, MapObjectMan *manager, LocalMapObject *player)
{
    BOOL idle = fsys->taskman == NULL && !fsys->textbox_open;
    BOOL wasIdle = gCoopCommBlock.track_field_idle == 1;
    struct CoopPlayerState remote;
    s32 dx, dz;
    u32 slot;

    gCoopCommBlock.track_field_idle = idle ? 1 : 0;
    if (!idle || !wasIdle || !(gSystem.newKeys & PAD_BUTTON_A))
        return;
    if (gCoopCommBlock.disconnect_state != COOP_DISCONNECT_NONE || MapObject_IsSingleMovementActive(player)
        || Coop_IsMapNameBannerShowing(fsys))
        return;
    Coop_DirectionDelta((u32)player->curFacing, &dx, &dz);
    for (slot = 0; slot < COOP_REMOTE_SLOTS; slot++)
    {
        LocalMapObject *ghost = Coop_FindObjectById(manager, COOP_GHOST_ID_FIRST + slot);
        if (ghost == NULL || ghost->xCurr != player->xCurr + dx || ghost->yCurr != player->yCurr + dz)
            continue;
        if (!Coop_ReadRemotePlayer(slot, &remote) || remote.trainer_id != (u32)COOP_GHOST_TRAINER(ghost))
            continue;
        // Free in the field after the adventure started: Trade / Battle /
        // Team up. Otherwise just their name, and why they are busy (a
        // battle, an app, or a message / menu in the field).
        if (remote.is_active && !remote.field_busy && Coop_IsAdventureStarted() && Coop_IsConnectedToRoom())
            (void)Coop_StartSessionMenu(fsys, slot, &remote);
        else if (remote.is_active)
            (void)Coop_StartTalkDialog(fsys, remote.name, remote.field_busy ? COOP_ACTIVITY_MENU : COOP_ACTIVITY_FIELD);
        else
            (void)Coop_StartTalkDialog(fsys, remote.name, remote.activity);
        return;
    }
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
    Coop_UpdateGhosts(fsys, manager, player);
    Coop_CheckGhostTalk(fsys, manager, player);
    Coop_UpdateTrade(fsys);
    Coop_UpdateSessionPrompt(fsys);
    Coop_ProcessDebugCommand(fsys, manager, player);
    Coop_ReportTestGhost(manager, player);
    Coop_UpdateFieldConnection(fsys); // last, as when the co-op SysTask ran it after this entry
}
