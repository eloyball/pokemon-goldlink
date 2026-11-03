#include "config.h"
#include "types.h"

#include "constants/buttons.h"
#include "constants/item.h"

#include "map_events_internal.h"
#include "message.h"
#include "pokemon.h"
#include "save.h"
#include "sound.h"
#include "system.h"
#include "task.h"
#include "coop_sync.h"
#include "coop_dialog.h"
#include "coop_choice.h"
#include "coop_challenge.h"
#pragma long_calls // Overlay 129 functions: see src/field/coop_challenge_nuzlocke.c
#include "pokemon_storage_system.h"
#pragma long_calls_off

// ---------------------------------------------------------------------------
// Milestone 5 Phase 5: Ghost Mode, the spirit state (docs/MILESTONE5_PLAN.md
// §5.5), Overlay 131. "Spirit" in code, "Ghost" on screen: "ghost" already
// means a remote player's map object everywhere else.
//
//   the sweep finds no living Pokemon (coop_challenge_nuzlocke.c)
//     -> Coop_SpiritEnter: "NAME has no Pokemon left...", the blackout spot
//        becomes return_*, the tile before the battle the anchor
//     -> warp to the anchor -> the spirit task (owns the input: one tile per
//        step, no warps, objects or zone changes; START: the spirit menu)
//     -> a teammate gives a Pokemon (coop_session.c, coop_trade.c: a
//        one-sided trade) -> Coop_SpiritRevived -> warp to return_*
//   every roster player a spirit -> the relay's "run lost" -> the summary,
//   a save, the title screen.
// ---------------------------------------------------------------------------

struct Party *LONG_CALL SaveArray_Party_Get(void *saveData);
BOOL LONG_CALL Field_SaveGameNormal(FieldSystem *fsys);
void *LONG_CALL sub_020538C0(FieldSystem *fsys, u32 mapId, int warpId, int x, int y, int direction);
// pret (rom.ld, docs/MILESTONE5_PLAN.md §11.1): vanilla's "can the follower
// step there" test in ScrCmd_FollowMonFacePlayer.
BOOL LONG_CALL sub_020549A8(FieldSystem *fsys, VecFx32 *pos, int x, int z, int unused); // TRUE: blocked
BOOL LONG_CALL sub_02060BFC(LocalMapObject *object, int x, int height, int z);          // TRUE: an object there
void LONG_CALL MapObject_CopyPositionVector(LocalMapObject *object, VecFx32 *out);
BOOL LONG_CALL MetatileBehavior_IsSurfableWater(u8 behavior);
void LONG_CALL MapID_GetLandmarkName(u32 mapId, u32 heapId, String *dest);

struct CoopMovementCommand // pret MovementScriptCommand
{
    u16 command;
    u16 length;
};

void *LONG_CALL EventObjectMovementMan_Create(LocalMapObject *object, const struct CoopMovementCommand *data);
BOOL LONG_CALL EventObjectMovementMan_IsFinish(void *movementMan);
void LONG_CALL EventObjectMovementMan_Delete(void *movementMan);

#define COOP_HEAP_FIELD          11
#define COOP_DIR_UP              0 // pret DIR_NORTH.. (constants/global_fieldmap.h)
#define COOP_DIR_DOWN            1
#define COOP_DIR_LEFT            2
#define COOP_DIR_RIGHT           3
#define COOP_MOVEMENT_FACE_UP    0  // + direction (pret constants/movements.h)
#define COOP_MOVEMENT_STEP_UP    12 // + direction
#define COOP_MOVEMENT_STEP_END   254
#define COOP_PLAYER_STATE_SURFING 2 // pret PLAYER_STATE_SURFING
#define COOP_SE_SELECT           1500
#define COOP_NAME_GLYPHS         12

// pret MapMatrix: u8 width, height, matrix_id, then MapMatrixData (u8 height,
// width, u16 headers[]) aligned to 2: the map of each 32x32 block at +6.
#define COOP_MATRIX_WIDTH   0
#define COOP_MATRIX_HEIGHT  1
#define COOP_MATRIX_HEADERS 6
#define COOP_BLOCK_TILES    32

// pret constants/maps.h: where a spirit can't stay (E27).
static const u16 sLeagueMaps[] = {
    301, 302, 303, 304, 305, 306, // the Elite Four, Lance, the Hall of Fame
    465,                          // the Mt. Silver summit
};
static const u16 sGymMaps[] = {
    80, 135, 136, 137, 138, 139, 140, 141, 180, 365, 395, 396, 397, 410, 427, 457, 473, 480, 496,
};

static BOOL Coop_MapIn(u32 map, const u16 *list, u32 count)
{
    u32 i;

    for (i = 0; i < count; i++)
        if (list[i] == map)
            return TRUE;
    return FALSE;
}

static LocalMapObject *Coop_Player(FieldSystem *fsys)
{
    return fsys->playerAvatar != NULL ? fsys->playerAvatar->mapObject : NULL;
}

