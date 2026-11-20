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
// §5.5), reworked by UC15 (dev/docs/unified-client/UC15_ROAMING_SPIRITS.md):
// the spirit roams freely and triggers nothing. Overlay 131. "Spirit" in code,
// "Ghost" on screen: "ghost" already means a remote player's map object.
//
//   the sweep finds no living Pokemon (coop_challenge_nuzlocke.c)
//     -> Coop_SpiritEnter: "NAME has no Pokemon left...", the white-out spot
//        (the last Pokemon Center) becomes return_*, the fly points are kept
//     -> the spirit task, started as soon as nothing else runs (it owns the
//        input: scenes, trainers, the start menu never get a frame): one tile
//        per step across zones, doors, ladders, stairs and mats as vanilla;
//        X: the spirit menu (go to a teammate, save)
//     -> a teammate gives a Pokemon (coop_session.c, coop_trade.c: a
//        one-sided trade) -> Coop_SpiritRevived: the fly points back, warp to
//        return_*
//   every roster player a spirit -> the relay's "run lost" -> the summary,
//   a save, the title screen.
// ---------------------------------------------------------------------------

struct Party *LONG_CALL SaveArray_Party_Get(void *saveData);
BOOL LONG_CALL Field_SaveGameNormal(FieldSystem *fsys);
// The script warp (rom.ld): a fade, no Fly landing. From inside a task it becomes the task
// (TaskManager_Jump, as sub_02053908 does for the Fly warp); from outside, a new task.
BOOL LONG_CALL Task_ScriptWarp(TaskManager *taskman);
void LONG_CALL TaskManager_Jump(TaskManager *taskman, BOOL (*func)(TaskManager *), void *env);
// pret (rom.ld, docs/MILESTONE5_PLAN.md §11.1): vanilla's "can the follower
// step there" test in ScrCmd_FollowMonFacePlayer.
BOOL LONG_CALL sub_020549A8(FieldSystem *fsys, VecFx32 *pos, int x, int z, int unused); // TRUE: blocked
BOOL LONG_CALL sub_02060BFC(LocalMapObject *object, int x, int height, int z);          // TRUE: an object there
void LONG_CALL MapObject_CopyPositionVector(LocalMapObject *object, VecFx32 *out);
BOOL LONG_CALL MetatileBehavior_IsSurfableWater(u8 behavior);
void LONG_CALL MapID_GetLandmarkName(u32 mapId, u32 heapId, String *dest);

// pret include/unk_02062108.h: one movement action on the object, as the player's own steps
// are set (a movement script's runner left 2 idle frames after each step: the spirit stopped
// at every tile, laggy to play and stop-and-go on teammates' screens; the user, 2026-11-19).
// ClearHeldMovementIfActive: TRUE once the last one is done (or there's none).
void LONG_CALL MapObject_SetHeldMovement(LocalMapObject *object, u32 action);
BOOL LONG_CALL MapObject_ClearHeldMovementIfActive(LocalMapObject *object);

#define COOP_HEAP_FIELD          11
#define COOP_DIR_UP              0 // pret DIR_NORTH.. (constants/global_fieldmap.h)
#define COOP_DIR_DOWN            1
#define COOP_DIR_LEFT            2
#define COOP_DIR_RIGHT           3
#define COOP_MOVEMENT_STEP_UP    12 // + direction
#define COOP_MOVEMENT_JUMP_FAR_UP 56 // + direction: two tiles, a ledge's jump (pret asm/macros/movement.inc JumpFar*)
#define COOP_MOVEMENT_TURN_UP    40 // + direction: vanilla's turn, a quick step on the spot (pret asm/macros/movement.inc WalkOnSpotFasterNorth)
#define COOP_PLAYER_STATE_SURFING 2 // pret PLAYER_STATE_SURFING
#define COOP_SE_SELECT           1500
#define COOP_NAME_GLYPHS         12

// The fly points (pret constants/flags.h FLAG_SYS_FLYPOINT_PALLET + 0..37):
// entering a town sets its own (sub_02053038 -> FlypointFlagAction), so a
// spirit's are kept when it falls and put back on revival.
#define COOP_FLAG_FLYPOINT_FIRST 0x9B0
#define COOP_FLYPOINTS           38

