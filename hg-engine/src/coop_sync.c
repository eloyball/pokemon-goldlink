#include "config.h"
#include "types.h"

#include "constants/buttons.h"
#include "constants/file.h"
#include "constants/maps.h"
#include "constants/species.h"

#include "message.h"
#include "pokemon.h"
#include "save.h"
#include "map_events_internal.h"
#include "overlay.h"
#include "system.h"
#include "task.h"
#include "window.h"
#include "coop_sync.h"

// Vanilla functions not declared (or declared without LONG_CALL) in any
// hg-engine header. Addresses live in rom.ld.
struct Party *LONG_CALL SaveArray_Party_Get(void *saveData);
void *LONG_CALL Save_PlayerData_GetOptionsAddr(void *saveData);
u8 LONG_CALL TextPrinterCheckActive(u8 printerId);
void LONG_CALL AddWindowParameterized(void *bgConfig, void *window, u8 bgId, u8 x, u8 y, u8 width, u8 height, u8 paletteNum, u16 baseTile);
// Overlay 01: syncs map objects + player position into the save, then
// calls SaveGameNormal. Returns TRUE on success.
BOOL LONG_CALL Field_SaveGameNormal(FieldSystem *fsys);

// Heaps (pret include/constants/heap.h).
#define HEAP_ID_DEFAULT 0
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

#define COOP_TEXT_MAX 96

// 512-byte shared mailbox. Its own initialized section, placed first in
// Overlay 129 by src/linker.ld, so it carries its initial values (Overlay
// 129 is loaded without BSS init) and sits at a fixed address
// (0x023D8060) that extract_coop_sym.py verifies.
__attribute__((section(".coop_mailbox"), used, aligned(4)))
volatile struct CoopCommBlock gCoopCommBlock = {
    .magic = COOP_MAGIC,
    .version = COOP_MAILBOX_VERSION,
    .assigned_player_id = COOP_PLAYER_ID_NONE,
    .disconnect_state = COOP_DISCONNECT_NONE,
    // Not connected until the bridge proves it is alive.
    .bridge_stale_frames = COOP_BRIDGE_TIMEOUT_FRAMES,
    .limit_map_id = MAP_ID_MAX,
    .limit_species = MAX_SPECIES_INCLUDING_FORMS,
};

// ---------------------------------------------------------------------------
// Text helpers
// ---------------------------------------------------------------------------

// Converts a plain ASCII message into an HG-encoded String (charmap.txt).
// Supports A-Z a-z 0-9, space, ! ? , . - ' and \n; "..." becomes the
// ellipsis glyph. Anything else is rendered as '?'.
static void Coop_SetStringFromAscii(String *dst, const char *src)
{
    u16 buf[COOP_TEXT_MAX + 1];
    u32 n = 0;

    while (*src != '\0' && n < COOP_TEXT_MAX)
    {
        char c = *src++;
        u16 out;

        if (c >= 'A' && c <= 'Z')
            out = 0x012B + (c - 'A');
        else if (c >= 'a' && c <= 'z')
            out = 0x0145 + (c - 'a');
        else if (c >= '0' && c <= '9')
            out = 0x0121 + (c - '0');
        else if (c == '.' && src[0] == '.' && src[1] == '.')
        {
            out = 0x01AF; // …
            src += 2;
        }
        else
        {
            switch (c)
            {
            case ' ':  out = 0x01DE; break;
            case '!':  out = 0x01AB; break;
            case ',':  out = 0x01AD; break;
            case '.':  out = 0x01AE; break;
            case '-':  out = 0x01BE; break;
            case '\'': out = 0x01B3; break;
            case '\n': out = 0xE000; break;
            default:   out = 0x01AC; break; // ?
            }
        }
        buf[n++] = out;
    }
    buf[n] = 0xFFFF;
    CopyU16ArrayToString(dst, buf);
}

// ---------------------------------------------------------------------------
// Connection state
// ---------------------------------------------------------------------------

