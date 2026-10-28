#include "config.h"
#include "types.h"
#include "constants/item.h"
#include "bag.h"

#include "constants/buttons.h"
#include "constants/maps.h"

#include "message.h"
#include "pokemon.h"
#include "save.h"
#include "map_events_internal.h"
#include "system.h"
#include "task.h"
#include "window.h"
#include "coop_sync.h"
#include "coop_dialog.h"
#include "coop_battle.h"
#include "coop_boss.h"

// ---------------------------------------------------------------------------
// Co-op field tasks and the native dialog box, Overlay 131.
//
// Moved out of the resident Overlay 129 (src/coop_sync.c), which needed the
// room for the virtual link (docs/MILESTONE3_PLAN.md §15): all of this only
// runs with the field map up. Coop_UpdateFieldConnection is called at the
// end of every field frame (Coop_FieldEntry, coop_ghosts.c) and starts the
// lobby and boss blocks, the adventure-start save and the disconnect save,
// as the co-op SysTask did before.
// ---------------------------------------------------------------------------

// Vanilla functions not declared (or declared without LONG_CALL) in any
// hg-engine header. Addresses live in rom.ld.
void *LONG_CALL Save_PlayerData_GetOptionsAddr(void *saveData);
u8 LONG_CALL TextPrinterCheckActive(u8 printerId);
void LONG_CALL AddWindowParameterized(void *bgConfig, void *window, u8 bgId, u8 x, u8 y, u8 width, u8 height, u8 paletteNum, u16 baseTile);
// Overlay 01: syncs map objects + player position into the save, then
// calls SaveGameNormal. Returns TRUE on success.
BOOL LONG_CALL Field_SaveGameNormal(FieldSystem *fsys);

// Heaps (pret include/constants/heap.h).
#define HEAP_ID_FIELD1  4
#define HEAP_ID_FIELD3  32 // field task heap (pret Task_New)

// Vanilla field dialog box (pret src/dialog_box.c).
#define DIALOG_BG_LAYER     3 // GF_BG_LYR_MAIN_3
#define DIALOG_X            2
#define DIALOG_Y            19
#define DIALOG_W            27
#define DIALOG_H            4
#define DIALOG_PLTT_TEXT    12
#define DIALOG_PLTT_FRAME   10
#define DIALOG_BASE_TILE    0x237
#define DIALOG_FRAME_TILE   0x3E2
#define DIALOG_FONT         1
#define DIALOG_BG_FILL      15

#define COOP_NAME_CHARS 8

// ---------------------------------------------------------------------------
// Overworld state
// ---------------------------------------------------------------------------

static LocalMapObject *Coop_GetPlayerMapObject(FieldSystem *fsys)
{
    if (fsys == NULL || fsys->playerAvatar == NULL)
        return NULL;
    return fsys->playerAvatar->mapObject;
}

// The field map overlay is running with no application (battle, bag,
// starter select, ...) on top of it. Mirrors pret sub_0203DF8C plus the
// child check. (Overlay 129 has its own copy for the co-op SysTask.)
static BOOL Coop_IsFieldMapActive(FieldSystem *fsys)
{
    FieldProcessManager *pm;

    if (fsys == NULL || fsys->savedata == NULL || fsys->location == NULL)
        return FALSE;
    pm = fsys->processManager;
    if (pm == NULL)
        return FALSE;
    return pm->parent != NULL && pm->child == NULL && !pm->isPaused && fsys->runningFieldMap;
}

// Safe to freeze, save and show a dialog: the vanilla
// FieldSystem_IsPlayerMovementAllowed() conditions (field map running, not
// paused, no field task - scripts, cutscenes, the start menu and warps all
// run as field tasks), no message box open, and the player is standing on
// a tile rather than mid-step.
// pret FieldSystem.mapSystems (+0x04, hg-engine unk4) -> drawMapNameInfo
// (+0x08) -> task (+0x08), the SysTask animating the location banner; NULL
// once the banner is gone (ccd draw_map_name.c FieldDrawMapName_ResetState).
#define COOP_FSYS_MAP_SYSTEMS_OFFSET     0x04
#define COOP_MAP_SYSTEMS_MAP_NAME_OFFSET 0x08
#define COOP_MAP_NAME_TASK_OFFSET        0x08

