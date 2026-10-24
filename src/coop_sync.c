#include "config.h"
#include "types.h"

#include "constants/buttons.h"
#include "constants/pokemon_nickname.h"

#include "message.h"
#include "coop_sync.h"

// 512-byte shared mailbox, placed in the Overlay 129 .data section so
// arm-none-eabi-nm emits it as a defined (type D) symbol inside
// 0x023D8060..0x023E0000. The BizHawk coop-bridge writes the
// is_connected_to_room flag (offset 0x160) here at 60Hz.
__attribute__((section(".data"), used, aligned(4)))
volatile struct CoopCommBlock gCoopCommBlock = {
    .magic = 0x434F4F50,
    .version = 1,
    .assigned_player_id = 0xFF,
    .active_players_mask = 0,
    .is_connected_to_room = 0,
    .is_boot_done = 0,
};

// Black-screen gate message lines, encoded with the HG font charmap
// (include/constants/pokemon_nickname.h).
struct CoopGateLine
{
    const u16 *text;
    u16 y;
};

static const u16 sGateLine1[] = {
    _Y, _o_, _u_, CHAR_SPACE, _a_, _r_, _e_, CHAR_SPACE, _n_, _o_, _t_, CHAR_SPACE,
    _i_, _n_, CHAR_SPACE, _a_, CHAR_SPACE, _r_, _o_, _o_, _m_, _endstr
}; // "You are not in a room"

static const u16 sGateLine2[] = {
    _P, _l_, _e_, _a_, _s_, _e_, CHAR_SPACE, _c_, _r_, _e_, _a_, _t_, _e_, CHAR_SPACE,
    _o_, _r_, CHAR_SPACE, _j_, _o_, _i_, _n_, _endstr
}; // "Please create or join"

static const u16 sGateLine3[] = {
    _a_, CHAR_SPACE, _r_, _o_, _o_, _m_, CHAR_SPACE, _i_, _n_, CHAR_SPACE,
    _C, _o_, CHAR_SPACE, _o_, _p_, CHAR_SPACE, _B, _r_, _i_, _d_, _g_, _e_, _endstr
}; // "a room in Co op Bridge"

static const u16 sGateLine4[] = {
    _b_, _e_, _f_, _o_, _r_, _e_, CHAR_SPACE, _c_, _o_, _n_, _t_, _i_, _n_, _u_, _i_, _n_, _g_, _endstr
}; // "before continuing"

static const u16 sGateLine5[] = {
    _P, _r_, _e_, _s_, _s_, CHAR_SPACE, _A, CHAR_SPACE, _t_, _o_, CHAR_SPACE,
    _r_, _e_, _t_, _u_, _r_, _n_, _endstr
}; // "Press A to return"

static const struct CoopGateLine sGateLines[] = {
    { sGateLine1, 16 },
    { sGateLine2, 36 },
    { sGateLine3, 56 },
    { sGateLine4, 76 },
    { sGateLine5, 104 },
};

BOOL LONG_CALL Coop_IsConnectedToRoom(void)
{
    if (gCoopCommBlock.is_connected_to_room != 0)
    {
        return TRUE;
    }
    if (gCoopCommBlock.assigned_player_id < 6)
    {
        return TRUE;
    }
    return FALSE;
}

// Standalone black-screen gate. Adapted from PrintCrashMessageAndReset
// (src/save.c): same heap/IRQ/BG setup, but with a black mask color and
// a static message instead of NARC msgdata. Blocks on A, then resets.
void LONG_CALL Coop_ShowDisconnectedGateAndReset(void)
{
    u32 window;
    u32 line;

    OS_SetArenaHi(0, OS_GetInitArenaHi(0));
    OS_SetArenaLo(0, OS_GetInitArenaLo(0));
    InitHeapSystem((void *)0x02108528, 1, 1, 0);

    sub_0200FBF4(0, 0);
    sub_0200FBF4(1, 0);

    OS_DisableIrqMask(1);
    OS_SetIrqFunction(1, (void *)(0x2096318 | 1));
    OS_EnableIrqMask(1);

    Main_SetVBlankIntrCB(NULL, NULL);
    Main_SetHBlankIntrCB(NULL, NULL);

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

    void *bg_config = BgConfig_Alloc(0);
    SetBothScreensModesAndDisable((void *)0x02108530);
    InitBgFromTemplate(bg_config, 0, (void *)0x02108540, 0);
    BgClearTilemapBufferAndCommit(bg_config, 0);
    LoadUserFrameGfx1(bg_config, 0, 0x1F7, 2, 0, 0);
    LoadFontPal0(0, 0x20, 0);
    BG_ClearCharDataRange(0, 0x20, 0, 0);

    // Black background (crash screen uses RGB(27, 1, 1))
    BG_SetMaskColor(0, 0x0000);
    BG_SetMaskColor(4, 0x0000);

    ResetAllTextPrinters();

    AddWindow(bg_config, &window, (void *)(0x02108520));
    FillWindowPixelRect(&window, 0xF, 0, 0, 208, 144);
    DrawFrameAndWindow1(&window, FALSE, 0x1F7, 2);

    for (line = 0; line < NELEMS(sGateLines); line++)
    {
        String *msg_str = String_New(64, 0);
        CopyU16ArrayToString(msg_str, sGateLines[line].text);
        AddTextPrinterParameterized(&window, 0, msg_str, 8, sGateLines[line].y, 0, NULL);
        String_Delete(msg_str);
    }

    GfGfx_BothDispOn();
    SetMasterBrightnessNeutral(0);
    SetMasterBrightnessNeutral(1);
    SetBlendBrightness(0, 0x3F, 3);

    // Flush the window and text printers to VRAM and turn on the visible
    // BG plane (same as PrintCrashMessageAndReset in src/save.c).
    sub_02038D90();

    while (TRUE)
    {
        HandleDSLidAction();
        sub_02036144();

        if (sub_02039AA4())
        {
            break;
        }
        OS_WaitIrq(TRUE, 1);
    }

    // The player is almost certainly still holding A/START from the menu
    // selection, so first wait for A to be RELEASED...
    while (TRUE)
    {
        HandleDSLidAction();
        if ((PAD_Read() & PAD_BUTTON_A) == 0)
        {
            break;
        }
        OS_WaitIrq(TRUE, 1);
    }

    // ...then wait for a fresh A PRESS before resetting.
    while (TRUE)
    {
        HandleDSLidAction();
        if (PAD_Read() & PAD_BUTTON_A)
        {
            break;
        }
        OS_WaitIrq(TRUE, 1);
    }

    sub_0200FBF4(0, 0x7FFF);
    sub_0200FBF4(1, 0x7FFF);

    RemoveWindow(&window);
    sys_FreeMemoryEz(bg_config);

    OS_ResetSystem(0);
}

// Startup gate: if the player is not in a coop room, block game start
// with the black-screen message and reset back to the title screen.
void LONG_CALL Coop_CheckStartupGate(void)
{
    if (!Coop_IsConnectedToRoom())
    {
        Coop_ShowDisconnectedGateAndReset();
    }
}
