#include "config.h"
#include "types.h"
#include "script.h"
#include "coop_sync.h"

// The wider picture's walk-off and walk-in (UC11, rewritten for v0.8, UC16): HidePerson (script
// command 101), ShowPerson (100) and MovePersonFacing (339) (routinepointers: their entries in the
// command table) load Overlay 154, run it (src/individual/coop_walk_out.c: the run of HidePersons'
// people in view at a side walk out together, then each is removed; someone appearing in a side
// strip walks in from past it) and unload it, as src/npc_trade.c runs its overlay. The movement
// lists the people walk by stay here, in Overlay 1, while Overlay 154 is gone: their walks last
// several frames after this returns (Overlay 1 is loaded while scripts run).

// Called with a plain bl (asm/coop_ov1/entry.s): the game's overlay functions, hooked by
// hg-engine (hooks: HandleLoadOverlay, UnloadOverlayByID).
u32 WalkOff_HandleLoadOverlay(u32 ovyId, u32 loadType);
void WalkOff_UnloadOverlayByID(u32 ovyId);

typedef BOOL (*CoopWalkOutFunc)(SCRIPTCONTEXT *ctx, u32 *lists, FieldSystem *fsys);

static u32 sWalkOutLists[COOP_WALK_OUT_MAX * 3];

BOOL CoopOv1_ScriptPeople(SCRIPTCONTEXT *ctx)
{
    BOOL wait;

    WalkOff_HandleLoadOverlay(COOP_OVERLAY_WALK_OUT, 2);
    wait = ((CoopWalkOutFunc)(0x023C0400 | 1))(ctx, sWalkOutLists, NULL);
    WalkOff_UnloadOverlayByID(COOP_OVERLAY_WALK_OUT);
    return wait;
}