// Vanilla removes the location banner (FieldDrawMapName_Reset, unmapped)
// whenever a field event starts. Our field tasks cannot, so they wait for
// it: a warp under it crashed in window code, and it scrolls BG3, the layer
// our dialogs draw on (docs/KNOWN_RISKS.md #14).
BOOL LONG_CALL Coop_IsMapNameBannerShowing(FieldSystem *fsys)
{
    const u8 *mapSystems;
    const u8 *info;
    void *task;

    __builtin_memcpy(&mapSystems, (const u8 *)fsys + COOP_FSYS_MAP_SYSTEMS_OFFSET, sizeof(mapSystems));
    if (mapSystems == NULL)
        return FALSE;
    __builtin_memcpy(&info, mapSystems + COOP_MAP_SYSTEMS_MAP_NAME_OFFSET, sizeof(info));
    if (info == NULL)
        return FALSE;
    __builtin_memcpy(&task, info + COOP_MAP_NAME_TASK_OFFSET, sizeof(task));
    return task != NULL;
}

static BOOL Coop_IsSafeOverworldWindow(FieldSystem *fsys)
{
    LocalMapObject *player;

    if (!Coop_IsFieldMapActive(fsys))
        return FALSE;
    if (Coop_IsMapNameBannerShowing(fsys))
        return FALSE;
    if (fsys->taskman != NULL || fsys->textbox_open)
        return FALSE;
    player = Coop_GetPlayerMapObject(fsys);
    if (player == NULL || MapObject_IsSingleMovementActive(player))
        return FALSE;
    if (fsys->location->mapId == 0)
        return FALSE;
    return TRUE;
}

// ---------------------------------------------------------------------------
// Native overworld dialog box + field task helpers
// ---------------------------------------------------------------------------

// struct CoopDialog: include/coop_dialog.h (shared with Overlay 131).

static u8 Coop_GetTextFrameDelay(void *saveData)
{
    // pret Options_GetTextFrameDelay: Options.textSpeed is bits 0-3.
    u16 options;
    __builtin_memcpy(&options, Save_PlayerData_GetOptionsAddr(saveData), sizeof(options));
    u32 textSpeed = options & 0xF;

    if (textSpeed == 0)
        return 8;
    if (textSpeed == 1)
        return 4;
    return 1;
}

void LONG_CALL Coop_DialogOpen(FieldSystem *fsys, struct CoopDialog *dialog)
{
    void *options = Save_PlayerData_GetOptionsAddr(fsys->savedata);

    AddWindowParameterized(fsys->bg_config, &dialog->window, DIALOG_BG_LAYER,
                           DIALOG_X, DIALOG_Y, DIALOG_W, DIALOG_H,
                           DIALOG_PLTT_TEXT, DIALOG_BASE_TILE);
    TalkWinGraphicSet(fsys->bg_config, DIALOG_BG_LAYER, DIALOG_FRAME_TILE, DIALOG_PLTT_FRAME,
                      CONFIG_GetWindowType(options), HEAP_ID_FIELD1);
    FillWindowPixelBuffer(&dialog->window, DIALOG_BG_FILL);
    BmpTalkWinWrite(&dialog->window, FALSE, DIALOG_FRAME_TILE, DIALOG_PLTT_FRAME);
    fsys->textbox_open = TRUE;

    dialog->text = String_New(COOP_TEXT_MAX * 2, HEAP_ID_FIELD3);
    dialog->textDelay = Coop_GetTextFrameDelay(fsys->savedata);
    dialog->printerId = 0xFF;
    dialog->pending = NULL;
}

// pret TextPrinter (include/font_types_def.h): the template the callback
// receives is its first member, and delayCounter (frames until the next
// character) sits at 0x2A.
#define COOP_TEXT_PRINTER_DELAY_OFFSET 0x2A

// Printer callback, run every frame the printer waits for its next
// character: A or B held prints one character per frame. Vanilla only does
// this after a new press, and only while the global canABSpeedUpPrint flag
// is set (unmapped; vanilla messages set it, so it depended on what printed
// last). Written through a u8 pointer, which may alias anything.
static BOOL Coop_DialogSpeedUp(void *printerTemplate, u16 glyph)
{
    (void)glyph;
    if (gSystem.heldKeys & (PAD_BUTTON_A | PAD_BUTTON_B))
        ((u8 *)printerTemplate)[COOP_TEXT_PRINTER_DELAY_OFFSET] = 0;
    return FALSE; // never pause the printer
}

// Prints one page (two lines max: the box is 4 tiles tall).
static BOOL Coop_DialogPrinterActive(const struct CoopDialog *dialog)
{
    return dialog->printerId != 0xFF && TextPrinterCheckActive(dialog->printerId);
}

void LONG_CALL Coop_DialogPrint(struct CoopDialog *dialog, const char *msg)
{
    if (Coop_DialogPrinterActive(dialog))
    {
        dialog->pending = msg;
        return;
    }
    FillWindowPixelBuffer(&dialog->window, DIALOG_BG_FILL);
    Coop_SetStringFromAscii(dialog->text, msg, dialog->args);
    dialog->printerId = AddTextPrinterParameterized(&dialog->window, DIALOG_FONT, dialog->text, 0, 0, dialog->textDelay,
                                                    Coop_DialogSpeedUp);
}

