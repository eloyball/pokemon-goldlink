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
#include "coop_dialog.h"
#include "coop_link.h"
#include "coop_boss.h"

// Vanilla functions not declared (or declared without LONG_CALL) in any
// hg-engine header. Addresses live in rom.ld.

// Heaps (pret include/constants/heap.h).
#define HEAP_ID_DEFAULT 0


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

#if COOP_DEBUG_TOOLS
// Initialized so it lands in .data (Overlay 129 is loaded without BSS init).
struct CoopDebugMonBuffer gCoopDebugMonBuffer = { .party_count = 0xFF };
#endif

// ---------------------------------------------------------------------------
// Text helpers
// ---------------------------------------------------------------------------

// Converts a plain ASCII message into an HG-encoded String (charmap.txt).
// Supports A-Z a-z 0-9, space, ! ? , . - ' and \n; "..." becomes the
// ellipsis glyph. Anything else is rendered as '?'. '@' inserts name (an
// HG-encoded player name from the relay, or NULL): up to 8 chars, ending at
// 0xFFFF; control and format codes (0x200 and up) become '?'.
#define COOP_ARG_CHARS 11 // Pokemon nicknames and species names: 10 + terminator

void LONG_CALL Coop_SetStringFromAscii(String *dst, const char *src, const u16 *const *args)
{
    u16 buf[COOP_TEXT_MAX + 1];
    u32 n = 0;
    u32 i;

    while (*src != '\0' && n < COOP_TEXT_MAX)
    {
        char c = *src++;
        u16 out;
        const u16 *arg;
        int argIdx = c == '@' ? 0 : c == '$' ? 1 : c == '&' ? 2 : c == '^' ? 3 : -1;

        if (argIdx >= 0 && args != NULL && (arg = args[argIdx]) != NULL)
        {
            for (i = 0; i < COOP_ARG_CHARS && arg[i] != 0xFFFF && n < COOP_TEXT_MAX; i++)
                buf[n++] = (arg[i] != 0 && arg[i] < 0x200) ? arg[i] : 0x01AC;
            continue;
        }
        if ((u8)c == 0xC3 && (u8)*src == 0xA9) // UTF-8 é
        {
            src++;
            buf[n++] = 0x0188;
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
            case '(':  out = 0x01B9; break;
            case ':':  out = 0x01C4; break;
            case ')':  out = 0x01BA; break;
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

// The virtual radio, in and out of the field (src/coop_link.c). On the
// VWait queue: the end of each main-loop pass, right before the next
// Comm_Update, so what the bridge wrote during the pass reaches the comm
// core without waiting another pass (the main queue runs after
// Comm_Update, and not at all while Comm_Update waits). Link latency,
// docs/GAME_REBALANCE_UPDATE.md §25.
void Coop_LinkSysTask(struct SysTask *task, void *param)
{
    (void)task;
    (void)param;
    Coop_LinkUpdate();
}

void Coop_MainSysTask(struct SysTask *task, void *param)
{
    FieldSystem *fsys = gFieldSysPtr;
    BOOL fieldActive;
    BOOL connected;

    (void)task;
    (void)param;

    gCoopCommBlock.heartbeat_rom++;
    Coop_UpdateBridgeWatchdog();
    Coop_LinkBattleWatchdog(fsys);

    fieldActive = Coop_IsFieldMapActive(fsys);
    connected = Coop_IsConnectedToRoom();
    if (!fieldActive)
        gCoopBoss.talkObj = NULL; // set again by the next field frame (Coop_UpdateBossWatch)

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

            // The lobby and boss blocks and the adventure-start save are
            // field tasks: Overlay 131 starts them (Coop_UpdateFieldConnection).
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
        // Deferred through battles, apps, scripts, menus and mid-step frames:
        // Overlay 131 saves and resets in the next safe field frame.
    case COOP_DISCONNECT_RUNNING:
    default:
        break;
    }
}