static BOOL Coop_IsLivingMon(struct PartyPokemon *mon)
{
    return GetMonData(mon, MON_DATA_SPECIES_EXISTS, NULL) && !GetMonData(mon, MON_DATA_IS_EGG, NULL)
        && !(GetMonData(mon, MON_DATA_RESERVED_114, NULL) & COOP_MON_DEAD);
}

static u32 Coop_PartyLiving(struct Party *party)
{
    u32 i, n = 0;

    for (i = 0; i < (u32)party->count; i++)
        if (Coop_IsLivingMon(Party_GetMonByIndex(party, i)))
            n++;
    return n;
}

BOOL Coop_IsSpirit(void)
{
    const struct CoopChallengeSave *save = Coop_ChallengeSave();

    return save != NULL && (save->rules & COOP_RULE_NUZLOCKE) && (save->state & COOP_CHALLENGE_SPIRIT);
}

// ---------------------------------------------------------------------------
// Anchors (§5.5.1)
// ---------------------------------------------------------------------------

void Coop_ChallengeRecordPrebattle(FieldSystem *fsys)
{
    struct CoopChallengeSave *save = Coop_ChallengeSave();
    LocalMapObject *player;
    u32 map;

    if (save == NULL || !(save->rules & COOP_RULE_NUZLOCKE) || fsys == NULL || fsys->location == NULL
        || (player = Coop_Player(fsys)) == NULL)
        return;
    map = fsys->location->mapId;
    save->prebattle_map = 0; // none: a spirit stays where it blacked out
    if (fsys->playerAvatar->state == COOP_PLAYER_STATE_SURFING || Coop_MapIn(map, sLeagueMaps, NELEMS(sLeagueMaps)))
        return;
    if (Coop_MapIn(map, sGymMaps, NELEMS(sGymMaps)))
    {
        // A gym's puzzle and trainers: the spirit waits at its entrance.
        MAP_EVENTS *events = fsys->map_events;
        if (events == NULL || events->num_warp_events == 0 || events->warp_events == NULL)
            return;
        save->prebattle_x = (s16)events->warp_events[0].x;
        save->prebattle_z = (s16)events->warp_events[0].y;
    }
    else
    {
        save->prebattle_x = (s16)player->xCurr;
        save->prebattle_z = (s16)player->yCurr;
    }
    save->prebattle_map = (u16)map;
}

// ---------------------------------------------------------------------------
// A message (one or two pages) over a paused world
// ---------------------------------------------------------------------------

struct CoopSpiritMessageEnv
{
    struct CoopDialog dialog;
    const char *page2;
    u16 name[COOP_NAME_GLYPHS];
    u8 state;
};

static BOOL Coop_Task_SpiritMessage(TaskManager *taskman)
{
    FieldSystem *fsys = taskman->fieldSystem;
    struct CoopSpiritMessageEnv *env = taskman->env;

    switch (env->state)
    {
    case 0:
    case 1:
        if (Coop_DialogIsPrinting(&env->dialog) || !Coop_DialogButtonPressed())
            break;
        if (env->state == 0 && env->page2 != NULL)
        {
            Coop_DialogPrint(&env->dialog, env->page2);
            env->state = 1;
            break;
        }
        Coop_DialogClose(fsys, &env->dialog);
        MapObjectMan_UnpauseAllMovement(fsys->mapObjectMan);
        sys_FreeMemoryEz(env);
        return TRUE;
    }
    return FALSE;
}

static BOOL Coop_StartSpiritMessage(FieldSystem *fsys, const u16 *name, const char *page1, const char *page2)
{
    struct CoopSpiritMessageEnv *env;
    u32 i;

    if (!Coop_StartFieldTask(fsys, Coop_Task_SpiritMessage, sizeof(struct CoopSpiritMessageEnv)))
        return FALSE;
    env = ((TaskManager *)fsys->taskman)->env;
    for (i = 0; i < COOP_NAME_GLYPHS - 1 && name != NULL && name[i] != 0xFFFF; i++)
        env->name[i] = name[i];
    env->name[i] = 0xFFFF;
    env->page2 = page2;
    MapObjectMan_PauseAllMovement(fsys->mapObjectMan);
    Coop_DialogOpen(fsys, &env->dialog);
    env->dialog.args[0] = env->name;
    Coop_DialogPrint(&env->dialog, page1);
    return TRUE;
}

static void Coop_PlayerName(FieldSystem *fsys, u16 *out)
{
    struct PlayerProfile *profile = Sav2_PlayerData_GetProfileAddr(fsys->savedata);
    u32 i;

    out[0] = 0xFFFF;
    if (profile == NULL)
        return;
    for (i = 0; i < 7 && profile->name[i] != 0xFFFF; i++)
        out[i] = profile->name[i];
    out[i] = 0xFFFF;
}