BOOL LONG_CALL Coop_DialogIsPrinting(struct CoopDialog *dialog)
{
    const char *next;

    if (Coop_DialogPrinterActive(dialog))
        return TRUE;
    if (dialog->pending == NULL)
        return FALSE;
    next = dialog->pending;
    dialog->pending = NULL;
    Coop_DialogPrint(dialog, next);
    return TRUE;
}

// pret ScrCmd_CloseMsg.
void LONG_CALL Coop_DialogClose(FieldSystem *fsys, struct CoopDialog *dialog)
{
    ClearFrameAndWindow2(&dialog->window, FALSE);
    RemoveWindow(&dialog->window);
    String_Delete(dialog->text);
    dialog->text = NULL;
    fsys->textbox_open = FALSE;
}

// A or B, like every vanilla overworld message (ScrCmd_WaitButton).
static BOOL Coop_DialogButtonPressed(void)
{
    return (gSystem.newKeys & (PAD_BUTTON_A | PAD_BUTTON_B)) != 0;
}

// Same allocation as pret FieldSystem_CreateTask / Task_New (task.c), which
// is not mapped in rom.ld.
TaskManager *Coop_FieldSystem_CreateTask(FieldSystem *fsys, TaskFunc func, void *env)
{
    TaskManager *taskman = sys_AllocMemoryLo(HEAP_ID_FIELD3, sizeof(TaskManager));
    TaskManagerUnkSub1C *sub;

    if (taskman == NULL)
        return NULL;
    sub = sys_AllocMemoryLo(HEAP_ID_FIELD3, sizeof(TaskManagerUnkSub1C));
    if (sub == NULL)
    {
        sys_FreeMemoryEz(taskman);
        return NULL;
    }

    taskman->prev = NULL;
    taskman->func = func;
    taskman->state = 0;
    taskman->env = env;
    taskman->unk10 = 0;
    taskman->unk14 = NULL;
    taskman->fieldSystem = fsys;
    taskman->unk1C = sub;
    fsys->taskman = taskman;
    return taskman;
}

// Allocates a zeroed env of envSize bytes and starts func as a field task.
// While it runs, vanilla blocks player input (no FieldInput_Process /
// PlayerAvatar_MoveControl), exactly like an NPC script. Returns FALSE,
// freeing everything, if the field heap is full (callers retry next frame).
BOOL Coop_StartFieldTask(FieldSystem *fsys, TaskFunc func, u32 envSize)
{
    void *env = sys_AllocMemory(HEAP_ID_FIELD3, envSize);
    u8 *bytes = env;
    u32 i;

    if (env == NULL)
        return FALSE;
    for (i = 0; i < envSize; i++)
        bytes[i] = 0;

    if (Coop_FieldSystem_CreateTask(fsys, func, env) == NULL)
    {
        sys_FreeMemoryEz(env);
        return FALSE;
    }
    return TRUE;
}

// ---------------------------------------------------------------------------
// Mid-game disconnect: native field task (freeze, dialog, save, reset)
// ---------------------------------------------------------------------------

enum CoopDisconnectTaskState
{
    COOP_TASK_OPEN_DIALOG,
    COOP_TASK_WAIT_SAVING_TEXT,
    COOP_TASK_SAVE,
    COOP_TASK_WAIT_RESULT_TEXT,
    COOP_TASK_WAIT_A,
};

struct CoopDisconnectTaskEnv
{
    struct CoopDialog dialog;
    u8 frameWait;
    u8 rejection; // relay ROOM_STATUS_ERR_* that turned this save away; 0 = plain disconnect
};

// Relay status codes the bridge copies into room_rejection
// (coop-relay/src/protocol.rs).
#define COOP_REJECT_MIDGAME_SAVE   6
#define COOP_REJECT_SAVE_OTHER_ROOM 9

