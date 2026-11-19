#include "../../include/config.h"
#include "../../include/types.h"
#include "../../include/map_events_internal.h"
#include "../../include/script.h"
#include "../../include/coop_sync.h"

// ---------------------------------------------------------------------------
// The wider picture's walk-off, v0.8 (UC16, dev/docs/unified-client/UC16_WIDE_NEIGHBOURS.md):
// Overlay 154, loaded only while the script command HidePerson (101) runs (Overlay 1's stub,
// src/coop_ov1/coop_walk_off.c, loads it, calls this and unloads it, as src/npc_trade.c does
// for its overlay; the field doesn't use 0x023C0400 otherwise).
//
// A cutscene removes the people it walked off the DS's screen; in GoldLink's wider picture they
// are still in view there. v0.7 walked each on alone, one HidePerson at a time (the next stood
// still until the one before was gone), from 8 tiles of the player only: Lyra's Marill after the
// catching tutorial stood, then vanished 7 tiles away; Lyra and Marill passed through each other.
// Now the first HidePerson of a run (the HidePersons that follow it, with SetFlag / ClearFlag
// between) starts everyone of the run who is in view at the side and faces out of the picture
// walking at once: 6 to 12 tiles from the camera (the scene's own camera, not the player), on to
// 13, then invisible. The script waits for them all (as WaitMovement), then runs this HidePerson
// again, and each HidePerson of the run removes its person as the game does. Anyone else (in the
// DS's middle, facing elsewhere, entering a door, already invisible) is removed at once, as before.
//
// The other way in (UC16's option B): ShowPerson (100) creating someone, or MovePersonFacing (339)
// moving someone, inside a side strip (8 to 12 tiles from the camera: the DS doesn't show them, the
// wider picture does) put them in view out of nowhere (the rival at Cherrygrove's exit, the Rocket
// grunts in their headquarters). Now they're placed on the same row past the wider picture and walk
// in to the spot, facing as the script asked, and the script waits for it.
// ---------------------------------------------------------------------------

#define WALK_OUT_FROM        6   // tiles from the camera: nearer, the DS shows them well inside
#define WALK_OUT_TO          13  // the widest strip ends at 11 tiles; a sprite's half past it
#define WALK_OUT_ROWS        9   // rows above or below the camera still in view
#define WALK_OUT_DIR_WEST    2   // the facing directions north, south, west, east: 0..3
#define WALK_OUT_NORMAL_WALK 12  // pret asm/macros/movement.inc: WalkNormalNorth; + direction
#define WALK_OUT_INVISIBLE   69  // SetInvisible
#define WALK_OUT_END         254 // EndMovement
#define SCRCMD_SET_FLAG      30
#define SCRCMD_CLEAR_FLAG    31
#define SCRCMD_SHOW_PERSON   100
#define SCRCMD_HIDE_PERSON   101
#define SCRCMD_MOVE_PERSON   339 // pret MovePersonFacing (old name ScrCmd_MovePerson)
#define WALK_IN_FROM         8   // tiles from the camera: the DS shows nearer ones itself
#define WALK_IN_FACE         0   // FaceNorth; + direction

u16 LONG_CALL VarGet(FieldSystem *fsys, u16 var);
LocalMapObject *LONG_CALL GetMapObjectByID(void *manager, int id);
void LONG_CALL DeleteMapObject(LocalMapObject *object);
void *LONG_CALL FieldSysGetAttrAddr(FieldSystem *fsys, int field);
// pret MovementScriptCommand: { u16 command, length }, here one word each.
void *LONG_CALL EventObjectMovementMan_Create(LocalMapObject *object, const u32 *cmds);
void LONG_CALL _ScheduleObjectEventMovement(FieldSystem *fsys, void *movementMan, void *cmds);
BOOL LONG_CALL ScrCmd_WaitMovement(SCRIPTCONTEXT *ctx);
// pret 152fe95ec asm/scrcmd.s, asm/map_object.s (the labels carry the addresses).
#define VANILLA_SHOW_PERSON         ((BOOL (*)(SCRIPTCONTEXT *))(0x02041FB8 | 1))
#define VANILLA_MOVE_PERSON_FACING  ((BOOL (*)(SCRIPTCONTEXT *))(0x0204473C | 1))
#define MapObject_SetPositionFromXYZAndDirection ((void (*)(LocalMapObject *, u32, u32, u32, u32))(0x0205FC2C | 1))
#define MapObject_SyncPosition      ((void (*)(LocalMapObject *))(0x02061070 | 1))