// ---------------------------------------------------------------------------
// Entering (§5.5.2)
// ---------------------------------------------------------------------------

void Coop_SpiritEnter(FieldSystem *fsys, struct CoopChallengeSave *save)
{
    LocalMapObject *player = Coop_Player(fsys);
    u16 name[COOP_NAME_GLYPHS];

    if (player == NULL || (save->state & COOP_CHALLENGE_SPIRIT))
        return;
    // After a lost battle the blackout already brought the player to the
    // last Pokemon Center: the way back on revival.
    save->return_map = (u16)fsys->location->mapId;
    save->return_x = (s16)player->xCurr;
    save->return_z = (s16)player->yCurr;
    if (save->prebattle_map != 0)
    {
        save->anchor_map = save->prebattle_map;
        save->anchor_x = save->prebattle_x;
        save->anchor_z = save->prebattle_z;
    }
    else
    {
        // A wipe outside a battle (Soul-Link), or an unsafe place: stay here.
        save->anchor_map = save->return_map;
        save->anchor_x = save->return_x;
        save->anchor_z = save->return_z;
    }
    save->prebattle_map = 0;
    save->state |= COOP_CHALLENGE_SPIRIT | COOP_CHALLENGE_ANCHOR_WARP;
    save->spirit_flags |= COOP_SPIRIT_INTRO;
    Coop_PlayerName(fsys, name);
    Coop_StartSpiritMessage(fsys, name, "@ has no Pokémon left...",
                            "@'s spirit returns to\nwhere they fell.");
}

void Coop_SpiritRevived(FieldSystem *fsys)
{
    struct CoopChallengeSave *save = Coop_ChallengeSave();

    (void)fsys;
    if (save == NULL)
        return;
    save->state &= ~(COOP_CHALLENGE_SPIRIT | COOP_CHALLENGE_ANCHOR_WARP);
    save->state |= COOP_CHALLENGE_RETURN_WARP;
    save->spirit_flags = 0;
}

// ---------------------------------------------------------------------------
// The donor's rules (§5.5.4)
// ---------------------------------------------------------------------------

static u32 Coop_BoxLiving(FieldSystem *fsys, const struct CoopChallengeSave *save, u32 enough)
{
    PCStorage *pc = SaveArray_Get(fsys->savedata, 41); // pret SAVE_PCSTORAGE
    u32 b, s, n = 0;

    for (b = 0; b < NUM_PC_BOXES && n < enough; b++)
    {
        if (b == COOP_GRAVEYARD_BOX || (b == COOP_GRAVEYARD_BOX2 && (save->state & COOP_CHALLENGE_GRAVE2)))
            continue;
        for (s = 0; s < MONS_PER_BOX && n < enough; s++)
        {
            struct BoxPokemon *mon = &pc->boxes[b].mons[s];
            BOOL fast = BoxMonSetFastModeOn(mon);
            if (GetBoxMonData(mon, MON_DATA_SPECIES_EXISTS, NULL) && !GetBoxMonData(mon, MON_DATA_IS_EGG, NULL)
                && !(GetBoxMonData(mon, MON_DATA_RESERVED_114, NULL) & COOP_MON_DEAD))
                n++;
            BoxMonSetFastModeOff(mon, fast);
        }
    }
    return n;
}

// Why this party Pokemon can't revive a spirit. A location's linked catch
// stays with its owner only under Soul-Link, where its fate is shared (E17).
static const char *Coop_DonationRule(const struct CoopChallengeSave *save, struct PartyPokemon *mon)
{
    if (GetMonData(mon, MON_DATA_IS_EGG, NULL))
        return "An Egg can't revive\nanyone.";
    if (GetMonData(mon, MON_DATA_RESERVED_114, NULL) & COOP_MON_DEAD)
        return "It has fallen. It can't\nbe given.";
    if (IS_ITEM_MAIL(GetMonData(mon, MON_DATA_HELD_ITEM, NULL)))
        return "It's holding Mail. Take\nthe Mail first.";
    if ((save->rules & COOP_RULE_SOUL_LINK_MASK) && (GetMonData(mon, MON_DATA_RESERVED_114, NULL) & COOP_MON_LINKED))
        return "Its fate is linked to yours.\nIt can't be given.";
    return NULL;
}

