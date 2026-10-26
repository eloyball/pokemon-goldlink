#include "config.h"
#include "types.h"

#include "constants/buttons.h"

#include "message.h"
#include "pokemon.h"
#include "sound.h"
#include "system.h"
#include "window.h"
#include "coop_dialog.h"
#include "coop_choice.h"

// ---------------------------------------------------------------------------
// Choice window for co-op field menus (Overlay 131): the ghost menu, Yes / No
// prompts and the trade's party list. See include/coop_choice.h.
// ---------------------------------------------------------------------------

// pret ClearFrameAndWindow1 (asm label ClearWindowPixelBuffer).
void LONG_CALL sub_0200E5D4(void *window, BOOL dontCopyToVram);
// Declared in sprite.h (rom.ld), which this file does not otherwise need.
void LONG_CALL AddWindowParameterized(void *bgConfig, void *window, u8 bgId, u8 x, u8 y, u8 width, u8 height, u8 paletteNum, u16 baseTile);

#define COOP_HEAP_ID_FIELD1 4    // pret constants/heap.h
#define COOP_HEAP_ID_FIELD3 32   // field task heap
#define COOP_SE_SELECT      1500 // pret SEQ_SE_DP_SELECT

// Vanilla script multichoice / yes-no (ccd src/field/script_menu.c,
// scrcmd_c.c): palette 13, text tiles from 0x3D, menu frame at 0x3D9 with
// palette 11. A full party list (15 x 14 tiles) ends at 0x10F, below the
// location banner's tiles (0x197) and the message box's (0x237).
#define COOP_CHOICE_BG         3
#define COOP_CHOICE_FONT       0
#define COOP_CHOICE_PLTT       13
#define COOP_CHOICE_BASE_TILE  0x3D
#define COOP_CHOICE_FRAME_TILE 0x3D9
#define COOP_CHOICE_FRAME_PLTT 11
#define COOP_CHOICE_RIGHT      31 // frame column; the window ends one tile before
#define COOP_CHOICE_BOTTOM     17 // frame row; the message frame starts at row 18
#define COOP_CHOICE_FILL       15
#define COOP_CURSOR_GLYPH      0x011F // '‣', vanilla ListMenuCursor (pret _020F60DC)
#define COOP_TEXT_NO_COPY      0xFF   // pret TEXT_SPEED_NOTRANSFER

static void Coop_ChoiceDrawCursor(struct CoopChoice *choice)
{
    static const u16 cursor[] = { COOP_CURSOR_GLYPH, 0xFFFF };

    FillWindowPixelRect(&choice->window, COOP_CHOICE_FILL, 0, 0, COOP_CHOICE_TEXT_X, choice->count * 16);
    CopyU16ArrayToString(choice->text, cursor);
    AddTextPrinterParameterized(&choice->window, COOP_CHOICE_FONT, choice->text, 0, choice->cursor * 16,
                                COOP_TEXT_NO_COPY, NULL);
    CopyWindowToVram(&choice->window);
}

void Coop_ChoiceOpenWindow(FieldSystem *fsys, struct CoopChoice *choice, const u8 *values, u32 count, u32 widthTiles)
{
    u32 i;

    if (count > COOP_CHOICE_MAX)
        count = COOP_CHOICE_MAX;
    choice->count = (u8)count;
    choice->cursor = 0;
    for (i = 0; i < count; i++)
        choice->values[i] = values[i];

    AddWindowParameterized(fsys->bg_config, &choice->window, COOP_CHOICE_BG,
                           (u8)(COOP_CHOICE_RIGHT - widthTiles), (u8)(COOP_CHOICE_BOTTOM - count * 2),
                           (u8)widthTiles, (u8)(count * 2), COOP_CHOICE_PLTT, COOP_CHOICE_BASE_TILE);
    LoadUserFrameGfx1(fsys->bg_config, COOP_CHOICE_BG, COOP_CHOICE_FRAME_TILE, COOP_CHOICE_FRAME_PLTT, 0,
                      COOP_HEAP_ID_FIELD1);
    DrawFrameAndWindow1(&choice->window, TRUE, COOP_CHOICE_FRAME_TILE, COOP_CHOICE_FRAME_PLTT);
    FillWindowPixelBuffer(&choice->window, COOP_CHOICE_FILL);
    choice->text = String_New(COOP_TEXT_MAX, COOP_HEAP_ID_FIELD3);
}

void Coop_ChoicePrint(struct CoopChoice *choice, u32 row, u32 x, const char *text, const u16 *const *args)
{
    Coop_SetStringFromAscii(choice->text, text, args);
    AddTextPrinterParameterized(&choice->window, COOP_CHOICE_FONT, choice->text, x, row * 16, COOP_TEXT_NO_COPY, NULL);
}

void Coop_ChoiceShow(struct CoopChoice *choice)
{
    Coop_ChoiceDrawCursor(choice); // copies the whole window
    choice->open = TRUE;
}

void Coop_ChoiceOpen(FieldSystem *fsys, struct CoopChoice *choice, const char *const *labels, const u8 *values,
                     u32 count, u32 widthTiles)
{
    u32 i;

    Coop_ChoiceOpenWindow(fsys, choice, values, count, widthTiles);
    for (i = 0; i < choice->count; i++)
        Coop_ChoicePrint(choice, i, COOP_CHOICE_TEXT_X, labels[i], NULL);
    Coop_ChoiceShow(choice);
}

void Coop_ChoiceOpenYesNo(FieldSystem *fsys, struct CoopChoice *choice)
{
    static const char *const labels[] = { "YES", "NO" };
    static const u8 values[] = { COOP_CHOICE_YES, COOP_CHOICE_NO };

    Coop_ChoiceOpen(fsys, choice, labels, values, 2, 6);
}

void Coop_ChoiceClose(struct CoopChoice *choice)
{
    if (!choice->open)
        return;
    sub_0200E5D4(&choice->window, FALSE);
    RemoveWindow(&choice->window);
    String_Delete(choice->text);
    choice->text = NULL;
    choice->open = FALSE;
}

u32 Coop_ChoiceInput(struct CoopChoice *choice)
{
    u32 keys = (u32)gSystem.newKeys;

    if (keys & PAD_BUTTON_A)
    {
        PlaySE(COOP_SE_SELECT);
        return choice->values[choice->cursor];
    }
    if (keys & PAD_BUTTON_B)
    {
        PlaySE(COOP_SE_SELECT);
        return choice->values[choice->count - 1];
    }
    if (keys & (PAD_KEY_UP | PAD_KEY_DOWN))
    {
        if (keys & PAD_KEY_UP)
            choice->cursor = choice->cursor == 0 ? choice->count - 1 : choice->cursor - 1;
        else
            choice->cursor = choice->cursor + 1 == choice->count ? 0 : choice->cursor + 1;
        PlaySE(COOP_SE_SELECT);
        Coop_ChoiceDrawCursor(choice);
    }
    return COOP_CHOICE_NONE;
}
