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

#define COOP_NAME_CHARS 8

// Converts a plain ASCII message into an HG-encoded String (charmap.txt).
// Supports A-Z a-z 0-9, space, ! ? , . - ' and \n; "..." becomes the
// ellipsis glyph. Anything else is rendered as '?'. '@' inserts name (an
// HG-encoded player name from the relay, or NULL): up to 8 chars, ending at
// 0xFFFF; control and format codes (0x200 and up) become '?'.
static void Coop_SetStringFromAscii(String *dst, const char *src, const u16 *name)
{
    u16 buf[COOP_TEXT_MAX + 1];
    u32 n = 0;
    u32 i;

    while (*src != '\0' && n < COOP_TEXT_MAX)
    {
        char c = *src++;
        u16 out;

        if (c == '@' && name != NULL)
        {
            for (i = 0; i < COOP_NAME_CHARS && name[i] != 0xFFFF && n < COOP_TEXT_MAX; i++)
                buf[n++] = (name[i] != 0 && name[i] < 0x200) ? name[i] : 0x01AC;
            continue;
        }
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

// Connected, but the bridge has not loaded this room's save yet
// (docs/MILESTONE2_PLAN.md 7.2). It swaps it in and the game restarts.
static const char *const sSlotGateLines[] = {
    "Getting this room's",
    "save ready...",
    "",
    "The game will restart.",
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
static void Coop_ShowStartupGateAndReset(const char *const *lines)
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

    for (line = 0; line < NELEMS(sGateLineY); line++)
    {
        String *msg_str = String_New(64, HEAP_ID_DEFAULT);
        Coop_SetStringFromAscii(msg_str, lines[line], NULL);
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
        Coop_ShowStartupGateAndReset(sGateLines);
    if (gCoopCommBlock.save_slot_ready != 1)
        Coop_ShowStartupGateAndReset(sSlotGateLines);
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
    const u16 *name; // what '@' prints (Coop_SetStringFromAscii), or NULL
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
static void Coop_DialogPrint(struct CoopDialog *dialog, const char *msg)
{
    FillWindowPixelBuffer(&dialog->window, DIALOG_BG_FILL);
    Coop_SetStringFromAscii(dialog->text, msg, dialog->name);
    dialog->printerId = AddTextPrinterParameterized(&dialog->window, DIALOG_FONT, dialog->text, 0, 0, dialog->textDelay,
                                                    Coop_DialogSpeedUp);
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
    u16 name[COOP_NAME_CHARS];
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
        env->dialog.name = env->name;
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
    u32 i;

    if (!Coop_IsAdventureStarted() || gCoopCommBlock.gates.flag_coop_sync_ready == 1)
        return FALSE;
    if (!Coop_IsFieldMapActive(fsys) || fsys->taskman != NULL || fsys->textbox_open)
        return FALSE;
    if (Coop_GetPlayerMapObject(fsys) == NULL)
        return FALSE;
    FieldSystem_GetFacingObject(fsys, &facing);
    if (facing == NULL)
        return FALSE;
    for (i = 0; i < NELEMS(sGatedBosses); i++)
    {
        const struct CoopGatedBoss *boss = &sGatedBosses[i];
        if (boss->mapId != fsys->location->mapId || boss->x != facing->xCurr || boss->z != facing->yCurr)
            continue;
        if (boss->badge != COOP_NO_BADGE && (gCoopCommBlock.badge_mask_local & (1u << boss->badge)))
            return FALSE;
        return TRUE;
    }
    return FALSE;
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
// Local player telemetry
// ---------------------------------------------------------------------------

// What the player is doing while the field map is not running.
static u8 Coop_GetActivity(FieldSystem *fsys)
{
    if (IsOverlayLoaded(OVERLAY_BATTLE))
        return COOP_ACTIVITY_BATTLE;
    if (fsys != NULL && fsys->processManager != NULL && fsys->processManager->child != NULL)
        return COOP_ACTIVITY_MENU;
    return COOP_ACTIVITY_TRANSITION;
}

// In the field, Overlay 131 publishes the full state every frame
// (Coop_PublishLocalPlayerState, src/field/coop_ghosts.c). Outside it, keep
// the last position and identity (other players' ghosts freeze where we
// stood) and report what we are doing instead.
static void Coop_UpdateLocalPlayerState(FieldSystem *fsys, BOOL fieldActive)
{
    volatile struct CoopPlayerState *local = &gCoopCommBlock.local_player;
    u8 activity;

    if (fieldActive || local->trainer_id == 0)
        return; // published by Overlay 131, or never been in the field
    activity = Coop_GetActivity(fsys);
    if (local->is_active == 0 && local->activity == activity)
        return;
    gCoopCommBlock.local_player_seq++;
    local->is_active = 0;
    local->activity = activity;
    gCoopCommBlock.local_player_seq++;
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
        || out->activity > COOP_ACTIVITY_TRANSITION)
    {
        Coop_Zero(out, sizeof(*out));
        return FALSE;
    }
    if (out->anim > COOP_MAX_ANIMATION)
        out->anim = 0; // cosmetic: show the pose's first animation instead of dropping the player
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

    // The bridge swapped this room's save into the cartridge (SRAM): restart
    // so the game loads it. Only before a session started (title screen,
    // main menu), never once a save is in play.
    if (gCoopCommBlock.reset_request == 1 && !gCoopCommBlock.was_in_active_session && !fieldActive)
        OS_ResetSystem(0);

    Coop_UpdateLocalPlayerState(fsys, fieldActive);
    if (fieldActive)
        Coop_RunFieldEntry(fsys);
    else
    {
        gCoopCommBlock.diag.field_entry_status = COOP_FIELD_ENTRY_NOT_IN_FIELD;
        gCoopCommBlock.track_field_idle = 0; // the first field frame cannot start a ghost talk
        gCoopCommBlock.track_field_hidden = 1; // ghosts catch up by teleporting when the field returns
    }

    // The relay turned this save away (docs/MILESTONE2_PLAN.md 7.1): same
    // safe-window path as a disconnect, but the task resets without saving.
    if (gCoopCommBlock.room_rejection != 0 && gCoopCommBlock.disconnect_state == COOP_DISCONNECT_NONE)
        gCoopCommBlock.disconnect_state = COOP_DISCONNECT_PENDING;

    switch (gCoopCommBlock.disconnect_state)
    {
    case COOP_DISCONNECT_NONE:
        if (connected)
        {
            // A session is "live" once the startup gate passed or we have
            // been in the overworld while connected.
            if (fieldActive)
                gCoopCommBlock.was_in_active_session = 1;

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
            else
                Coop_TryAdventureStartSave(fsys);
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
