@ The wider picture (src/coop_ov1/linker.ld): the entries armips/asm/coop_wide.s branches to from
@ Overlay 1, at fixed addresses.

    .section .coop_ov1_entry, "ax"
    .syntax unified
    .thumb

@ 0x021FFCA8, called instead of GF3dRender_BindModelSet where a map block's model is loaded.
    .global CoopOv1_BindMapModel
    .thumb_func
CoopOv1_BindMapModel:
    b Coop_BindMapModel

@ 0x021FFCAA, called where MapPropManager_LoadFromNARC replaces a building whose model isn't
@ loaded with model 0 (pret 94102e517^ asm/overlay_01_021F3610.s ov01_021F3744, r4 the MapProp):
@ model 0 as before, and the placeholder isn't drawn (MapProp.active, +4).
    .global CoopOv1_HidePlaceholder
    .thumb_func
CoopOv1_HidePlaceholder:
    movs r0, #0
    str r0, [r4]
    str r0, [r4, #4]
    bx lr

@ What src/coop_ov1/coop_walk_off.c calls, as Thumb functions: Overlay 1 is within a bl's reach
@ of arm9, but rom.ld's absolute symbols carry no type, so the linker would branch through an
@ ARM-state veneer, which can't enter Thumb code (and costs 8 bytes each). rom.ld's addresses;
@ src/coop_ov1/linker.ld asserts they are the same (the linker keeps a Thumb symbol without bit 0).
    .global WalkOff_VarGet
    .thumb_set WalkOff_VarGet, 0x020403AC
    .global WalkOff_GetMapObjectByID
    .thumb_set WalkOff_GetMapObjectByID, 0x0205EE60
    .global WalkOff_DeleteMapObject
    .thumb_set WalkOff_DeleteMapObject, 0x0205E400
    .global WalkOff_FieldSysGetAttrAddr
    .thumb_set WalkOff_FieldSysGetAttrAddr, 0x020402F0
    .global WalkOff_EventObjectMovementMan_Create
    .thumb_set WalkOff_EventObjectMovementMan_Create, 0x02062214
    .global WalkOff__ScheduleObjectEventMovement
    .thumb_set WalkOff__ScheduleObjectEventMovement, 0x02041CC4
    .global WalkOff_ScrCmd_WaitMovement
    .thumb_set WalkOff_ScrCmd_WaitMovement, 0x02041C98