// Never returns TRUE: the task ends in OS_ResetSystem, so the field task
// manager never frees it. A rejected save (docs/MILESTONE2_PLAN.md 7.1)
// shows why and resets without saving: it is not this room's save.
static BOOL Coop_Task_DisconnectSaveAndReset(TaskManager *taskman)
{
    FieldSystem *fsys = taskman->fieldSystem;
    struct CoopDisconnectTaskEnv *env = taskman->env;

    switch (taskman->state)
    {
    case COOP_TASK_OPEN_DIALOG:
        // Freeze every map object, like the LockAll script command.
        MapObjectMan_PauseAllMovement(fsys->mapObjectMan);
        Coop_DialogOpen(fsys, &env->dialog);
        if (env->rejection == COOP_REJECT_SAVE_OTHER_ROOM)
            Coop_DialogPrint(&env->dialog, "This save belongs to another\nCo-op Room.");
        else if (env->rejection != 0)
            Coop_DialogPrint(&env->dialog, "This room hasn't started yet,\nso it only takes new saves.");
        else
            Coop_DialogPrint(&env->dialog, "Disconnected from the Co-op Room.\nSaving your adventure...");
        taskman->state = COOP_TASK_WAIT_SAVING_TEXT;
        break;

    case COOP_TASK_WAIT_SAVING_TEXT:
        if (!Coop_DialogIsPrinting(&env->dialog))
        {
            env->frameWait = 0;
            taskman->state = COOP_TASK_SAVE;
        }
        break;

    case COOP_TASK_SAVE:
        if (env->rejection != 0)
        {
            if (!Coop_DialogButtonPressed())
                break;
            if (env->rejection == COOP_REJECT_SAVE_OTHER_ROOM)
                Coop_DialogPrint(&env->dialog, "Load that room's save, or\nstart a New Game here.");
            else
                Coop_DialogPrint(&env->dialog, "Start a New Game to join.\nReturning to the Title Screen...");
            taskman->state = COOP_TASK_WAIT_RESULT_TEXT;
            break;
        }
        // Let the finished text reach VRAM before the save blocks the CPU.
        if (++env->frameWait < 2)
            break;

        if (Field_SaveGameNormal(fsys))
            Coop_DialogPrint(&env->dialog, "Your adventure was saved!\nReturning to the Title Screen...");
        else
            Coop_DialogPrint(&env->dialog, "The save failed!\nReturning to the Title Screen...");
        taskman->state = COOP_TASK_WAIT_RESULT_TEXT;
        break;

    case COOP_TASK_WAIT_RESULT_TEXT:
        if (!Coop_DialogIsPrinting(&env->dialog))
            taskman->state = COOP_TASK_WAIT_A;
        break;

    case COOP_TASK_WAIT_A:
        if (Coop_DialogButtonPressed())
            OS_ResetSystem(0);
        break;
    }

    return FALSE;
}

static BOOL Coop_BeginDisconnectSaveAndReset(FieldSystem *fsys)
{
    struct CoopDisconnectTaskEnv *env;

    if (!Coop_StartFieldTask(fsys, Coop_Task_DisconnectSaveAndReset, sizeof(struct CoopDisconnectTaskEnv)))
        return FALSE;
    env = ((TaskManager *)fsys->taskman)->env;
    env->rejection = gCoopCommBlock.room_rejection;
    gCoopCommBlock.disconnect_state = COOP_DISCONNECT_RUNNING;
    return TRUE;
}

// ---------------------------------------------------------------------------
// Bedroom stairs block (lobby): nobody leaves before the adventure starts
// ---------------------------------------------------------------------------

#define COOP_DIR_WEST 2 // pret DIR_WEST (constants/global_fieldmap.h)

// pret constants/movements.h, run through the script movement engine.
#define COOP_MOVEMENT_FACE_UP    0  // + direction
#define COOP_MOVEMENT_STEP_RIGHT 15
#define COOP_MOVEMENT_STEP_END   254

struct CoopMovementCommand // pret MovementScriptCommand
{
    u16 command;
    u16 length;
};

void *LONG_CALL EventObjectMovementMan_Create(LocalMapObject *object, const struct CoopMovementCommand *data);
BOOL LONG_CALL EventObjectMovementMan_IsFinish(void *movementMan);
void LONG_CALL EventObjectMovementMan_Delete(void *movementMan);

enum CoopBedroomBlockState
{
    COOP_BLOCK_WAIT_STILL,
    COOP_BLOCK_OPEN_DIALOG,
    COOP_BLOCK_WAIT_PAGE1_TEXT,
    COOP_BLOCK_WAIT_PAGE1_BUTTON,
    COOP_BLOCK_WAIT_PAGE2_TEXT,
    COOP_BLOCK_WAIT_PAGE2_BUTTON,
    COOP_BLOCK_WAIT_STEP,
};

// A field block (bedroom stairs, gated bosses): two message pages, then one
// movement that takes the player out of the blocking spot, so the check
// does not fire again until they come back. Also talking to a ghost: one or
// two pages naming the player ('@'), and no movement (STEP_END).
struct CoopFieldBlockEnv
{
    struct CoopDialog dialog;
    void *movement;
    const char *page1;
    const char *page2; // NULL: one page
    struct CoopMovementCommand moves[2]; // read by the movement engine until it finishes
    u16 name[COOP_NAME_CHARS + 1]; // always 0xFFFF-terminated (Coop_SetStringFromAscii reads up to 11)
};

