#ifndef COOP_DIALOG_H
#define COOP_DIALOG_H

#include "types.h"
#include "message.h"
#include "window.h"

struct FieldSystem;

#define COOP_TEXT_MAX  96
#define COOP_TEXT_ARGS 4

// Vanilla overworld dialog box (pret ScrCmd_OpenMsg / DialogBox_LoadFrame) on
// BG3, implemented in Overlay 129 (src/coop_sync.c) and shared with the field
// code in Overlay 131. Kept out of coop_sync.h so that header stays free of
// window and string types.
struct CoopDialog
{
    GF_BGL_BMPWIN window;
    String *text;
    const u16 *args[COOP_TEXT_ARGS]; // what '@' '$' '&' '^' print (Coop_SetStringFromAscii)
    const char *pending; // queued by Coop_DialogPrint while a page is still printing
    u8 printerId;        // 0xFF: nothing printed yet
    u8 textDelay;
};

// Plain ASCII to an HG-encoded String. Supports A-Z a-z 0-9, space,
// ! ? , . - ' ( ) and \n, and UTF-8 'é' (for "Pokémon"); "..." becomes the
// ellipsis glyph; anything else prints '?'. '@' '$' '&' '^' insert args[0..3]
// (HG-encoded, up to 11 chars, ending at 0xFFFF); a NULL arg (or args NULL)
// prints '?'.
void LONG_CALL Coop_SetStringFromAscii(String *dst, const char *src, const u16 *const *args);

// Opens the box and sets textbox_open; Close undoes both, and must only run
// once IsPrinting is FALSE (a printer must never outlive its window). Print
// shows one page (two lines max, about 30 characters each), at the player's
// text speed, faster while A or B is held. Printing while a page is still
// printing queues the new page (the last one wins): a running printer
// cannot be stopped (pret RemoveTextPrinter is unmapped), so IsPrinting
// starts the queued page once the current one is done, and reports TRUE
// until that one is done too.
void LONG_CALL Coop_DialogOpen(struct FieldSystem *fsys, struct CoopDialog *dialog);
void LONG_CALL Coop_DialogPrint(struct CoopDialog *dialog, const char *msg);
BOOL LONG_CALL Coop_DialogIsPrinting(struct CoopDialog *dialog);
void LONG_CALL Coop_DialogClose(struct FieldSystem *fsys, struct CoopDialog *dialog);

#endif // COOP_DIALOG_H