// Connected only when the bridge says so AND it is still alive. The
// heartbeat check covers the bridge window being closed, the bridge
// crashing, and savestates restoring a stale "connected" flag.
BOOL LONG_CALL Coop_IsConnectedToRoom(void)
{
    return gCoopCommBlock.is_connected_to_room != 0
        && gCoopCommBlock.bridge_stale_frames < COOP_BRIDGE_TIMEOUT_FRAMES;
}

static void Coop_UpdateBridgeWatchdog(void)
{
    u32 hb = gCoopCommBlock.heartbeat_bridge;

    if (hb != gCoopCommBlock.bridge_hb_last_seen)
    {
        gCoopCommBlock.bridge_hb_last_seen = hb;
        gCoopCommBlock.bridge_stale_frames = 0;
    }
    else if (gCoopCommBlock.bridge_stale_frames < COOP_BRIDGE_TIMEOUT_FRAMES)
    {
        gCoopCommBlock.bridge_stale_frames++;
    }
}

// ---------------------------------------------------------------------------
// Startup gate (title screen, before the overworld exists)
// ---------------------------------------------------------------------------

// Kept to <= 22 characters per line: the window is 208 px wide.
static const char *const sGateLines[] = {
    "Not in a Co-op Room.",
    "Open the Co-op Bridge,",
    "create or join a room,",
    "then start again.",
    "Press A, B or START.",
};

static const u16 sGateLineY[] = { 16, 44, 60, 76, 108 };

// Any of these dismisses the startup gate. A and B are how every vanilla
// message is dismissed, and START is what players press on the title flow
// this screen interrupts. SELECT is left out (it is part of the vanilla
// soft-reset chord) and so are the D-pad and shoulders, so resting a thumb
// or holding L/R never skips the message by accident.
#define COOP_GATE_DISMISS_KEYS (PAD_BUTTON_A | PAD_BUTTON_B | PAD_BUTTON_START)

// Full-screen message adapted from PrintCrashMessageAndReset (src/save.c):
// same heap/IRQ/BG setup with a black background. Only used before the
// overworld is running (New Game / Continue), never mid-game: it rebuilds
// the heap system. Blocks on a fresh A press, then soft-resets.
static void Coop_ShowStartupGateAndReset(void)
{
    u32 window;
    u32 line;

    // Quiesce interrupts BEFORE touching the heap: a VBlank IRQ firing while
    // InitHeapSystem runs would walk freed SysTask lists / DMA buffers.
    OS_DisableIrqMask(1);
    Main_SetVBlankIntrCB(NULL, NULL);
    Main_SetHBlankIntrCB(NULL, NULL);

    OS_SetArenaHi(0, OS_GetInitArenaHi(0));
    OS_SetArenaLo(0, OS_GetInitArenaLo(0));
    InitHeapSystem((void *)0x02108528, 1, 1, 0);

    sub_0200FBF4(0, 0);
    sub_0200FBF4(1, 0);

    OS_SetIrqFunction(1, (void *)(0x2096318 | 1));
    OS_EnableIrqMask(1);

    GfGfx_DisableEngineAPlanes();
    GfGfx_DisableEngineBPlanes();
    GX_SetVisiblePlane(0);
    GXS_SetVisiblePlane(0);

    SetKeyRepeatTimers(4, 8);

    *(u8 *)(0x21D116C) = FALSE; // gSystem.screensFlipped

    GfGfx_SwapDisplay();
    G2_BlendNone();
    G2S_BlendNone();
    GX_SetVisibleWnd(0);
    GXS_SetVisibleWnd(0);
    GfGfx_SetBanks((void *)0x0210855C);

    void *bg_config = BgConfig_Alloc(HEAP_ID_DEFAULT);
    SetBothScreensModesAndDisable((void *)0x02108530);
    InitBgFromTemplate(bg_config, 0, (void *)0x02108540, 0);
    BgClearTilemapBufferAndCommit(bg_config, 0);
    LoadUserFrameGfx1(bg_config, 0, 0x1F7, 2, 0, HEAP_ID_DEFAULT);
    LoadFontPal0(0, 0x20, HEAP_ID_DEFAULT);
    BG_ClearCharDataRange(0, 0x20, 0, 0);

    BG_SetMaskColor(0, 0x0000);
    BG_SetMaskColor(4, 0x0000);

    ResetAllTextPrinters();

    AddWindow(bg_config, &window, (void *)(0x02108520));
    FillWindowPixelRect(&window, 0xF, 0, 0, 208, 144);
    DrawFrameAndWindow1(&window, FALSE, 0x1F7, 2);

    for (line = 0; line < NELEMS(sGateLines); line++)
    {
        String *msg_str = String_New(64, HEAP_ID_DEFAULT);
        Coop_SetStringFromAscii(msg_str, sGateLines[line]);
        AddTextPrinterParameterized(&window, 0, msg_str, 8, sGateLineY[line], 0, NULL);
        String_Delete(msg_str);
    }

    GfGfx_BothDispOn();
    SetMasterBrightnessNeutral(0);
    SetMasterBrightnessNeutral(1);
    SetBlendBrightness(0, 0x3F, 3);

    // Flush the window and text printers to VRAM (as PrintCrashMessageAndReset).
    sub_02038D90();

    while (TRUE)
    {
        HandleDSLidAction();
        sub_02036144();
        if (sub_02039AA4())
            break;
        OS_WaitIrq(TRUE, 1);
    }

    // A/START is usually still held from the title menu: wait until every
    // dismiss key is released, then for a fresh press of any of them.
    while (PAD_Read() & COOP_GATE_DISMISS_KEYS)
    {
        HandleDSLidAction();
        OS_WaitIrq(TRUE, 1);
    }
    while (!(PAD_Read() & COOP_GATE_DISMISS_KEYS))
    {
        HandleDSLidAction();
        OS_WaitIrq(TRUE, 1);
    }

    sub_0200FBF4(0, 0x7FFF);
    sub_0200FBF4(1, 0x7FFF);

    RemoveWindow(&window);
    sys_FreeMemoryEz(bg_config);

    OS_ResetSystem(0);
}