BOOL LONG_CALL Coop_IsAdventureStarted(void)
{
    return gCoopCommBlock.gates.flag_campaign_started == 1;
}

// Runs at the end of every frame. The stairs warp
// (FieldSystem_CheckMapTransition) only fires in a later frame's field
// control, once the player's move state is END/NONE while standing on
// (3, 4) facing west with Left held. Facing only changes inside a frame
// (PlayerAvatar_MoveControl or the map-object update), so this check always
// sees "on (3, 4) facing west" before any frame that could warp. Deliberately
// no "standing still" requirement: a field task may start mid-step or
// mid-turn (it only blocks new input), and the task waits for the player to
// stop before showing the message.
static BOOL Coop_ShouldBlockBedroomStairs(FieldSystem *fsys)
{
    LocalMapObject *player;

    if (Coop_IsAdventureStarted() || !Coop_IsFieldMapActive(fsys))
        return FALSE;
    if (fsys->taskman != NULL || fsys->textbox_open)
        return FALSE;
    if (fsys->location->mapId != COOP_BEDROOM_MAP_ID)
        return FALSE;
    player = Coop_GetPlayerMapObject(fsys);
    return player != NULL
        && player->xCurr == COOP_BEDROOM_STAIRS_X
        && player->yCurr == COOP_BEDROOM_STAIRS_Z
        && player->curFacing == COOP_DIR_WEST;
}

// Closes the message and starts the movement (none for STEP_END).
static void Coop_FieldBlockClose(TaskManager *taskman, FieldSystem *fsys, struct CoopFieldBlockEnv *env)
{
    Coop_DialogClose(fsys, &env->dialog);
    MapObjectMan_UnpauseAllMovement(fsys->mapObjectMan);
    // Same engine as the applymovement script command; player input stays
    // blocked until this task ends.
    if (env->moves[0].command != COOP_MOVEMENT_STEP_END)
        env->movement = EventObjectMovementMan_Create(Coop_GetPlayerMapObject(fsys), env->moves);
    taskman->state = COOP_BLOCK_WAIT_STEP;
}

// Returns TRUE when done: the field task manager then frees the task, and
// the env is freed here.
static BOOL Coop_Task_FieldBlock(TaskManager *taskman)
{
    FieldSystem *fsys = taskman->fieldSystem;
    struct CoopFieldBlockEnv *env = taskman->env;

    switch (taskman->state)
    {
    case COOP_BLOCK_WAIT_STILL:
        // Let a step/turn already in progress finish (pret LockAll waits the
        // same way); no new input is accepted while this task exists.
        if (MapObject_IsSingleMovementActive(Coop_GetPlayerMapObject(fsys)))
            break;
        taskman->state = COOP_BLOCK_OPEN_DIALOG;
        // fallthrough
    case COOP_BLOCK_OPEN_DIALOG:
        MapObjectMan_PauseAllMovement(fsys->mapObjectMan);
        Coop_DialogOpen(fsys, &env->dialog);
        env->dialog.args[0] = env->name;
        Coop_DialogPrint(&env->dialog, env->page1);
        taskman->state = COOP_BLOCK_WAIT_PAGE1_TEXT;
        break;

    case COOP_BLOCK_WAIT_PAGE1_TEXT:
        if (!Coop_DialogIsPrinting(&env->dialog))
            taskman->state = COOP_BLOCK_WAIT_PAGE1_BUTTON;
        break;

    case COOP_BLOCK_WAIT_PAGE1_BUTTON:
        if (!Coop_DialogButtonPressed())
            break;
        if (env->page2 != NULL)
        {
            Coop_DialogPrint(&env->dialog, env->page2);
            taskman->state = COOP_BLOCK_WAIT_PAGE2_TEXT;
        }
        else
            Coop_FieldBlockClose(taskman, fsys, env);
        break;

    case COOP_BLOCK_WAIT_PAGE2_TEXT:
        if (!Coop_DialogIsPrinting(&env->dialog))
            taskman->state = COOP_BLOCK_WAIT_PAGE2_BUTTON;
        break;

    case COOP_BLOCK_WAIT_PAGE2_BUTTON:
        if (Coop_DialogButtonPressed())
            Coop_FieldBlockClose(taskman, fsys, env);
        break;

    case COOP_BLOCK_WAIT_STEP:
        // Delete asserts the movement finished, so only after IsFinish.
        if (env->movement != NULL && !EventObjectMovementMan_IsFinish(env->movement))
            break;
        if (env->movement != NULL)
            EventObjectMovementMan_Delete(env->movement);
        sys_FreeMemoryEz(env);
        return TRUE;
    }

    return FALSE;
}