const char *Coop_SpiritDonorRule(struct Party *party, u32 slot)
{
    const struct CoopChallengeSave *save = Coop_ChallengeSave();
    u32 i, living;

    if (save == NULL || !(save->rules & COOP_RULE_NUZLOCKE) || gFieldSysPtr == NULL)
        return "That isn't possible\nright now.";
    living = Coop_PartyLiving(party);
    if (slot == 0xFF)
    {
        // Two living Pokemon in all (party and boxes), one of them givable.
        if (living < 2 && living + Coop_BoxLiving(gFieldSysPtr, save, 2 - living) < 2)
            return "You need two living Pokémon\nto give one away.";
        for (i = 0; i < (u32)party->count; i++)
            if (Coop_DonationRule(save, Party_GetMonByIndex(party, i)) == NULL)
                return NULL;
        return "None of your party Pokémon\ncan be given.";
    }
    if (slot >= (u32)party->count)
        return "That isn't possible\nright now.";
    if (Coop_DonationRule(save, Party_GetMonByIndex(party, slot)) != NULL)
        return Coop_DonationRule(save, Party_GetMonByIndex(party, slot));
    for (i = 0; i < (u32)party->count; i++)
    {
        struct PartyPokemon *other = Party_GetMonByIndex(party, i);
        if (i != slot && Coop_IsLivingMon(other) && GetMonData(other, MON_DATA_HP, NULL) != 0)
            return NULL;
    }
    return "You'd have no Pokémon left\nthat can battle!";
}

// ---------------------------------------------------------------------------
// The spirit task (§5.5.3)
// ---------------------------------------------------------------------------

enum CoopSpiritTaskState
{
    COOP_SPIRIT_WAIT_STILL,
    COOP_SPIRIT_INTRO_TEXT,   // first time at the anchor: two pages, then a save
    COOP_SPIRIT_INTRO_PAGE2,
    COOP_SPIRIT_INTRO_SAVE,
    COOP_SPIRIT_INTRO_CLOSE,  // the spinner's hide reaches the screen before the window goes
    COOP_SPIRIT_ROAM,
    COOP_SPIRIT_MENU,
    COOP_SPIRIT_TEAM_PAGE,    // one page per teammate
    COOP_SPIRIT_SAVE_TEXT,
    COOP_SPIRIT_SAVE,
    COOP_SPIRIT_CLOSE_TEXT,   // a last page, then back to roaming
};

#define COOP_SPIRIT_MENU_TEAM   1
#define COOP_SPIRIT_MENU_SAVE   2
#define COOP_SPIRIT_MENU_CANCEL 3

struct CoopSpiritEnv
{
    struct CoopDialog dialog;
    struct CoopChoice choice;
    void *movement;
    struct CoopMovementCommand moves[2]; // read by the movement engine until it finishes
    u16 name[COOP_NAME_GLYPHS];
    u16 place1[COOP_NAME_GLYPHS]; // a landmark name, split for the 11-glyph arguments
    u16 place2[COOP_NAME_GLYPHS];
    u16 frames;
    u8 teamSlot; // next remote slot for the team pages
    u8 open;     // the dialog is open
};

static void Coop_SpiritOpenDialog(FieldSystem *fsys, struct CoopSpiritEnv *env)
{
    if (env->open)
        return;
    MapObjectMan_PauseAllMovement(fsys->mapObjectMan);
    Coop_DialogOpen(fsys, &env->dialog);
    env->dialog.args[0] = env->name;
    env->dialog.args[1] = env->place1;
    env->dialog.args[2] = env->place2;
    env->open = TRUE;
}

static void Coop_SpiritCloseDialog(FieldSystem *fsys, struct CoopSpiritEnv *env)
{
    Coop_ChoiceClose(&env->choice);
    if (!env->open)
        return;
    Coop_DialogClose(fsys, &env->dialog);
    MapObjectMan_UnpauseAllMovement(fsys->mapObjectMan);
    env->open = FALSE;
}

// Something else must run now: a revive (the trade task), a disconnect, the
// end of the run, or the state is gone.
static BOOL Coop_SpiritMustYield(void)
{
    const struct CoopChallengeSave *save = Coop_ChallengeSave();
    const struct CoopChallengeDown *in = Coop_ChallengeIn();

    if (save == NULL || !(save->state & COOP_CHALLENGE_SPIRIT) || (save->state & COOP_CHALLENGE_ANCHOR_WARP))
        return TRUE;
    if (gCoopCommBlock.disconnect_state != COOP_DISCONNECT_NONE || gCoopCommBlock.room_rejection != 0)
        return TRUE;
    if (gCoopCommBlock.gates.trade_pending == 1 && Coop_IsConnectedToRoom())
        return TRUE;
    return in != NULL && (in->room_state & COOP_ROOM_RUN_LOST);
}

// The map of the 32x32 block holding a tile, from the loaded matrix.
static BOOL Coop_TileZone(FieldSystem *fsys, s32 x, s32 z, u16 *map)
{
    const u8 *matrix = fsys->map_matrix;
    u32 width, height, index;

    if (matrix == NULL || x < 0 || z < 0)
        return FALSE;
    width = matrix[COOP_MATRIX_WIDTH];
    height = matrix[COOP_MATRIX_HEIGHT];
    if ((u32)x / COOP_BLOCK_TILES >= width || (u32)z / COOP_BLOCK_TILES >= height)
        return FALSE;
    index = ((u32)z / COOP_BLOCK_TILES) * width + (u32)x / COOP_BLOCK_TILES;
    __builtin_memcpy(map, matrix + COOP_MATRIX_HEADERS + index * sizeof(u16), sizeof(u16));
    return TRUE;
}