// Called from the New Game (Save_InitDynamicRegion) and Continue
// (FieldSystem_RestoreMapObjectsFromSave) hooks in src/save.c.
void LONG_CALL Coop_CheckStartupGate(void)
{
    if (!Coop_IsConnectedToRoom())
    {
        Coop_ShowStartupGateAndReset();
    }
    // Passing the gate starts the session: a drop from here on (even during
    // the New Game intro) is latched and handled at the first safe overworld
    // frame, so nobody reaches the overworld offline.
    gCoopCommBlock.was_in_active_session = 1;
}

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
// child check.
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
static BOOL Coop_IsSafeOverworldWindow(FieldSystem *fsys)
{
    LocalMapObject *player;

    if (!Coop_IsFieldMapActive(fsys))
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

// Vanilla dialog box (pret ScrCmd_OpenMsg / DialogBox_LoadFrame) on BG3.
struct CoopDialog
{
    GF_BGL_BMPWIN window;
    String *text;
    u8 printerId;
    u8 textDelay;
};

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

static void Coop_DialogOpen(FieldSystem *fsys, struct CoopDialog *dialog)
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
}

// Prints one page (two lines max: the box is 4 tiles tall).
static void Coop_DialogPrint(struct CoopDialog *dialog, const char *msg)
{
    FillWindowPixelBuffer(&dialog->window, DIALOG_BG_FILL);
    Coop_SetStringFromAscii(dialog->text, msg);
    dialog->printerId = AddTextPrinterParameterized(&dialog->window, DIALOG_FONT, dialog->text, 0, 0, dialog->textDelay, NULL);
}

static BOOL Coop_DialogIsPrinting(const struct CoopDialog *dialog)
{
    return TextPrinterCheckActive(dialog->printerId);
}

// pret ScrCmd_CloseMsg.
static void Coop_DialogClose(FieldSystem *fsys, struct CoopDialog *dialog)
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
};

// Never returns TRUE: the task ends in OS_ResetSystem, so the field task
// manager never frees it.
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
    if (!Coop_StartFieldTask(fsys, Coop_Task_DisconnectSaveAndReset, sizeof(struct CoopDisconnectTaskEnv)))
        return FALSE;
    gCoopCommBlock.disconnect_state = COOP_DISCONNECT_RUNNING;
    return TRUE;
}