// pret constants/metatile_behavior.h (enum TILE_BEHAVIOR) for the warps of
// pret src/field/field_control.c: FieldSystem_CheckMapTransition (pushed:
// ladders, doors, side and south exits, stairs) and FieldSystem_CheckTransition
// (stepped onto: north entrances, warp panels, a ladder down, escalators).
#define COOP_TILE_LADDER_NORTH        0x3C
#define COOP_TILE_LADDER_SOUTH        0x3D
#define COOP_TILE_LADDER_DOWN         0x3E
#define COOP_TILE_WARP_STAIRS_EAST    0x5E
#define COOP_TILE_WARP_STAIRS_WEST    0x5F
#define COOP_TILE_WARP_ENTRANCE_EAST  0x62
#define COOP_TILE_WARP_ENTRANCE_WEST  0x63
#define COOP_TILE_WARP_ENTRANCE_NORTH 0x64
#define COOP_TILE_WARP_ENTRANCE_SOUTH 0x65
#define COOP_TILE_WARP_PANEL          0x67
#define COOP_TILE_DOOR                0x69
#define COOP_TILE_ESCALATOR_FLIP_FACE 0x6A
#define COOP_TILE_ESCALATOR           0x6B
#define COOP_TILE_WARP_EAST           0x6C
#define COOP_TILE_WARP_WEST           0x6D
#define COOP_TILE_WARP_NORTH          0x6E
#define COOP_TILE_WARP_SOUTH          0x6F
#define COOP_TILE_JUMP_EAST           0x38 // ledges, jumped in their own direction (pret sub_0205DB68)
#define COOP_TILE_JUMP_WEST           0x39
#define COOP_TILE_JUMP_NORTH          0x3A
#define COOP_TILE_JUMP_SOUTH          0x3B
#define COOP_WARP_DYNAMIC_HEADER      0xFFF // pret FieldSystem_MapConnection: "back where you came from"

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

// pret include/field_system.h FieldInput, as FieldInput_Process reads it.
struct CoopFieldInput
{
    u16 flags; // interact, endMovement, menu, registeredItem (2), sign, mapTransition, movement, unk0_8 (the
               // phone), unk0_9, unk0_A..C, unk0_D (bit 13: no scenes, no trainers' eyes)
    u16 unk2;
    u8 playerDir;
    s8 transitionDir;
    u16 newKeys;
    u16 heldKeys;
    u16 unkA;
};
_Static_assert(sizeof(struct CoopFieldInput) == 0x0C, "pret FieldInput");
#define COOP_FIELD_INPUT_NO_SCENES 0x2000 // unk0_D
#define COOP_DIR_NONE              (-1)   // pret DIR_NONE

// UC15 (hooks: FieldInput_Process, asm/field/field_hooks.s): vanilla's field
// input runs only in frames the spirit task doesn't hold (right after a load,
// before the bridge connects, a revive handing over). For a spirit it reads as
// nothing pressed and no scene to check: no step, talk, menu, sign, phone
// call, trainer's eyes or scene. Anyone else's input is untouched.
void Coop_SpiritFilterInput(struct CoopFieldInput *input, FieldSystem *fsys)
{
    (void)fsys;
    if (!Coop_IsSpirit())
        return;
    input->flags = COOP_FIELD_INPUT_NO_SCENES;
    input->transitionDir = COOP_DIR_NONE;
    input->newKeys = 0;
    input->heldKeys = 0;
}

// ---------------------------------------------------------------------------
// The fly points (UC15)
// ---------------------------------------------------------------------------

static void Coop_SpiritKeepFly(struct CoopChallengeSave *save)
{
    u32 i;

    for (i = 0; i < sizeof(save->fly_saved); i++)
        save->fly_saved[i] = 0;
    for (i = 0; i < COOP_FLYPOINTS; i++)
        if (CheckScriptFlag((u16)(COOP_FLAG_FLYPOINT_FIRST + i)))
            save->fly_saved[i / 8] |= (u8)(1 << (i % 8));
    save->spirit_flags |= COOP_SPIRIT_FLY_SAVED;
}