// A spirit may step onto a tile that vanilla's follower could step onto, in
// the same map zone, with no warp on it (Q2: no warps, no zone changes).
static BOOL Coop_SpiritCanStep(FieldSystem *fsys, LocalMapObject *player, s32 x, s32 z)
{
    MAP_EVENTS *events = fsys->map_events;
    VecFx32 pos;
    u16 zone;
    u32 i;

    if (!Coop_TileZone(fsys, x, z, &zone) || zone != (u16)fsys->location->mapId)
        return FALSE;
    MapObject_CopyPositionVector(player, &pos);
    if (sub_020549A8(fsys, &pos, x, z, 0) || MetatileBehavior_IsSurfableWater(GetMetatileBehaviorAt(fsys, x, z))
        || sub_02060BFC(player, x, player->hCurr, z))
        return FALSE;
    if (events != NULL && events->warp_events != NULL)
        for (i = 0; i < events->num_warp_events; i++)
            if (events->warp_events[i].x == x && events->warp_events[i].y == z)
                return FALSE;
    return TRUE;
}

static void Coop_SpiritMove(struct CoopSpiritEnv *env, LocalMapObject *player, u32 command)
{
    env->moves[0].command = (u16)command;
    env->moves[0].length = 1;
    env->moves[1].command = COOP_MOVEMENT_STEP_END;
    env->moves[1].length = 0;
    env->movement = EventObjectMovementMan_Create(player, env->moves);
}

// The D-pad: a step, or a turn toward a tile the spirit can't enter.
static void Coop_SpiritWalk(FieldSystem *fsys, struct CoopSpiritEnv *env, LocalMapObject *player)
{
    static const s8 sDx[4] = { 0, 0, -1, 1 };
    static const s8 sDz[4] = { -1, 1, 0, 0 };
    u32 held = gSystem.heldKeys, dir;

    if (held & PAD_KEY_UP)
        dir = COOP_DIR_UP;
    else if (held & PAD_KEY_DOWN)
        dir = COOP_DIR_DOWN;
    else if (held & PAD_KEY_LEFT)
        dir = COOP_DIR_LEFT;
    else if (held & PAD_KEY_RIGHT)
        dir = COOP_DIR_RIGHT;
    else
        return;
    if (Coop_SpiritCanStep(fsys, player, (s32)player->xCurr + sDx[dir], (s32)player->yCurr + sDz[dir]))
        Coop_SpiritMove(env, player, COOP_MOVEMENT_STEP_UP + dir);
    else if ((u32)player->curFacing != dir)
        Coop_SpiritMove(env, player, COOP_MOVEMENT_FACE_UP + dir);
}

// One HG glyph string (0xFFFF-terminated) split in two 11-glyph halves.
static void Coop_SplitName(const u16 *src, u32 max, u16 *a, u16 *b)
{
    u32 i, n = 0;

    for (i = 0; i < COOP_NAME_GLYPHS - 1 && n < max && src[n] != 0xFFFF; i++, n++)
        a[i] = src[n];
    a[i] = 0xFFFF;
    for (i = 0; i < COOP_NAME_GLYPHS - 1 && n < max && src[n] != 0xFFFF; i++, n++)
        b[i] = src[n];
    b[i] = 0xFFFF;
}

// The next teammate's page: "NAME is at PLACE." and what they are doing.
static BOOL Coop_SpiritTeamPage(struct CoopSpiritEnv *env)
{
    struct CoopPlayerState remote;
    String *place;
    u32 i;

    for (; env->teamSlot < COOP_REMOTE_SLOTS; env->teamSlot++)
    {
        if (!Coop_ReadRemotePlayer(env->teamSlot, &remote) || remote.trainer_id == 0)
            continue;
        env->teamSlot++;
        for (i = 0; i < 8 && remote.name[i] != 0xFFFF; i++)
            env->name[i] = remote.name[i];
        env->name[i] = 0xFFFF;
        env->place1[0] = env->place2[0] = 0xFFFF;
        place = String_New(24, COOP_HEAP_FIELD);
        if (place != NULL)
        {
            MapID_GetLandmarkName(remote.map_id, COOP_HEAP_FIELD, place);
            Coop_SplitName(place->data, place->size < place->maxsize ? place->size : place->maxsize, env->place1, env->place2);
            String_Delete(place);
        }
        if (remote.challenge_flags & COOP_PLAYER_FLAG_SPIRIT)
            Coop_DialogPrint(&env->dialog, "@'s spirit lingers\nat $&.");
        else if (!remote.is_active && remote.activity == COOP_ACTIVITY_BATTLE)
            Coop_DialogPrint(&env->dialog, "@ is in a battle\nnear $&.");
        else if (!remote.is_active)
            Coop_DialogPrint(&env->dialog, "@ is busy\nnear $&.");
        else
            Coop_DialogPrint(&env->dialog, "@ is exploring\n$&.");
        return TRUE;
    }
    return FALSE;
}