// ---------------------------------------------------------------------------
// Bedroom stairs block (lobby): nobody leaves before the adventure starts
// ---------------------------------------------------------------------------

#define COOP_DIR_WEST 2 // pret DIR_WEST (constants/global_fieldmap.h)

// pret constants/movements.h, run through the script movement engine.
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

// The block only fires while facing west, so stepping back east always
// returns the player to (4, 4), the tile they came from or an open one.
static const struct CoopMovementCommand sStepAwayFromStairs[] = {
    { COOP_MOVEMENT_STEP_RIGHT, 1 },
    { COOP_MOVEMENT_STEP_END, 0 },
};

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

struct CoopBedroomBlockEnv
{
    struct CoopDialog dialog;
    void *movement;
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

// Returns TRUE when done: the field task manager then frees the task, and
// the env is freed here.
static BOOL Coop_Task_BedroomBlock(TaskManager *taskman)
{
    FieldSystem *fsys = taskman->fieldSystem;
    struct CoopBedroomBlockEnv *env = taskman->env;

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
        Coop_DialogPrint(&env->dialog, "Oops! Looks like some trainers\nare still asleep.");
        taskman->state = COOP_BLOCK_WAIT_PAGE1_TEXT;
        break;

    case COOP_BLOCK_WAIT_PAGE1_TEXT:
        if (!Coop_DialogIsPrinting(&env->dialog))
            taskman->state = COOP_BLOCK_WAIT_PAGE1_BUTTON;
        break;

    case COOP_BLOCK_WAIT_PAGE1_BUTTON:
        if (Coop_DialogButtonPressed())
        {
            Coop_DialogPrint(&env->dialog, "Please wait until everyone is\nready to start your adventure.");
            taskman->state = COOP_BLOCK_WAIT_PAGE2_TEXT;
        }
        break;

    case COOP_BLOCK_WAIT_PAGE2_TEXT:
        if (!Coop_DialogIsPrinting(&env->dialog))
            taskman->state = COOP_BLOCK_WAIT_PAGE2_BUTTON;
        break;

    case COOP_BLOCK_WAIT_PAGE2_BUTTON:
        if (Coop_DialogButtonPressed())
        {
            Coop_DialogClose(fsys, &env->dialog);
            MapObjectMan_UnpauseAllMovement(fsys->mapObjectMan);
            // Same engine as the applymovement script command; player input
            // stays blocked until this task ends.
            env->movement = EventObjectMovementMan_Create(Coop_GetPlayerMapObject(fsys), sStepAwayFromStairs);
            taskman->state = COOP_BLOCK_WAIT_STEP;
        }
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

// ---------------------------------------------------------------------------
// Local player telemetry
// ---------------------------------------------------------------------------

static void Coop_BuildLocalPlayerState(FieldSystem *fsys, struct CoopPlayerState *ps, u16 *badgeMask)
{
    LocalMapObject *mo = Coop_GetPlayerMapObject(fsys);
    MapObjectMan *mapObjects = fsys->mapObjectMan;
    struct PlayerProfile *profile;
    struct Party *party;
    u32 partyCount = 0;
    u32 i;

    ps->map_id = (u16)fsys->location->mapId;
    if (mo != NULL)
    {
        // Tile coordinates (ints in LocalMapObject).
        ps->x = (s16)mo->xCurr;
        ps->y = (s16)mo->yCurr;
        ps->prev_x = (s16)mo->xPrev;
        ps->prev_y = (s16)mo->yPrev;
        ps->direction = (u8)mo->curFacing;
        ps->movement_type = (u8)mo->movement;
        ps->sprite_id = (u16)mo->gfxId;
        ps->animation_frame = (u8)mo->movementStep;
    }

    // Live map objects other than the player (NPCs, follower, props). The
    // relay hides remote followers above a threshold to protect OBJ VRAM.
    if (mapObjects != NULL && mapObjects->active_count > 0)
    {
        u32 others = mapObjects->active_count - 1;
        ps->map_npc_count = (u8)(others > 0xFF ? 0xFF : others);
    }

    profile = Sav2_PlayerData_GetProfileAddr(fsys->savedata);
    if (profile != NULL)
    {
        ps->trainer_id = profile->id;
        for (i = 0; i < 16; i++)
        {
            if (PlayerProfile_TestBadgeFlag(profile, (s32)i) == TRUE)
                *badgeMask |= (u16)(1u << i);
        }
    }

    party = SaveArray_Party_Get(fsys->savedata);
    if (party != NULL)
    {
        partyCount = (u32)party->count;
        if (partyCount > 0)
        {
            struct PartyPokemon *lead = Party_GetMonByIndex(party, 0);
            if (lead != NULL)
            {
                ps->lead_species = (u16)GetMonData(lead, MON_DATA_SPECIES, NULL);
                ps->lead_form = (u16)GetMonData(lead, MON_DATA_FORM, NULL);
                ps->lead_is_shiny = MonIsShiny(lead) ? 1 : 0;
                ps->lead_gender = (u8)GetMonData(lead, MON_DATA_GENDER, NULL);
            }
        }
    }

    // Lobby readiness lives here, next to the game state it depends on.
    ps->client_ready = (ps->map_id == COOP_BEDROOM_MAP_ID && ps->trainer_id != 0 && partyCount == 0) ? 1 : 0;
    ps->is_active = 1;
}

// Publishes a complete snapshot under a sequence lock: local_player_seq is
// odd while the copy is in progress, so the bridge can reject a torn read
// (possible on lag frames, when a frame boundary lands mid-copy).
static void Coop_PublishLocalPlayerState(const struct CoopPlayerState *ps, u16 badgeMask)
{
    volatile u32 *dst = (volatile u32 *)&gCoopCommBlock.local_player;
    u32 words[sizeof(*ps) / sizeof(u32)];
    u32 i;

    // memcpy, not a (const u32 *) cast of ps: reading the struct through an
    // incompatible pointer type is undefined behaviour, and at -Os with
    // -fipa-pta GCC deleted the byte stores (is_active, client_ready,
    // lead_gender) as dead, publishing them as 0.
    __builtin_memcpy(words, ps, sizeof(words));

    gCoopCommBlock.local_player_seq++;
    for (i = 0; i < sizeof(words) / sizeof(u32); i++)
        dst[i] = words[i];
    gCoopCommBlock.badge_mask_local = badgeMask;
    gCoopCommBlock.local_player_seq++;
}

static void Coop_UpdateLocalPlayerState(FieldSystem *fsys, BOOL fieldActive)
{
    if (fieldActive)
    {
        struct CoopPlayerState ps = { 0 };
        u16 badgeMask = 0;

        Coop_BuildLocalPlayerState(fsys, &ps, &badgeMask);
        Coop_PublishLocalPlayerState(&ps, badgeMask);
    }
    else if (gCoopCommBlock.local_player.is_active)
    {
        // In a battle / app / transition: keep the last known position and
        // OTID (other players keep seeing where we stood), but flag inactive.
        gCoopCommBlock.local_player_seq++;
        gCoopCommBlock.local_player.is_active = 0;
        gCoopCommBlock.local_player_seq++;
    }
}

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

BOOL LONG_CALL Coop_ReadRemotePlayer(u32 remoteSlot, struct CoopPlayerState *out)
{
    Coop_Zero(out, sizeof(*out));
    if (remoteSlot >= COOP_REMOTE_SLOTS)
        return FALSE;

    Coop_CopyFromMailbox(out, &gCoopCommBlock.remote_players[remoteSlot], sizeof(*out));
    if (out->is_active != 1
        || out->map_id == 0 || out->map_id >= MAP_ID_MAX
        || !Coop_IsValidDirection(out->direction)
        || out->lead_species > MAX_SPECIES_INCLUDING_FORMS
        || out->lead_is_shiny > 1 || out->lead_gender > 2 || out->client_ready > 1)
    {
        Coop_Zero(out, sizeof(*out));
        return FALSE;
    }
    return TRUE;
}

BOOL LONG_CALL Coop_ReadRemoteFollower(u32 remoteSlot, struct CoopFollowerState *out)
{
    Coop_Zero(out, sizeof(*out));
    if (remoteSlot >= COOP_REMOTE_SLOTS)
        return FALSE;

    Coop_CopyFromMailbox(out, &gCoopCommBlock.remote_followers[remoteSlot], sizeof(*out));
    if (out->is_visible != 1
        || out->species == 0 || out->species > MAX_SPECIES_INCLUDING_FORMS
        || !Coop_IsValidDirection(out->direction)
        || out->is_shiny > 1 || out->gender > 2)
    {
        Coop_Zero(out, sizeof(*out));
        return FALSE;
    }
    return TRUE;
}

// ---------------------------------------------------------------------------
// Overlay 131 field code
// ---------------------------------------------------------------------------

// Calls the co-op field code in Overlay 131 (src/field/coop_ghosts.c) through
// the header src/field/linker.ld pins at COOP_FIELD_ENTRY_ADDR: this overlay
// links before 131 and cannot reference its symbols. The magic check guards
// against a stale or missing Overlay 131 build.
static void Coop_RunFieldEntry(FieldSystem *fsys)
{
    const struct CoopFieldEntryHeader *header = (const struct CoopFieldEntryHeader *)COOP_FIELD_ENTRY_ADDR;

    if (!IsOverlayLoaded(OVERLAY_FIELD_EXTENSION))
    {
        gCoopCommBlock.diag.field_entry_status = COOP_FIELD_ENTRY_NO_OVERLAY;
        return;
    }
    if (header->magic != COOP_FIELD_ENTRY_MAGIC || header->entry == NULL)
    {
        gCoopCommBlock.diag.field_entry_status = COOP_FIELD_ENTRY_BAD_HEADER;
        return;
    }
    gCoopCommBlock.diag.field_entry_status = COOP_FIELD_ENTRY_CALLED;
    header->entry(fsys);
}

// ---------------------------------------------------------------------------
// Main SysTask
// ---------------------------------------------------------------------------

void Coop_MainSysTask(struct SysTask *task, void *param)
{
    FieldSystem *fsys = gFieldSysPtr;
    BOOL fieldActive;
    BOOL connected;

    (void)task;
    (void)param;

    gCoopCommBlock.heartbeat_rom++;
    Coop_UpdateBridgeWatchdog();

    fieldActive = Coop_IsFieldMapActive(fsys);
    connected = Coop_IsConnectedToRoom();

    Coop_UpdateLocalPlayerState(fsys, fieldActive);
    if (fieldActive)
        Coop_RunFieldEntry(fsys);
    else
        gCoopCommBlock.diag.field_entry_status = COOP_FIELD_ENTRY_NOT_IN_FIELD;

    switch (gCoopCommBlock.disconnect_state)
    {
    case COOP_DISCONNECT_NONE:
        if (connected)
        {
            // A session is "live" once the startup gate passed or we have
            // been in the overworld while connected.
            if (fieldActive)
                gCoopCommBlock.was_in_active_session = 1;

            // Lobby: keep everyone in the bedroom until the adventure starts.
            if (Coop_ShouldBlockBedroomStairs(fsys))
                (void)Coop_StartFieldTask(fsys, Coop_Task_BedroomBlock, sizeof(struct CoopBedroomBlockEnv));
        }
        else if (gCoopCommBlock.was_in_active_session || fieldActive)
        {
            // Latched: a reconnect before the safe window does not cancel it.
            // Being in the overworld without a connection is never allowed,
            // even if the startup gate was bypassed (e.g. the vanilla
            // "continue from Union Room" path skips the Continue hook).
            gCoopCommBlock.disconnect_state = COOP_DISCONNECT_PENDING;
        }
        break;

    case COOP_DISCONNECT_PENDING:
        // Deferred through battles, apps, scripts, menus and mid-step frames.
        if (Coop_IsSafeOverworldWindow(fsys))
            (void)Coop_BeginDisconnectSaveAndReset(fsys);
        break;

    case COOP_DISCONNECT_RUNNING:
    default:
        break;
    }
}