static void Coop_SpiritRestoreFly(struct CoopChallengeSave *save)
{
    u32 i;

    if (!(save->spirit_flags & COOP_SPIRIT_FLY_SAVED))
        return;
    for (i = 0; i < COOP_FLYPOINTS; i++)
    {
        if (save->fly_saved[i / 8] & (1 << (i % 8)))
            SetScriptFlag((u16)(COOP_FLAG_FLYPOINT_FIRST + i));
        else
            ClearScriptFlag((u16)(COOP_FLAG_FLYPOINT_FIRST + i));
    }
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
// Entering (§5.5.2, UC15)
// ---------------------------------------------------------------------------

void Coop_SpiritEnter(FieldSystem *fsys, struct CoopChallengeSave *save)
{
    LocalMapObject *player = Coop_Player(fsys);
    u16 name[COOP_NAME_GLYPHS];
    u32 i;

    if (player == NULL || (save->state & COOP_CHALLENGE_SPIRIT))
        return;
    // After a lost battle the blackout already brought the player to the
    // last Pokemon Center: the spirit starts there, and comes back there.
    save->return_map = (u16)fsys->location->mapId;
    save->return_x = (s16)player->xCurr;
    save->return_z = (s16)player->yCurr;
    for (i = 0; i < NELEMS(save->unused7C); i++)
        save->unused7C[i] = 0;
    Coop_SpiritKeepFly(save);
    save->state |= COOP_CHALLENGE_SPIRIT;
    save->state &= ~COOP_CHALLENGE_ANCHOR_WARP;
    save->spirit_flags |= COOP_SPIRIT_INTRO;
    Coop_PlayerName(fsys, name);
    Coop_StartSpiritMessage(fsys, name, "@ has no Pokémon left...", NULL);
}

void Coop_SpiritRevived(FieldSystem *fsys)
{
    struct CoopChallengeSave *save = Coop_ChallengeSave();

    (void)fsys;
    if (save == NULL)
        return;
    Coop_SpiritRestoreFly(save); // a spirit's visits unlock no fly points
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
        return "An Egg can't rescue\nanyone.";
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
// The spirit task (§5.5.3, UC15)
// ---------------------------------------------------------------------------

enum CoopSpiritTaskState
{
    COOP_SPIRIT_WAIT_STILL,
    COOP_SPIRIT_INTRO_TEXT,   // first time: two pages, then a save
    COOP_SPIRIT_INTRO_PAGE2,
    COOP_SPIRIT_INTRO_SAVE,
    COOP_SPIRIT_INTRO_CLOSE,  // the spinner's hide reaches the screen before the window goes
    COOP_SPIRIT_ROAM,
    COOP_SPIRIT_MENU,         // X: the teammates, SAVE, CANCEL
    COOP_SPIRIT_GO_TEXT,      // "Go to NAME in PLACE?"
    COOP_SPIRIT_GO_CHOICE,    // YES / NO
    COOP_SPIRIT_WARP,         // a warp waits for the map name banner to go
    COOP_SPIRIT_SAVE_TEXT,
    COOP_SPIRIT_SAVE,
    COOP_SPIRIT_CLOSE_TEXT,   // a last page, then back to roaming
};

#define COOP_SPIRIT_MENU_SAVE   0x10 // teammates are their remote slot + 1
#define COOP_SPIRIT_MENU_CANCEL 0x11
#define COOP_SPIRIT_MENU_WIDTH  9    // tiles: a seven-glyph name

struct CoopSpiritEnv
{
    struct CoopDialog dialog;
    struct CoopChoice choice;
    u16 name[COOP_NAME_GLYPHS];
    u16 place1[COOP_NAME_GLYPHS]; // a landmark name, split for the 11-glyph arguments
    u16 place2[COOP_NAME_GLYPHS];
    s16 warpX, warpZ;             // COOP_SPIRIT_WARP: where to
    u16 warpMap;
    s16 warpId;                   // -1: (warpX, warpZ)
    u16 frames;
    u8 warpDir;
    u8 stepped;  // the last movement was a step (not a turn): check a warp underfoot
    u8 walking;  // the last movement was a step, and a direction is still held
    u8 menuAsked; // X pressed under the map name banner: the menu opens once it's gone
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

    if (save == NULL || !(save->state & COOP_CHALLENGE_SPIRIT))
        return TRUE;
    if (gCoopCommBlock.disconnect_state != COOP_DISCONNECT_NONE || gCoopCommBlock.room_rejection != 0)
        return TRUE;
    if (gCoopCommBlock.gates.trade_pending == 1 && Coop_IsConnectedToRoom())
        return TRUE;
    return in != NULL && (in->room_state & COOP_ROOM_RUN_LOST);
}

// The map's warp at a tile (not one leading "back where you came from": a
// spirit writes no entrance, UC15), else NULL.
static const WARP_EVENT *Coop_SpiritWarpAt(FieldSystem *fsys, s32 x, s32 z)
{
    MAP_EVENTS *events = fsys->map_events;
    u32 i;

    if (events == NULL || events->warp_events == NULL)
        return NULL;
    for (i = 0; i < events->num_warp_events; i++)
    {
        const WARP_EVENT *warp = &events->warp_events[i];
        if (warp->x == x && warp->y == z)
            return warp->header == COOP_WARP_DYNAMIC_HEADER ? NULL : warp;
    }
    return NULL;
}

// The direction a tile's warp is taken by pushing (vanilla's
// FieldSystem_CheckMapTransition), 0xFF if none.
static u32 Coop_PushWarpDir(u8 behavior)
{
    switch (behavior)
    {
    case COOP_TILE_LADDER_NORTH:
        return COOP_DIR_UP;
    case COOP_TILE_LADDER_SOUTH:
    case COOP_TILE_WARP_ENTRANCE_SOUTH:
    case COOP_TILE_WARP_SOUTH:
        return COOP_DIR_DOWN;
    case COOP_TILE_WARP_STAIRS_EAST:
    case COOP_TILE_WARP_ENTRANCE_EAST:
    case COOP_TILE_WARP_EAST:
        return COOP_DIR_RIGHT;
    case COOP_TILE_WARP_STAIRS_WEST:
    case COOP_TILE_WARP_ENTRANCE_WEST:
    case COOP_TILE_WARP_WEST:
        return COOP_DIR_LEFT;
    }
    return 0xFF;
}

// A tile whose warp is taken by stepping onto it (FieldSystem_CheckTransition).
static BOOL Coop_StepWarp(u8 behavior)
{
    return behavior == COOP_TILE_WARP_ENTRANCE_NORTH || behavior == COOP_TILE_WARP_NORTH
        || behavior == COOP_TILE_WARP_PANEL || behavior == COOP_TILE_LADDER_DOWN
        || behavior == COOP_TILE_ESCALATOR || behavior == COOP_TILE_ESCALATOR_FLIP_FACE;
}

static void Coop_SpiritAimWarp(struct CoopSpiritEnv *env, u32 map, s32 warpId, s32 x, s32 z, u32 dir)
{
    env->warpMap = (u16)map;
    env->warpId = (s16)warpId;
    env->warpX = (s16)x;
    env->warpZ = (s16)z;
    env->warpDir = (u8)dir;
}

// pret overlay_01.h ErrorContinueEnv: Task_ScriptWarp's state and its Location.
struct CoopScriptWarpEnv
{
    int state;
    u32 mapId;
    int warpId;
    int x;
    int z;
    int direction;
};

// The script warp's env (FIELD2, as vanilla's warps: it outlives the map; Task_ScriptWarp
// frees it).
static struct CoopScriptWarpEnv *Coop_ScriptWarpEnv(u32 map, s32 warpId, s32 x, s32 z, s32 dir)
{
    struct CoopScriptWarpEnv *warp = sys_AllocMemoryLo(COOP_HEAP_FIELD, sizeof(*warp));

    if (warp != NULL)
    {
        warp->state = 0;
        warp->mapId = map;
        warp->warpId = warpId;
        warp->x = x;
        warp->z = z;
        warp->direction = dir;
    }
    return warp;
}

// pret include/screen_fade.h: a script's FadeScreen 6, 1, 0 / 1, RGB_BLACK (out / in, both screens).
void LONG_CALL BeginNormalPaletteFade(int mode, int typeMain, int typeSub, u16 color, int steps, int framesPerStep, int heapId);
BOOL LONG_CALL IsPaletteFadeFinished(void);
#define COOP_HEAP_FIELD1 4 // pret HEAP_ID_FIELD1
extern u32 gCoopLocalLastSprite; // coop_ghosts.c: 0xFFFFFFFF until the field runs on a new map
static u8 sCoopSpiritArrival __attribute__((section(".data"))) = 0;

// A spirit's warp: the script warp between a script's fades, out and in (the spirit had none:
// the 3D light went off and the next map showed half loaded, the user's play test, 2026-11-19;
// the step warp's own fades, ov01_021F35C4, are the warp panel's, the player dropping in).
// Its env is the script warp's, which frees it.
static BOOL Coop_Task_SpiritWarp(TaskManager *taskman)
{
    u32 state = taskman->state;

    if (state & 1) // 1, 3: a fade runs
    {
        if (!IsPaletteFadeFinished())
            return FALSE;
        if (state == 3)
            return TRUE;
        TaskManager_Call(taskman, Task_ScriptWarp, taskman->env); // it frees its env
    }
    // In once the field has run on the new map and the avatar had time to become ours (Ethan or
    // Lyra showed through the fade-in, the user's play test, 2026-11-19). The count starts at 0
    // with each map (Overlay 131 is loaded again).
    else if (state == 2 && (gCoopLocalLastSprite == 0xFFFFFFFF || ++sCoopSpiritArrival < 8))
        return FALSE;
    else // 0: out, 2: in
        BeginNormalPaletteFade(0, state >> 1, state >> 1, 0, 6, 1, COOP_HEAP_FIELD1);
    taskman->state = state + 1;
    return FALSE;
}

// A spirit may step onto a tile that vanilla's follower could step onto: no
// water, no object (UC15: other zones and warp tiles too).
static BOOL Coop_SpiritCanStep(FieldSystem *fsys, LocalMapObject *player, s32 x, s32 z)
{
    VecFx32 pos;

    if (x < 0 || z < 0)
        return FALSE;
    MapObject_CopyPositionVector(player, &pos);
    return !sub_020549A8(fsys, &pos, x, z, 0) && !MetatileBehavior_IsSurfableWater(GetMetatileBehaviorAt(fsys, x, z))
        && !sub_02060BFC(player, x, player->hCurr, z);
}


// The D-pad: a warp (a pushed one underfoot, a door ahead), a step, or a turn.
// TRUE: a warp is aimed.
static BOOL Coop_SpiritWalk(FieldSystem *fsys, struct CoopSpiritEnv *env, LocalMapObject *player)
{
    static const s8 sDx[4] = { 0, 0, -1, 1 };
    static const s8 sDz[4] = { -1, 1, 0, 0 };
    u32 held = gSystem.heldKeys, dir;
    s32 x = (s32)player->xCurr, z = (s32)player->yCurr, nx, nz;
    const WARP_EVENT *warp;

    if (held & PAD_KEY_UP)
        dir = COOP_DIR_UP;
    else if (held & PAD_KEY_DOWN)
        dir = COOP_DIR_DOWN;
    else if (held & PAD_KEY_LEFT)
        dir = COOP_DIR_LEFT;
    else if (held & PAD_KEY_RIGHT)
        dir = COOP_DIR_RIGHT;
    else
        return env->walking = FALSE;
    // From standing, a new direction turns first (vanilla's turn: a tap of up to 6 frames turns,
    // a longer press walks on); walking, it steps that way at once, as vanilla's player does
    // (a turn at each change of direction made the spirit feel laggy).
    if ((u32)player->curFacing != dir && !env->walking)
    {
        env->stepped = FALSE;
        MapObject_SetHeldMovement(player, COOP_MOVEMENT_TURN_UP + dir);
        return FALSE;
    }
    nx = x + sDx[dir];
    nz = z + sDz[dir];
    warp = Coop_SpiritWarpAt(fsys, x, z);
    if (warp != NULL && Coop_PushWarpDir(GetMetatileBehaviorAt(fsys, x, z)) == dir)
    {
        Coop_SpiritAimWarp(env, warp->header, warp->anchor, 0, 0, dir);
        return TRUE;
    }
    warp = Coop_SpiritWarpAt(fsys, nx, nz);
    if (warp != NULL && GetMetatileBehaviorAt(fsys, nx, nz) == COOP_TILE_DOOR)
    {
        Coop_SpiritAimWarp(env, warp->header, warp->anchor, 0, 0, dir);
        return TRUE;
    }
    env->stepped = env->walking = FALSE;
    {
        // A ledge ahead, jumped the way it faces: two tiles, onto a free one.
        static const u8 sLedge[4] = { COOP_TILE_JUMP_NORTH, COOP_TILE_JUMP_SOUTH, COOP_TILE_JUMP_WEST, COOP_TILE_JUMP_EAST };
        if (GetMetatileBehaviorAt(fsys, nx, nz) == sLedge[dir])
        {
            if (Coop_SpiritCanStep(fsys, player, nx + sDx[dir], nz + sDz[dir]))
            {
                MapObject_SetHeldMovement(player, COOP_MOVEMENT_JUMP_FAR_UP + dir);
                env->stepped = env->walking = TRUE;
            }
            return FALSE;
        }
    }
    // Spirits walk: no B, no running shoes' lock (the user's call: one run step at a time
    // stuttered, and the lock can't be turned off from the spirit's screens).
    if (Coop_SpiritCanStep(fsys, player, nx, nz))
    {
        MapObject_SetHeldMovement(player, COOP_MOVEMENT_STEP_UP + dir);
        env->stepped = env->walking = TRUE;
    }
    return FALSE;
}

// After a step: a warp taken by stepping onto its tile.
static BOOL Coop_SpiritSteppedWarp(FieldSystem *fsys, struct CoopSpiritEnv *env, LocalMapObject *player)
{
    s32 x = (s32)player->xCurr, z = (s32)player->yCurr;
    const WARP_EVENT *warp;

    if (!env->stepped)
        return FALSE;
    env->stepped = FALSE;
    warp = Coop_SpiritWarpAt(fsys, x, z);
    if (warp == NULL || !Coop_StepWarp(GetMetatileBehaviorAt(fsys, x, z)))
        return FALSE;
    Coop_SpiritAimWarp(env, warp->header, warp->anchor, 0, 0, (u32)player->curFacing);
    return TRUE;
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

static void Coop_CopyName(const u16 *src, u16 *dst)
{
    u32 i;

    for (i = 0; i < 8 && src[i] != 0xFFFF; i++)
        dst[i] = src[i];
    dst[i] = 0xFFFF;
}

// X: each online teammate, SAVE, CANCEL.
static void Coop_SpiritOpenMenu(FieldSystem *fsys, struct CoopSpiritEnv *env)
{
    struct CoopPlayerState remote;
    u16 names[COOP_REMOTE_SLOTS][COOP_NAME_GLYPHS];
    const u16 *args[1];
    u8 values[COOP_CHOICE_MAX];
    u32 slot, count = 0, i;

    for (slot = 0; slot < COOP_REMOTE_SLOTS && count < COOP_CHOICE_MAX - 2; slot++)
    {
        if (!Coop_ReadRemotePlayer(slot, &remote) || remote.trainer_id == 0)
            continue;
        Coop_CopyName(remote.name, names[count]);
        values[count++] = (u8)(slot + 1);
    }
    values[count] = COOP_SPIRIT_MENU_SAVE;
    values[count + 1] = COOP_SPIRIT_MENU_CANCEL;
    Coop_ChoiceOpenWindow(fsys, &env->choice, values, count + 2, COOP_SPIRIT_MENU_WIDTH);
    for (i = 0; i < count; i++)
    {
        args[0] = names[i];
        Coop_ChoicePrint(&env->choice, i, COOP_CHOICE_TEXT_X, "@", args);
    }
    Coop_ChoicePrint(&env->choice, count, COOP_CHOICE_TEXT_X, "SAVE", NULL);
    Coop_ChoicePrint(&env->choice, count + 1, COOP_CHOICE_TEXT_X, "CANCEL", NULL);
    Coop_ChoiceShow(&env->choice);
}

// A teammate picked in the menu: "Go to NAME in PLACE?" (TRUE: asked), or
// why not. The warp is aimed at their tile.
static BOOL Coop_SpiritAskGo(FieldSystem *fsys, struct CoopSpiritEnv *env, u32 slot)
{
    struct CoopPlayerState remote;
    String *place;

    Coop_SpiritOpenDialog(fsys, env);
    if (!Coop_ReadRemotePlayer(slot, &remote) || remote.trainer_id == 0)
    {
        Coop_DialogPrint(&env->dialog, "That teammate isn't online\nanymore.");
        return FALSE;
    }
    Coop_CopyName(remote.name, env->name);
    if (remote.map_id == 0)
    {
        Coop_DialogPrint(&env->dialog, "@ can't be reached\nright now.");
        return FALSE;
    }
    if (remote.avatar_state == COOP_PLAYER_STATE_SURFING)
    {
        Coop_DialogPrint(&env->dialog, "@ is out on the water.\nTry again later.");
        return FALSE;
    }
    env->place1[0] = env->place2[0] = 0xFFFF;
    place = String_New(24, COOP_HEAP_FIELD);
    if (place != NULL)
    {
        MapID_GetLandmarkName(remote.map_id, COOP_HEAP_FIELD, place);
        Coop_SplitName(place->data, place->size < place->maxsize ? place->size : place->maxsize, env->place1, env->place2);
        String_Delete(place);
    }
    Coop_SpiritAimWarp(env, remote.map_id, -1, remote.x, remote.z, COOP_DIR_DOWN);
    if (remote.challenge_flags & COOP_PLAYER_FLAG_SPIRIT)
        Coop_DialogPrint(&env->dialog, "Go to @'s spirit\nin $&?");
    else
        Coop_DialogPrint(&env->dialog, "Go to @\nin $&?");
    return TRUE;
}

static BOOL Coop_Task_Spirit(TaskManager *taskman)
{
    FieldSystem *fsys = taskman->fieldSystem;
    struct CoopSpiritEnv *env = taskman->env;
    struct CoopChallengeSave *save = Coop_ChallengeSave();
    LocalMapObject *player = Coop_Player(fsys);
    u32 map, value;
    s32 warpId, x, z, dir;

    // A step finishes before anything else happens (the next one starts on the frame it ends).
    if (player == NULL || !MapObject_ClearHeldMovementIfActive(player))
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
        // Out of a house onto its door (facing out): a step out, as vanilla's player
        // (on the door, walking back in needed a step away first; the user's play test).
        if (GetMetatileBehaviorAt(fsys, player->xCurr, player->yCurr) == COOP_TILE_DOOR)
        {
            MapObject_SetHeldMovement(player, COOP_MOVEMENT_STEP_UP + player->curFacing);
            break;
        }
        if (save != NULL && (save->spirit_flags & COOP_SPIRIT_INTRO))
        {
            // The task may start under the map name banner (UC15); our windows
            // wait for it (it scrolls BG3, where they draw: KNOWN_RISKS #14).
            // Walking needn't.
            if (Coop_IsMapNameBannerShowing(fsys))
                break;
            Coop_PlayerName(fsys, env->name);
            Coop_SpiritOpenDialog(fsys, env);
            Coop_DialogPrint(&env->dialog, "@'s spirit lingers here.\nTeammates can rescue @.");
            taskman->state = COOP_SPIRIT_INTRO_TEXT;
            break;
        }
        taskman->state = COOP_SPIRIT_ROAM;
        break;

    case COOP_SPIRIT_INTRO_TEXT:
        if (Coop_DialogIsPrinting(&env->dialog) || !Coop_DialogButtonPressed())
            break;
        Coop_DialogPrint(&env->dialog, "X opens the menu.");
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
        if (Coop_SpiritSteppedWarp(fsys, env, player))
        {
            taskman->state = COOP_SPIRIT_WARP;
            break;
        }
        if (gSystem.newKeys & PAD_BUTTON_X)
            env->menuAsked = TRUE;
        if (env->menuAsked && !Coop_IsMapNameBannerShowing(fsys))
        {
            env->menuAsked = FALSE;
            PlaySE(COOP_SE_SELECT);
            Coop_SpiritOpenMenu(fsys, env);
            taskman->state = COOP_SPIRIT_MENU;
            break;
        }
        if (Coop_SpiritWalk(fsys, env, player))
            taskman->state = COOP_SPIRIT_WARP;
        break;

    case COOP_SPIRIT_MENU:
        value = Coop_ChoiceInput(&env->choice);
        if (value == COOP_CHOICE_NONE)
        {
            if (gSystem.newKeys & PAD_BUTTON_X)
            {
                Coop_ChoiceClose(&env->choice);
                taskman->state = COOP_SPIRIT_ROAM;
            }
            break;
        }
        Coop_ChoiceClose(&env->choice);
        if (value == COOP_SPIRIT_MENU_SAVE)
        {
            Coop_SpiritOpenDialog(fsys, env);
            Coop_DialogPrint(&env->dialog, "Saving. Don't turn off\nthe power.");
            taskman->state = COOP_SPIRIT_SAVE_TEXT;
        }
        else if (value == COOP_SPIRIT_MENU_CANCEL)
            taskman->state = COOP_SPIRIT_ROAM;
        else
            taskman->state = Coop_SpiritAskGo(fsys, env, value - 1) ? COOP_SPIRIT_GO_TEXT : COOP_SPIRIT_CLOSE_TEXT;
        break;

    case COOP_SPIRIT_GO_TEXT:
        if (Coop_DialogIsPrinting(&env->dialog))
            break;
        Coop_ChoiceOpenYesNo(fsys, &env->choice);
        taskman->state = COOP_SPIRIT_GO_CHOICE;
        break;

    case COOP_SPIRIT_GO_CHOICE:
        value = Coop_ChoiceInput(&env->choice);
        if (value == COOP_CHOICE_NONE)
            break;
        Coop_SpiritCloseDialog(fsys, env);
        taskman->state = value == COOP_CHOICE_YES ? COOP_SPIRIT_WARP : COOP_SPIRIT_ROAM;
        break;

    case COOP_SPIRIT_WARP:
        // A warp under the map name banner crashed in window code
        // (KNOWN_RISKS #14): it waits. Then the task becomes the warp (as a
        // script's warp does); this task starts again on the other side.
        if (Coop_IsMapNameBannerShowing(fsys))
            break;
        map = env->warpMap;
        warpId = env->warpId;
        x = env->warpX;
        z = env->warpZ;
        dir = env->warpDir;
        Coop_SpiritCloseDialog(fsys, env);
        sys_FreeMemoryEz(env);
        PlaySE(COOP_SE_SELECT);
        // The script warp, not the Fly warp: a fade, no Fly landing (the user, 2026-11-18).
        {
            struct CoopScriptWarpEnv *warp = Coop_ScriptWarpEnv(map, warpId, x, z, dir);

            if (warp == NULL)
                return TRUE; // this task starts again where it is
            TaskManager_Jump(taskman, Coop_Task_SpiritWarp, warp);
        }
        return FALSE;

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
    // The spirit's warp (its fades, in once the field runs there; the bare script warp showed a
    // black screen and a strip of the Center on a rescue, the user's v0.7 play test): unloads the
    // field (and this overlay) and loads the map, no Fly landing.
    {
        struct CoopScriptWarpEnv *warp = Coop_ScriptWarpEnv(map, -1, x, z, COOP_DIR_DOWN);

        if (warp != NULL && Coop_FieldSystem_CreateTask(fsys, Coop_Task_SpiritWarp, warp) == NULL)
            sys_FreeMemoryEz(warp);
    }
}

BOOL Coop_SpiritKeepTask(FieldSystem *fsys, struct CoopChallengeSave *save)
{
    if (!(save->state & COOP_CHALLENGE_SPIRIT))
        return FALSE;
    // Saves from before UC15: no anchor any more, roam from here; the fly
    // points as they are now (the spirit couldn't leave its stretch of map).
    save->state &= ~COOP_CHALLENGE_ANCHOR_WARP;
    if (!(save->spirit_flags & COOP_SPIRIT_FLY_SAVED))
        Coop_SpiritKeepFly(save);
    if (fsys->taskman != NULL)
        return ((TaskManager *)fsys->taskman)->func == Coop_Task_Spirit;
    // Anything else goes through the sweep: a living Pokemon (a restored
    // backup), a revive, a disconnect, the run's end.
    if (fsys->textbox_open || Coop_PartyLiving(SaveArray_Party_Get(fsys->savedata)) != 0 || Coop_SpiritMustYield()
        || Coop_BoxLiving(fsys, save, 1) != 0)
        return FALSE;
    // At once, even under the map name banner: vanilla's controls (scenes,
    // trainers' eyes, the start menu, talking) never get a frame.
    return Coop_StartFieldTask(fsys, Coop_Task_Spirit, sizeof(struct CoopSpiritEnv));
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
        Coop_SpiritRestoreFly(save);
        save->state &= ~(COOP_CHALLENGE_SPIRIT | COOP_CHALLENGE_ANCHOR_WARP);
        save->spirit_flags = 0;
        return;
    }
    if (Coop_SpiritMustYield())
        return; // the trade task takes a revive
    (void)Coop_StartFieldTask(fsys, Coop_Task_Spirit, sizeof(struct CoopSpiritEnv));
}