static BOOL Coop_Task_Spirit(TaskManager *taskman)
{
    FieldSystem *fsys = taskman->fieldSystem;
    struct CoopSpiritEnv *env = taskman->env;
    struct CoopChallengeSave *save = Coop_ChallengeSave();
    LocalMapObject *player = Coop_Player(fsys);
    const char *labels[3];
    u8 values[3];
    u32 value;

    // A step finishes before anything else happens.
    if (env->movement != NULL)
    {
        if (!EventObjectMovementMan_IsFinish(env->movement))
            return FALSE;
        EventObjectMovementMan_Delete(env->movement);
        env->movement = NULL;
    }
    if (player == NULL)
        return FALSE;
    // The menu and its pages give way too (a revive left waiting would time
    // out at the relay); the intro finishes first, it saves.
    if (taskman->state >= COOP_SPIRIT_MENU && taskman->state != COOP_SPIRIT_SAVE && Coop_SpiritMustYield()
        && !Coop_DialogIsPrinting(&env->dialog))
    {
        Coop_SpiritCloseDialog(fsys, env);
        sys_FreeMemoryEz(env);
        return TRUE;
    }

    switch (taskman->state)
    {
    case COOP_SPIRIT_WAIT_STILL:
        if (MapObject_IsSingleMovementActive(player))
            break;
        if (save != NULL && (save->spirit_flags & COOP_SPIRIT_INTRO))
        {
            Coop_PlayerName(fsys, env->name);
            Coop_SpiritOpenDialog(fsys, env);
            Coop_DialogPrint(&env->dialog, "@'s spirit lingers here.\nTeammates can revive @.");
            taskman->state = COOP_SPIRIT_INTRO_TEXT;
            break;
        }
        taskman->state = COOP_SPIRIT_ROAM;
        break;

    case COOP_SPIRIT_INTRO_TEXT:
        if (Coop_DialogIsPrinting(&env->dialog) || !Coop_DialogButtonPressed())
            break;
        Coop_DialogPrint(&env->dialog, "Walk with the + Control Pad.\nSTART opens the spirit menu.");
        taskman->state = COOP_SPIRIT_INTRO_PAGE2;
        break;

    case COOP_SPIRIT_INTRO_PAGE2:
        if (Coop_DialogIsPrinting(&env->dialog) || !Coop_DialogButtonPressed())
            break;
        env->frames = 0;
        taskman->state = COOP_SPIRIT_INTRO_SAVE;
        break;

    case COOP_SPIRIT_INTRO_SAVE:
        // Saved at once: a reset can't take the wipe back.
        Coop_DialogShowSpinner(&env->dialog);
        if (++env->frames < 2)
            break;
        if (save != NULL)
            save->spirit_flags &= ~COOP_SPIRIT_INTRO;
        (void)Field_SaveGameNormal(fsys);
        // The hide restores the frame on the next VBlank, which reads the
        // window (coop_dialog.h): close it only after that.
        Coop_DialogHideSpinner(&env->dialog);
        env->frames = 0;
        taskman->state = COOP_SPIRIT_INTRO_CLOSE;
        break;

    case COOP_SPIRIT_INTRO_CLOSE:
        if (++env->frames < 3)
            break;
        Coop_SpiritCloseDialog(fsys, env);
        taskman->state = COOP_SPIRIT_ROAM;
        break;

    case COOP_SPIRIT_ROAM:
        if (Coop_SpiritMustYield())
        {
            Coop_SpiritCloseDialog(fsys, env);
            sys_FreeMemoryEz(env);
            return TRUE;
        }
        if (gSystem.newKeys & (PAD_BUTTON_START | PAD_BUTTON_SELECT))
        {
            PlaySE(COOP_SE_SELECT);
            labels[0] = "TEAM";
            values[0] = COOP_SPIRIT_MENU_TEAM;
            labels[1] = "SAVE";
            values[1] = COOP_SPIRIT_MENU_SAVE;
            labels[2] = "CANCEL";
            values[2] = COOP_SPIRIT_MENU_CANCEL;
            Coop_ChoiceOpen(fsys, &env->choice, labels, values, 3, 8);
            taskman->state = COOP_SPIRIT_MENU;
            break;
        }
        Coop_SpiritWalk(fsys, env, player);
        break;

    case COOP_SPIRIT_MENU:
        value = Coop_ChoiceInput(&env->choice);
        if (value == COOP_CHOICE_NONE)
        {
            if (gSystem.newKeys & (PAD_BUTTON_START | PAD_BUTTON_SELECT))
            {
                Coop_ChoiceClose(&env->choice);
                taskman->state = COOP_SPIRIT_ROAM;
            }
            break;
        }
        Coop_ChoiceClose(&env->choice);
        if (value == COOP_SPIRIT_MENU_TEAM)
        {
            Coop_SpiritOpenDialog(fsys, env);
            env->teamSlot = 0;
            if (Coop_SpiritTeamPage(env))
                taskman->state = COOP_SPIRIT_TEAM_PAGE;
            else
            {
                Coop_DialogPrint(&env->dialog, "None of your teammates\nare online.");
                taskman->state = COOP_SPIRIT_CLOSE_TEXT;
            }
        }
        else if (value == COOP_SPIRIT_MENU_SAVE)
        {
            Coop_SpiritOpenDialog(fsys, env);
            Coop_DialogPrint(&env->dialog, "Saving. Don't turn off\nthe power.");
            taskman->state = COOP_SPIRIT_SAVE_TEXT;
        }
        else
            taskman->state = COOP_SPIRIT_ROAM;
        break;

    case COOP_SPIRIT_TEAM_PAGE:
        if (Coop_DialogIsPrinting(&env->dialog) || !Coop_DialogButtonPressed())
            break;
        if (!Coop_SpiritTeamPage(env))
        {
            Coop_SpiritCloseDialog(fsys, env);
            taskman->state = COOP_SPIRIT_ROAM;
        }
        break;

    case COOP_SPIRIT_SAVE_TEXT:
        if (Coop_DialogIsPrinting(&env->dialog))
            break;
        env->frames = 0;
        taskman->state = COOP_SPIRIT_SAVE;
        break;

    case COOP_SPIRIT_SAVE:
        Coop_DialogShowSpinner(&env->dialog);
        if (++env->frames < 2)
            break;
        value = Field_SaveGameNormal(fsys);
        Coop_DialogHideSpinner(&env->dialog);
        Coop_DialogPrint(&env->dialog, value ? "Your adventure was saved." : "The save failed.");
        taskman->state = COOP_SPIRIT_CLOSE_TEXT;
        break;

    case COOP_SPIRIT_CLOSE_TEXT:
        if (Coop_DialogIsPrinting(&env->dialog) || !Coop_DialogButtonPressed())
            break;
        Coop_SpiritCloseDialog(fsys, env);
        taskman->state = COOP_SPIRIT_ROAM;
        break;
    }
    return FALSE;
}