static BOOL Coop_StartFieldBlock(FieldSystem *fsys, const char *page1, const char *page2, u16 movement)
{
    struct CoopFieldBlockEnv *env;

    if (!Coop_StartFieldTask(fsys, Coop_Task_FieldBlock, sizeof(struct CoopFieldBlockEnv)))
        return FALSE;
    env = ((TaskManager *)fsys->taskman)->env;
    env->page1 = page1;
    env->page2 = page2;
    env->moves[0].command = movement;
    env->moves[0].length = 1;
    env->moves[1].command = COOP_MOVEMENT_STEP_END;
    return TRUE;
}

BOOL LONG_CALL Coop_StartTalkDialog(FieldSystem *fsys, const u16 *name, u32 activity)
{
    struct CoopFieldBlockEnv *env;
    const char *page2 = NULL;
    u32 i;

    if (activity == COOP_ACTIVITY_BATTLE)
        page2 = "Looks like they're in the\nmiddle of a battle!";
    else if (activity == COOP_ACTIVITY_MENU)
        page2 = "Looks like they're busy\nright now.";
    if (!Coop_StartFieldBlock(fsys, "It's @!", page2, COOP_MOVEMENT_STEP_END))
        return FALSE;
    env = ((TaskManager *)fsys->taskman)->env;
    for (i = 0; i < COOP_NAME_CHARS; i++)
        env->name[i] = name[i];
    env->name[COOP_NAME_CHARS] = 0xFFFF;
    return TRUE;
}

// ---------------------------------------------------------------------------
// Gym / boss gate (docs/MILESTONE2_PLAN.md 7.2)
// ---------------------------------------------------------------------------

#define COOP_NO_BADGE 0xFF // always gated (Elite Four, Red)

// What A would talk to: the object on the facing tile, or across a counter
// one tile further (pret asm/unk_0203DB6C.s, ccd field_interaction.c).
void LONG_CALL FieldSystem_GetFacingObject(FieldSystem *fsys, LocalMapObject **ret);

struct CoopGatedBoss
{
    u16 mapId;
    u8 x;
    u8 z;
    u8 badge; // PlayerProfile badge index (Johto 0-7, Kanto 8-15)
};

// Bosses the player starts by talking to them (pret zone_event JSON object
// positions). Lance is started by a map script, but only after the Elite
// Four. Talking to a leader whose badge you already have is never blocked.
static const struct CoopGatedBoss sGatedBosses[] = {
    { MAP_T22GYM0101, 15, 4, 0 },              // Falkner
    { MAP_T23GYM0102, 9, 5, 1 },               // Bugsy
    { MAP_T25GYM0101, 13, 4, 2 },              // Whitney
    { MAP_T27GYM0101, 10, 8, 3 },              // Morty
    { MAP_T24GYM0101, 13, 10, 4 },             // Chuck
    { MAP_T26GYM0101, 6, 5, 5 },               // Jasmine
    { MAP_T28GYM0101, 7, 4, 6 },               // Pryce
    { MAP_T30GYM0101, 12, 3, 7 },              // Clair
    { MAP_T03GYM0101, 7, 4, 8 },               // Brock
    { MAP_T04GYM0101, 10, 1, 9 },              // Misty
    { MAP_T06GYM0101, 6, 4, 10 },              // Lt. Surge
    { MAP_T07GYM0101, 7, 3, 11 },              // Erika
    { MAP_T08GYM0101, 7, 16, 12 },             // Janine
    { MAP_T11GYM0101, 15, 13, 13 },            // Sabrina
    { MAP_D11R0106, 13, 12, 14 },              // Blaine
    { MAP_T02GYM0101, 5, 4, 15 },              // Blue
    { MAP_T10R0201, 6, 10, COOP_NO_BADGE },    // Will
    { MAP_T10R0301, 6, 11, COOP_NO_BADGE },    // Koga
    { MAP_T10R0401, 6, 10, COOP_NO_BADGE },    // Bruno
    { MAP_T10R0501, 6, 10, COOP_NO_BADGE },    // Karen
    { MAP_D41R0108, 30, 11, COOP_NO_BADGE },   // Red
};

