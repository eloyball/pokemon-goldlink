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
