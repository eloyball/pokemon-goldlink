#ifndef COOP_DIALOG_H
#define COOP_DIALOG_H

#include "types.h"
#include "message.h"
#include "window.h"

struct FieldSystem;

// Vanilla overworld dialog box (pret ScrCmd_OpenMsg / DialogBox_LoadFrame) on
// BG3, implemented in Overlay 129 (src/coop_sync.c) and shared with the field
// code in Overlay 131. Kept out of coop_sync.h so that header stays free of
// window and string types.
struct CoopDialog
{
    GF_BGL_BMPWIN window;
    String *text;
    const u16 *name; // what '@' prints (Coop_SetStringFromAscii), or NULL
    u8 printerId;
    u8 textDelay;
};

#define COOP_TEXT_MAX 96

// Plain ASCII to an HG-encoded String. Supports A-Z a-z 0-9, space,
// ! ? , . - ' and \n; "..." becomes the ellipsis glyph; anything else prints
// '?'. '@' inserts name (HG-encoded, up to 8 chars); with name NULL it prints '?'.
void LONG_CALL Coop_SetStringFromAscii(String *dst, const char *src, const u16 *name);

// Opens the box and sets textbox_open; Close undoes both. Print shows one
// page (two lines max), at the player's text speed, faster while A or B is held.
void LONG_CALL Coop_DialogOpen(struct FieldSystem *fsys, struct CoopDialog *dialog);
void LONG_CALL Coop_DialogPrint(struct CoopDialog *dialog, const char *msg);
BOOL LONG_CALL Coop_DialogIsPrinting(const struct CoopDialog *dialog);
void LONG_CALL Coop_DialogClose(struct FieldSystem *fsys, struct CoopDialog *dialog);

#endif // COOP_DIALOG_H