BOOL Coop_SpiritIsRoaming(FieldSystem *fsys)
{
    const TaskManager *taskman = fsys->taskman;

    return taskman != NULL && taskman->func == Coop_Task_Spirit && taskman->state == COOP_SPIRIT_ROAM;
}

// ---------------------------------------------------------------------------
// The run is lost (§5.5.5)
// ---------------------------------------------------------------------------

enum CoopRunLostState
{
    COOP_RUN_LOST_PAGE1,
    COOP_RUN_LOST_PAGE2,
    COOP_RUN_LOST_PAGE3,
    COOP_RUN_LOST_SAVE,
    COOP_RUN_LOST_RESULT,
    COOP_RUN_LOST_RESET,
};

struct CoopRunLostEnv
{
    struct CoopDialog dialog;
    u16 teamDeaths[6];
    u16 badges[4];
    u16 deaths[6];
    u8 frames;
};

static void Coop_NumberGlyphs(u32 value, u16 *out)
{
    u16 digits[5];
    u32 n = 0, i;

    do
    {
        digits[n++] = (u16)(0x0121 + value % 10); // charmap '0'..'9'
        value /= 10;
    } while (value != 0 && n < 5);
    for (i = 0; i < n; i++)
        out[i] = digits[n - 1 - i];
    out[n] = 0xFFFF;
}

// Never returns TRUE: the task ends in OS_ResetSystem.
static BOOL Coop_Task_RunLost(TaskManager *taskman)
{
    FieldSystem *fsys = taskman->fieldSystem;
    struct CoopRunLostEnv *env = taskman->env;
    struct CoopChallengeSave *save = Coop_ChallengeSave();

    switch (taskman->state)
    {
    case COOP_RUN_LOST_PAGE1:
        if (Coop_DialogIsPrinting(&env->dialog) || !Coop_DialogButtonPressed())
            break;
        Coop_DialogPrint(&env->dialog, "Your team lost @ Pokémon.\nYour badges: $.");
        taskman->state = COOP_RUN_LOST_PAGE2;
        break;

    case COOP_RUN_LOST_PAGE2:
        if (Coop_DialogIsPrinting(&env->dialog) || !Coop_DialogButtonPressed())
            break;
        Coop_DialogPrint(&env->dialog, "Saving the end of the run.\nDon't turn off the power.");
        taskman->state = COOP_RUN_LOST_PAGE3;
        break;

    case COOP_RUN_LOST_PAGE3:
        if (Coop_DialogIsPrinting(&env->dialog))
            break;
        env->frames = 0;
        taskman->state = COOP_RUN_LOST_SAVE;
        break;

    case COOP_RUN_LOST_SAVE:
        Coop_DialogShowSpinner(&env->dialog);
        if (++env->frames < 2)
            break;
        // The relay turns this game away once its report says so: save first.
        if (save != NULL)
            save->state |= COOP_CHALLENGE_RUN_LOST;
        (void)Field_SaveGameNormal(fsys);
        Coop_DialogHideSpinner(&env->dialog);
        Coop_DialogPrint(&env->dialog, "Thank you for playing!\nReturning to the Title Screen...");
        taskman->state = COOP_RUN_LOST_RESULT;
        break;

    case COOP_RUN_LOST_RESULT:
        if (!Coop_DialogIsPrinting(&env->dialog))
            taskman->state = COOP_RUN_LOST_RESET;
        break;

    case COOP_RUN_LOST_RESET:
        if (Coop_DialogButtonPressed())
            OS_ResetSystem(0);
        break;
    }
    return FALSE;
}