// pret include/camera.h Camera: the point it looks at (after CameraPerspective, 20 bytes, and
// CameraLookAt.camPos).
struct CoopCamera
{
    u8 pad[0x20];
    s32 target[3]; // fx32
};
_Static_assert(__builtin_offsetof(struct CoopCamera, target) == 0x20, "pret Camera.lookAt.camTarget");
// pret include/field_system.h FieldSystem.camera (hg-engine's struct keeps it in unk24).
#define FSYS_CAMERA_OFFSET 0x24

static u16 HalfwordAt(const u8 *p)
{
    return (u16)(p[0] | (p[1] << 8));
}

// The camera's tile (the field camera follows the player; a scene may move it elsewhere).
static void CameraTile(FieldSystem *fsys, LocalMapObject *player, s32 *x, s32 *z)
{
    const struct CoopCamera *camera;

    *x = player->xCurr;
    *z = player->yCurr;
    __builtin_memcpy(&camera, (const u8 *)fsys + FSYS_CAMERA_OFFSET, sizeof(camera));
    if ((u32)camera < 0x02000000 || (u32)camera >= 0x02400000)
        return;
    *x = camera->target[0] >> 16; // fx32 world units, 16 to a tile
    *z = camera->target[2] >> 16;
}

// Someone appearing at (x, z) in a side strip: from the same row past the wider picture, a walk
// in to there, then facing `facing`; the script waits for it. FALSE (nothing done) elsewhere.
static BOOL WalkIn(SCRIPTCONTEXT *ctx, LocalMapObject *o, s32 x, s32 y, s32 z, u32 facing, u32 *lists)
{
    FieldSystem *fsys = ctx->fsys;
    s32 camX, camZ, dx, dz, far;
    u32 toward;

    if (o == NULL || (o->flags & BIT_VANISH))
        return FALSE;
    CameraTile(fsys, fsys->playerAvatar->mapObject, &camX, &camZ);
    dx = x - camX;
    dz = z - camZ;
    far = dx < 0 ? -dx : dx;
    if (far < WALK_IN_FROM || far >= WALK_OUT_TO || dz < -WALK_OUT_ROWS || dz > WALK_OUT_ROWS)
        return FALSE;
    toward = dx < 0 ? WALK_OUT_DIR_WEST + 1 : WALK_OUT_DIR_WEST; // east from the left, west from the right
    MapObject_SetPositionFromXYZAndDirection(o, (u32)(dx < 0 ? camX - WALK_OUT_TO : camX + WALK_OUT_TO), (u32)y,
                                             (u32)z, toward);
    MapObject_SyncPosition(o);
    lists[0] = (WALK_OUT_NORMAL_WALK + toward) | ((u32)(WALK_OUT_TO - far) << 16);
    lists[1] = (WALK_IN_FACE + (facing & 3)) | (1 << 16);
    lists[2] = WALK_OUT_END;
    (*(u8 *)FieldSysGetAttrAddr(fsys, SCRIPTENV_NUM_ACTIVE_MOVEMENT))++;
    _ScheduleObjectEventMovement(fsys, EventObjectMovementMan_Create(o, lists), NULL);
    return TRUE;
}

static BOOL HidePerson(SCRIPTCONTEXT *ctx, u32 *lists);

// Overlay 1's stub calls this for HidePerson, ShowPerson and MovePersonFacing (the command just
// read: the halfword before the arguments).
BOOL __attribute__((section(".init"))) CoopWalkOut_Command(SCRIPTCONTEXT *ctx, u32 *lists)
{
    FieldSystem *fsys = ctx->fsys;
    const u8 *arg = ctx->script_ptr;
    u16 command = HalfwordAt(arg - 2);

    if (command == SCRCMD_SHOW_PERSON)
    {
        u16 id = VarGet(fsys, HalfwordAt(arg));
        LocalMapObject *o;
        BOOL wait = VANILLA_SHOW_PERSON(ctx);

        o = GetMapObjectByID(fsys->mapObjectMan, id);
        if (o != NULL && WalkIn(ctx, o, o->xCurr, o->hCurr, o->yCurr, (u32)o->curFacing, lists))
            return ScrCmd_WaitMovement(ctx);
        return wait;
    }
    if (command == SCRCMD_MOVE_PERSON)
    {
        u16 id = VarGet(fsys, HalfwordAt(arg));
        s32 x = VarGet(fsys, HalfwordAt(arg + 2));
        s32 y = VarGet(fsys, HalfwordAt(arg + 4));
        s32 z = VarGet(fsys, HalfwordAt(arg + 6));
        u32 facing = VarGet(fsys, HalfwordAt(arg + 8));
        LocalMapObject *o = GetMapObjectByID(fsys->mapObjectMan, id);

        if (WalkIn(ctx, o, x, y, z, facing, lists))
        {
            ctx->script_ptr = arg + 10;
            return ScrCmd_WaitMovement(ctx);
        }
        return VANILLA_MOVE_PERSON_FACING(ctx);
    }
    return HidePerson(ctx, lists);
}

