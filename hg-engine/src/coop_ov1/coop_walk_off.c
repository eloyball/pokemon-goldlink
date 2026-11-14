#include "config.h"
#include "types.h"
#include "script.h"
#include "map_events_internal.h"
#include "pokemon.h"

// The wider picture (include/coop_sync.h wide_side), Overlay 1's unused space
// (src/coop_ov1/linker.ld).
//
// A cutscene walks a person off the DS's screen, then removes them (the script command
// HidePerson, ccd src/scrcmd_c.c ScrCmd_HidePerson: GetMapObjectByID, then DeleteMapObject). With
// the client's side strips the person is still in view there, and vanished. This is HidePerson
// for every script (routinepointers: its entry in the command table): a person still in a side
// strip, facing out of the picture, first walks on (their last step's kind and speed, else a
// normal walk) until past the strip, and turns invisible; the script waits for it, as
// WaitMovement does, and runs HidePerson again, which removes them as before. Flags, objects and
// the script's path end as in the game; the scene lasts a moment longer, the same for every
// player (the widest strip, whatever this player's window shows or whether it's on). Anyone else
// (in the DS's picture, past the strip, facing elsewhere, already invisible) is removed at once,
// as before.

#define WALK_OFF_DS_HALF     128 // the DS's picture, each side of the player (the camera's centre)
#define WALK_OFF_TILE        16
#define WALK_OFF_STRIP_MAX   64  // GL_CORE_MAX_WIDE_SIDE: the widest strip any player has
#define WALK_OFF_DIR_EAST    3   // the facing directions north, south, west, east: 0..3
#define WALK_OFF_NORMAL_WALK 12  // pret asm/macros/movement.inc: WalkNormalNorth; + direction
#define WALK_OFF_INVISIBLE   69  // SetInvisible
#define WALK_OFF_END         254 // EndMovement

// Called with a plain bl (Overlay 1 is within its reach of arm9; asm/coop_ov1/entry.s): the
// headers' LONG_CALL declarations cost a literal and a register each, which this space can't
// spare. rom.ld's functions; pret 152fe95ec asm/scrcmd.s ScrCmd_HidePerson's calls and labels.
u16 WalkOff_VarGet(FieldSystem *fsys, u16 var);
LocalMapObject *WalkOff_GetMapObjectByID(void *manager, int id);
void WalkOff_DeleteMapObject(LocalMapObject *object);
void *WalkOff_FieldSysGetAttrAddr(FieldSystem *fsys, int field);
// The list is pret's MovementScriptCommand: { u16 command, length }, here one word each.
void *WalkOff_EventObjectMovementMan_Create(LocalMapObject *object, const u32 *cmds);
void WalkOff__ScheduleObjectEventMovement(FieldSystem *fsys, void *movementMan, void *cmds);
BOOL WalkOff_ScrCmd_WaitMovement(SCRIPTCONTEXT *ctx);

BOOL CoopOv1_HidePerson(SCRIPTCONTEXT *ctx)
{
    FieldSystem *fsys = ctx->fsys;
    const u8 *arg = ctx->script_ptr;
    LocalMapObject *object;
    s32 dx, side;
    u32 facing, command;
    // The walk's list, in the context's scratch words (a wait's; free while a command runs): the
    // script waits on it right here, and the movement is over before the script goes on.
    u32 *cmds = ctx->data;

    ctx->script_ptr = arg + 2;
    object = WalkOff_GetMapObjectByID(fsys->mapObjectMan, WalkOff_VarGet(fsys, arg[0] | (arg[1] << 8)));
    if (object == NULL)
        return FALSE;
    facing = object->curFacing;
    dx = (object->xCurr - fsys->playerAvatar->mapObject->xCurr) * WALK_OFF_TILE;
    side = dx >> 31; // -1 on the left
    dx = (dx ^ side) - side;
    // In a strip (some of them past the DS's edge, some of them before the strip's end), facing
    // out of the picture (west on the left, 2, east on the right, 3), still visible.
    if ((u32)(dx - (WALK_OFF_DS_HALF - WALK_OFF_TILE + 1)) >= 2 * WALK_OFF_TILE + WALK_OFF_STRIP_MAX - 1
        || facing != (u32)(WALK_OFF_DIR_EAST + side) || (object->flags & BIT_VANISH))
    {
        WalkOff_DeleteMapObject(object);
        return FALSE;
    }
    // Their last step's speed (WalkSlower..WalkFaster, 4..23, the direction in the low two bits),
    // else a normal walk; whole tiles to past the strip; then invisible; the end.
    command = (u32)object->movementCmd;
    if (command - 4 > 19 || (command & 3) != facing)
        command = WALK_OFF_NORMAL_WALK + facing;
    cmds[0] = command | ((WALK_OFF_DS_HALF + 2 * WALK_OFF_TILE + WALK_OFF_STRIP_MAX - 1 - dx) >> 4 << 16);
    cmds[1] = WALK_OFF_INVISIBLE | (1 << 16);
    cmds[2] = WALK_OFF_END;
    (*(u8 *)WalkOff_FieldSysGetAttrAddr(fsys, SCRIPTENV_NUM_ACTIVE_MOVEMENT))++;
    WalkOff__ScheduleObjectEventMovement(fsys, WalkOff_EventObjectMovementMan_Create(object, cmds), NULL);
    // HidePerson again once every movement is done (its command and argument, 4 bytes back).
    ctx->script_ptr = arg - 2;
    return WalkOff_ScrCmd_WaitMovement(ctx);
}