static BOOL Coop_StartRunLost(FieldSystem *fsys, const struct CoopChallengeDown *in)
{
    struct PlayerProfile *profile = Sav2_PlayerData_GetProfileAddr(fsys->savedata);
    struct CoopRunLostEnv *env;
    u32 badges = 0, bits;

    if (!Coop_StartFieldTask(fsys, Coop_Task_RunLost, sizeof(struct CoopRunLostEnv)))
        return FALSE;
    env = ((TaskManager *)fsys->taskman)->env;
    if (profile != NULL)
        for (bits = profile->johtoBadges | ((u32)profile->kantoBadges << 8); bits != 0; bits &= bits - 1)
            badges++;
    Coop_NumberGlyphs(in->team_deaths, env->teamDeaths);
    Coop_NumberGlyphs(badges, env->badges);
    gCoopCommBlock.disconnect_state = COOP_DISCONNECT_RUNNING; // no disconnect save over this one
    MapObjectMan_PauseAllMovement(fsys->mapObjectMan);
    Coop_DialogOpen(fsys, &env->dialog);
    env->dialog.args[0] = env->teamDeaths;
    env->dialog.args[1] = env->badges;
    Coop_DialogPrint(&env->dialog, "Every member of the team\nhas fallen. The run is over.");
    return TRUE;
}

// ---------------------------------------------------------------------------
// The field frame
// ---------------------------------------------------------------------------

static void Coop_SpiritWarp(FieldSystem *fsys, u32 map, s32 x, s32 z)
{
    LocalMapObject *player = Coop_Player(fsys);

    if (map == (u32)fsys->location->mapId && player != NULL && player->xCurr == x && player->yCurr == z)
        return; // already there
    // Fades out, unloads the field (and this overlay) and loads the map.
    sub_020538C0(fsys, map, -1, x, z, COOP_DIR_DOWN);
}

void Coop_SpiritFieldFrame(FieldSystem *fsys, struct CoopChallengeSave *save)
{
    const struct CoopChallengeDown *in = Coop_ChallengeIn();
    struct Party *party = SaveArray_Party_Get(fsys->savedata);

    if (!Coop_IsSafeOverworldWindow(fsys) || gCoopCommBlock.disconnect_state != COOP_DISCONNECT_NONE
        || gCoopCommBlock.room_rejection != 0)
        return;
    if (in != NULL && (in->room_state & COOP_ROOM_RUN_LOST) && Coop_IsAdventureStarted())
    {
        (void)Coop_StartRunLost(fsys, in);
        return;
    }
    if (save->state & COOP_CHALLENGE_RETURN_WARP)
    {
        save->state &= ~COOP_CHALLENGE_RETURN_WARP;
        if (save->return_map != 0)
            Coop_SpiritWarp(fsys, save->return_map, save->return_x, save->return_z);
        return;
    }
    if (!(save->state & COOP_CHALLENGE_SPIRIT))
        return;
    // Re-derived (§4.3): a spirit with a living Pokemon (a restored backup) isn't one.
    if (Coop_PartyLiving(party) != 0)
    {
        save->state &= ~(COOP_CHALLENGE_SPIRIT | COOP_CHALLENGE_ANCHOR_WARP);
        save->spirit_flags = 0;
        return;
    }
    if (save->state & COOP_CHALLENGE_ANCHOR_WARP)
    {
        save->state &= ~COOP_CHALLENGE_ANCHOR_WARP;
        if (save->anchor_map != 0)
            Coop_SpiritWarp(fsys, save->anchor_map, save->anchor_x, save->anchor_z);
        return;
    }
    if (Coop_SpiritMustYield())
        return; // the trade task takes a revive
    (void)Coop_StartFieldTask(fsys, Coop_Task_Spirit, sizeof(struct CoopSpiritEnv));
}