static BOOL HidePerson(SCRIPTCONTEXT *ctx, u32 *lists)
{
    FieldSystem *fsys = ctx->fsys;
    const u8 *arg = ctx->script_ptr;
    const u8 *p;
    LocalMapObject *object;
    s32 camX, camZ;
    u32 walking = 0, run = 0;
    LocalMapObject *who[COOP_WALK_OUT_MAX];
    s32 steps[2] = {0, 0}; // west, east

    ctx->script_ptr = arg + 2;
    object = GetMapObjectByID(fsys->mapObjectMan, VarGet(fsys, HalfwordAt(arg)));
    if (object == NULL)
        return FALSE;
    // Walked off already (this run's second pass), or hidden: removed now.
    if (object->flags & BIT_VANISH)
    {
        DeleteMapObject(object);
        return FALSE;
    }
    CameraTile(fsys, fsys->playerAvatar->mapObject, &camX, &camZ);
    // This HidePerson and the run after it: each of its people in view at a side, facing out.
    for (p = arg; run < COOP_WALK_OUT_MAX; run++)
    {
        LocalMapObject *o = GetMapObjectByID(fsys->mapObjectMan, VarGet(fsys, HalfwordAt(p)));

        if (o != NULL && !(o->flags & BIT_VANISH))
        {
            s32 dx = o->xCurr - camX, dz = o->yCurr - camZ;
            u32 side = dx < 0 ? WALK_OUT_DIR_WEST : WALK_OUT_DIR_WEST + 1;
            s32 far = dx < 0 ? -dx : dx;

            if (far >= WALK_OUT_FROM && far < WALK_OUT_TO && dz >= -WALK_OUT_ROWS && dz <= WALK_OUT_ROWS
                && (u32)o->curFacing == side)
            {
                who[walking] = o;
                if (WALK_OUT_TO - far > steps[side & 1])
                    steps[side & 1] = WALK_OUT_TO - far;
                walking++;
            }
        }
        // The next command: another HidePerson of the run, past flags set or cleared.
        p += 2;
        while (HalfwordAt(p) == SCRCMD_SET_FLAG || HalfwordAt(p) == SCRCMD_CLEAR_FLAG)
            p += 4;
        if (HalfwordAt(p) != SCRCMD_HIDE_PERSON)
            break;
        p += 2;
    }
    // All of a side walk as many steps as its nearest needs to leave the picture: they keep
    // their spacing (each to 13 tiles, Lance and Dragonite ended on one tile).
    for (run = 0; run < walking; run++)
    {
        LocalMapObject *o = who[run];
        u32 side = (u32)o->curFacing;
        // Their last step's speed (WalkSlower..WalkFaster, 4..23), else a normal walk.
        u32 command = (u32)o->movementCmd;
        u32 *cmds = &lists[run * 3];

        if (command - 4 > 19)
            command = WALK_OUT_NORMAL_WALK;
        command = (command & ~3u) | side;
        cmds[0] = command | ((u32)steps[side & 1] << 16);
        cmds[1] = WALK_OUT_INVISIBLE | (1 << 16);
        cmds[2] = WALK_OUT_END;
        (*(u8 *)FieldSysGetAttrAddr(fsys, SCRIPTENV_NUM_ACTIVE_MOVEMENT))++;
        _ScheduleObjectEventMovement(fsys, EventObjectMovementMan_Create(o, cmds), NULL);
    }
    if (walking == 0)
    {
        DeleteMapObject(object);
        return FALSE;
    }
    // This HidePerson again once every movement is done (its command and argument, 4 bytes back).
    ctx->script_ptr = arg - 2;
    return ScrCmd_WaitMovement(ctx);
}