// Same end-of-frame timing argument as the bedroom stairs: talking needs a
// later frame's A press while facing the boss (from the next tile, or across
// a counter like Falkner's), and this sees "facing the boss" first. The relay keeps
// flag_coop_sync_ready at 0 for this player while a teammate is behind on
// badges (and is neither in this map nor letting others go ahead).
static BOOL Coop_ShouldBlockGatedBoss(FieldSystem *fsys)
{
    LocalMapObject *facing = NULL;

    if (!Coop_IsAdventureStarted() || gCoopCommBlock.gates.flag_coop_sync_ready == 1)
        return FALSE;
    if (!Coop_IsFieldMapActive(fsys) || fsys->taskman != NULL || fsys->textbox_open)
        return FALSE;
    if (Coop_GetPlayerMapObject(fsys) == NULL)
        return FALSE;
    FieldSystem_GetFacingObject(fsys, &facing);
    return Coop_FindGatedBoss(fsys, facing) >= 0;
}

// Team bosses (docs/MILESTONE3_PLAN.md §18): the team boss the player faces,
// for the resident talk hook (src/coop_boss.c). A player whose teammate is
// offline (no partner slot) fights alone, as vanilla.
void LONG_CALL Coop_UpdateBossWatch(FieldSystem *fsys)
{
    LocalMapObject *facing = NULL;

    gCoopBoss.talkObj = NULL;
    gCoopBoss.talkStart = Coop_BossTalkStart;
    gCoopBoss.teamRoster = Coop_TeamRosterFor;
    if (gCoopCommBlock.gates.team_partner_slot == COOP_SLOT_NONE || !Coop_IsAdventureStarted())
        return;
    if (!Coop_IsFieldMapActive(fsys) || fsys->taskman != NULL || fsys->textbox_open || Coop_GetPlayerMapObject(fsys) == NULL)
        return;
    FieldSystem_GetFacingObject(fsys, &facing);
    if (Coop_FindGatedBoss(fsys, facing) >= 0)
        gCoopBoss.talkObj = facing;
}

int LONG_CALL Coop_FindGatedBoss(FieldSystem *fsys, LocalMapObject *obj)
{
    u32 i;

    if (obj == NULL || fsys->location == NULL)
        return -1;
    for (i = 0; i < NELEMS(sGatedBosses); i++)
    {
        const struct CoopGatedBoss *boss = &sGatedBosses[i];
        if (boss->mapId != fsys->location->mapId || boss->x != obj->xCurr || boss->z != obj->yCurr)
            continue;
        if (boss->badge != COOP_NO_BADGE && (gCoopCommBlock.badge_mask_local & (1u << boss->badge)))
            return -1;
        return (int)i;
    }
    return -1;
}

// ---------------------------------------------------------------------------
// Adventure-start save: the roster just locked (docs/MILESTONE2_PLAN.md 7.2)
// ---------------------------------------------------------------------------

enum CoopStartSaveTaskState
{
    COOP_START_OPEN,
    COOP_START_WAIT_PAGE1,
    COOP_START_SAVE,
    COOP_START_WAIT_RESULT,
    COOP_START_WAIT_BUTTON,
};

struct CoopStartSaveTaskEnv
{
    struct CoopDialog dialog;
    u8 frameWait;
    u8 saved;
};

// Returns TRUE when done; the env is freed here, the task by the manager.
static BOOL Coop_Task_AdventureStartSave(TaskManager *taskman)
{
    FieldSystem *fsys = taskman->fieldSystem;
    struct CoopStartSaveTaskEnv *env = taskman->env;

    switch (taskman->state)
    {
    case COOP_START_OPEN:
        MapObjectMan_PauseAllMovement(fsys->mapObjectMan);
        Coop_DialogOpen(fsys, &env->dialog);
        Coop_DialogPrint(&env->dialog, "Everyone is here! Your\nadventure begins now!");
        taskman->state = COOP_START_WAIT_PAGE1;
        break;

    case COOP_START_WAIT_PAGE1:
        if (!Coop_DialogIsPrinting(&env->dialog) && Coop_DialogButtonPressed())
        {
            Coop_DialogPrint(&env->dialog, "The game will now be saved.\nDon't turn off the power.");
            env->frameWait = 0;
            taskman->state = COOP_START_SAVE;
        }
        break;

    case COOP_START_SAVE:
        // Let the text finish and reach VRAM before the save blocks the CPU.
        if (Coop_DialogIsPrinting(&env->dialog) || ++env->frameWait < 2)
            break;
        env->saved = Field_SaveGameNormal(fsys) ? 1 : 0;
        Coop_DialogPrint(&env->dialog, env->saved ? "Your adventure was saved!\nHave fun, everyone!"
                                                   : "The save failed!\nIt will be tried again.");
        taskman->state = COOP_START_WAIT_RESULT;
        break;

    case COOP_START_WAIT_RESULT:
        if (!Coop_DialogIsPrinting(&env->dialog))
            taskman->state = COOP_START_WAIT_BUTTON;
        break;

    case COOP_START_WAIT_BUTTON:
        if (!Coop_DialogButtonPressed())
            break;
        Coop_DialogClose(fsys, &env->dialog);
        MapObjectMan_UnpauseAllMovement(fsys->mapObjectMan);
        gCoopCommBlock.start_save_state = env->saved ? COOP_START_SAVE_DONE : COOP_START_SAVE_NONE;
        sys_FreeMemoryEz(env);
        return TRUE;
    }
    return FALSE;
}

