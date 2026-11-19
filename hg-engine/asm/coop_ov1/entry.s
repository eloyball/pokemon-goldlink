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
@ ARM-state veneer, which can't enter Thumb code (and costs 8 bytes each). The game's overlay
@ functions, where hg-engine's hooks branch to its own (hooks: HandleLoadOverlay 02006FF8,
@ UnloadOverlayByID 02006F7C).
    .global WalkOff_HandleLoadOverlay
    .thumb_set WalkOff_HandleLoadOverlay, 0x02006FF8
    .global WalkOff_UnloadOverlayByID
    .thumb_set WalkOff_UnloadOverlayByID, 0x02006F7C
