#ifndef COOP_CHOICE_H
#define COOP_CHOICE_H

#include "types.h"
#include "message.h"
#include "window.h"

struct FieldSystem;

// A vertical choice list in its own framed window, drawn like vanilla script
// multichoice / yes-no menus (ccd src/field/script_menu.c): BG3, system font,
// '‣' cursor, anchored bottom-right just above the message box. Overlay 131
// (src/field/coop_choice.c). B picks the last row (Cancel / No).
#define COOP_CHOICE_MAX    7 // rows: a full party plus Cancel
#define COOP_CHOICE_TEXT_X 12 // pixels: the cursor glyph sits left of the text
#define COOP_CHOICE_NONE   0  // Coop_ChoiceInput: still choosing (row values must not be 0)

struct CoopChoice
{
    GF_BGL_BMPWIN window;
    String *text;
    u8 values[COOP_CHOICE_MAX];
    u8 count;
    u8 cursor;
    u8 open;
};

// Two steps for custom rows: open the empty framed window, print each row,
// then Coop_ChoiceShow (cursor + copy to VRAM).
void Coop_ChoiceOpenWindow(struct FieldSystem *fsys, struct CoopChoice *choice, const u8 *values, u32 count, u32 widthTiles);
void Coop_ChoicePrint(struct CoopChoice *choice, u32 row, u32 x, const char *text, const u16 *const *args);
void Coop_ChoiceShow(struct CoopChoice *choice);

// Plain labels in one call.
void Coop_ChoiceOpen(struct FieldSystem *fsys, struct CoopChoice *choice, const char *const *labels, const u8 *values,
                     u32 count, u32 widthTiles);
// YES / NO; the values are COOP_CHOICE_YES / COOP_CHOICE_NO.
#define COOP_CHOICE_YES 0xFE
#define COOP_CHOICE_NO  0xFF
void Coop_ChoiceOpenYesNo(struct FieldSystem *fsys, struct CoopChoice *choice);

// The chosen row's value, or COOP_CHOICE_NONE while choosing.
u32 Coop_ChoiceInput(struct CoopChoice *choice);
// Safe to call when not open.
void Coop_ChoiceClose(struct CoopChoice *choice);

#endif // COOP_CHOICE_H
