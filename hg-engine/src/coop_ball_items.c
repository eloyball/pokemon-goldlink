#include "config.h"
#include "types.h"
#include "script.h"
#include "coop_sync.h"

// ---------------------------------------------------------------------------
// Milestone 6 §5.1: random items. Every item ball (scripts 7000-7255, pret
// scr_seq_0141.s) sets 0x8008 to its item and 0x8009 to the count, then jumps
// to a shared script that copies them into 0x8004 / 0x8005 before
// HasSpaceForItem, GiveItem and the "found" message. Replacing the copied item
// renames everything after it.
//
// Script command CopyVar (pret ScrCmd_CopyVar, 0x02040F38) is replaced
// through `hooks`. It is resident: scripts run wherever the field runs, but
// the item-ball decision is field code in Overlay 131, reached through its
// pinned header (Overlay 129 links before it).
// ---------------------------------------------------------------------------

#define COOP_VAR_BALL_ITEM_OUT 0x8004 // VAR_SPECIAL_x8004
#define COOP_VAR_BALL_ITEM     0x8008 // VAR_SPECIAL_x8008

static u16 Coop_ScriptReadU16(SCRIPTCONTEXT *ctx)
{
    u16 lo = *ctx->script_ptr++;
    u16 hi = *ctx->script_ptr++;

    return (u16)(lo | (hi << 8));
}

BOOL LONG_CALL Coop_ScrCmd_CopyVar(SCRIPTCONTEXT *ctx)
{
    u16 dest = Coop_ScriptReadU16(ctx);
    u16 src = Coop_ScriptReadU16(ctx);
    u16 *destVar = GetVarPointer(ctx->fsys, dest);
    u16 *srcVar = GetVarPointer(ctx->fsys, src);
    const struct CoopFieldEntryHeader *header;

    *destVar = *srcVar;
    if (dest == COOP_VAR_BALL_ITEM_OUT && src == COOP_VAR_BALL_ITEM)
    {
        header = Coop_FieldHeader();
        if (header != NULL && header->ballItem != NULL)
            *destVar = (u16)header->ballItem(ctx->fsys, *destVar);
    }
    return FALSE;
}