// Start the save once the roster is locked while this player still has the
// fresh bedroom save the room was formed with.
static void Coop_TryAdventureStartSave(FieldSystem *fsys)
{
    if (gCoopCommBlock.start_save_state != COOP_START_SAVE_NONE || !Coop_IsAdventureStarted())
        return;
    if (gCoopCommBlock.local_player.save_status != COOP_SAVE_FRESH || !Coop_IsSafeOverworldWindow(fsys))
        return;
    if (Coop_StartFieldTask(fsys, Coop_Task_AdventureStartSave, sizeof(struct CoopStartSaveTaskEnv)))
        gCoopCommBlock.start_save_state = COOP_START_SAVE_RUNNING;
}


// ---------------------------------------------------------------------------
// Gifts (docs/GAME_REBALANCE_UPDATE.md §7.2, §9.1)
// ---------------------------------------------------------------------------

// Done-flags in the range no vanilla script, pret source or hg-engine uses
// (2522-2559, rebalance plan §18).
#define COOP_FLAG_GIFT_KEY_STONE 2522
#define COOP_FLAG_GAME_CLEAR     2404 // vanilla FLAG_GAME_CLEAR (asm/include/flags.inc)

// The Key Stone, once the credits have rolled: in the next quiet moment in the
// overworld, the player finds it and Mega Evolution unlocks. Returns TRUE when
// a gift message started.
static BOOL Coop_TryGifts(FieldSystem *fsys)
{
    if (!CheckScriptFlag(COOP_FLAG_GAME_CLEAR) || CheckScriptFlag(COOP_FLAG_GIFT_KEY_STONE))
        return FALSE;
    if (!Coop_IsSafeOverworldWindow(fsys))
        return FALSE;
    if (!Coop_StartFieldBlock(fsys, "A Key Stone arrived from the\nPokémon League for its Champion!",
                              "Mega Evolution awaits you\nin Kanto.", COOP_MOVEMENT_STEP_END))
        return FALSE;
    Bag_AddItem(Sav2_Bag_get(fsys->savedata), ITEM_KEY_STONE, 1, 11 /* HEAP_ID_FIELD2 */);
    SetScriptFlag(FLAG_MEGA_EVOLUTION_ENABLED);
    SetScriptFlag(COOP_FLAG_GIFT_KEY_STONE);
    return TRUE;
}

// ---------------------------------------------------------------------------
// Connection-driven field tasks (called at the end of every field frame)
// ---------------------------------------------------------------------------

void LONG_CALL Coop_UpdateFieldConnection(FieldSystem *fsys)
{
    switch (gCoopCommBlock.disconnect_state)
    {
    case COOP_DISCONNECT_NONE:
        if (!Coop_IsConnectedToRoom())
            break;
        // Lobby: keep everyone in the bedroom until the adventure starts,
        // then save once so the room is tied to a save on disk.
        if (Coop_ShouldBlockBedroomStairs(fsys))
            // Facing west on the stairs: stepping back east always returns
            // the player to (4, 4), the tile they came from or an open one.
            (void)Coop_StartFieldBlock(fsys, "Oops! Looks like some trainers\nare still asleep.",
                                       "Please wait until everyone is\nready to start your adventure.",
                                       COOP_MOVEMENT_STEP_RIGHT);
        else if (Coop_ShouldBlockGatedBoss(fsys))
            // Turn away (the tile behind may be a wall); turning back
            // re-checks before the next A press can reach the boss.
            (void)Coop_StartFieldBlock(fsys, "Your teammates need to catch\nup before this battle.",
                                       "Wait for them, or ask them to\nlet you go ahead.",
                                       COOP_MOVEMENT_FACE_UP + ((u32)Coop_GetPlayerMapObject(fsys)->curFacing ^ 1));
        else if (!Coop_TryGifts(fsys))
            Coop_TryAdventureStartSave(fsys);
        break;

    case COOP_DISCONNECT_PENDING:
        // Deferred through battles, apps, scripts, menus and mid-step frames.
        if (Coop_IsSafeOverworldWindow(fsys))
            (void)Coop_BeginDisconnectSaveAndReset(fsys);
        break;

    default:
        break;
    }
}
