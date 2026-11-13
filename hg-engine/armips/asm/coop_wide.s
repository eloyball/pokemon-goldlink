// The wider picture (unified client UC9 §4): the client shows the top screen
// past the DS's 256 pixels, gCoopCommBlock.wide_side (include/coop_sync.h,
// written by the client) more on each side; 0 is the DS's own picture.
//
// The game's sprite culling (pret asm/unk_02025C44.s sub_02025C98, ccd
// src/g2d_cell_culling.c GF_G2dIsCellVisible) skips every sprite whose bounds
// are fully past the DS's edge, so a sprite leaving the screen (a trainer
// sliding out at a battle's start) vanished at the DS's edge, in the middle of
// the side strip. This rewrites it in place, same arithmetic (FX_Mul: 64-bit
// product, + 0x800, >> 12), with the top screen's view wide_side further on
// each side. The touch screen, and the top screen with wide_side 0, are culled
// as in vanilla.
//
// It is the culling of every render surface the OAM manager sets up (its only
// caller: the literal of pret asm/unk_0200B150.s sub_0200B27C, through the
// renderer, which passes &surface->viewRect: ccd lib/NitroSystem/src/
// g2d_renderer.c RndCoreCBFuncBeforeCell_). In ARM code (smull) instead of the
// original Thumb with eight _ll_mul calls, so it fits the original's space.

COOP_WIDE_SIDE_ADDR equ 0x023D8060 + 0x1BF // gCoopCommBlock (src/linker.ld: always first in Overlay 129) .wide_side
COOP_CULL_START equ 0x02025C98
COOP_CULL_END equ 0x02025E88 // the original's end (its last literal at 0x02025E84)

.nds

// The original starts push {r3-r7, lr}; sub sp, #0x60. A rebuild sees this patch (bx pc; nop).
.if readu32("base/arm9.bin", COOP_CULL_START - 0x02000000) != 0xB098B5F8 && readu32("base/arm9.bin", COOP_CULL_START - 0x02000000) != 0x46C04778
    .error "coop_wide.s: base/arm9.bin 0x02025C98 is not sub_02025C98: the patch is out of date"
.endif

.macro coop_fx_mul, dst, a, b // dst = FX_Mul(a, b); uses r12; dst is neither a nor r12
    smull dst, r12, a, b
    adds dst, dst, #0x800
    adc r12, r12, #0
    mov dst, dst, lsr #12
    orr dst, dst, r12, lsl #20
.endmacro

.open "base/arm9.bin", 0x02000000
.org COOP_CULL_START
.thumb
    // Callers come in Thumb (the pointer is 0x02025C99): on to the ARM code.
    bx pc
    nop
.arm
    // r0 cell (NNSG2dCellData), r1 mtx (MtxFx32), r2 rect (NNSG2dViewRect, the surface's head)
    stmfd sp!, {r4-r11, lr}
    mov r10, r1
    mov r11, r2

    // The cell's bounds: x0 r4, x1 r5, y0 r6, y1 r7. cellAttr bit 11: a bounding
    // rectangle follows the cell (maxX, maxY, minX, minY); otherwise a radius,
    // (cellAttr & 0x3F) * 4, on every side.
    ldrh r3, [r0, #2]
    tst r3, #0x800
    beq @@radius
    ldrsh r5, [r0, #8]
    ldrsh r7, [r0, #10]
    ldrsh r4, [r0, #12]
    ldrsh r6, [r0, #14]
    b @@fixed
@@radius:
    and r3, r3, #0x3F
    mov r5, r3, lsl #2
    rsb r4, r5, #0
    mov r7, r5
    mov r6, r4
@@fixed:
    mov r4, r4, lsl #12
    mov r5, r5, lsl #12
    mov r6, r6, lsl #12
    mov r7, r7, lsl #12

    // dx r8 = mtx._20 - rect.x, dy r9 = mtx._21 - rect.y
    ldr r0, [r10, #0x10]
    ldr r1, [r11]
    sub r8, r0, r1
    ldr r0, [r10, #0x14]
    ldr r1, [r11, #4]
    sub r9, r0, r1

    // top r6 = dy + y0 * _01 + y0 * _11; bottom r7 = dy + y1 * _01 + y1 * _11
    ldr r2, [r10, #4]
    ldr r3, [r10, #0xC]
    coop_fx_mul r0, r2, r6
    coop_fx_mul r1, r3, r6
    add r0, r0, r1
    add r6, r9, r0
    coop_fx_mul r0, r2, r7
    coop_fx_mul r1, r3, r7
    add r0, r0, r1
    add r7, r9, r0

    // left r4 = dx + x0 * _00 + x0 * _10; right r5 = dx + x1 * _00 + x1 * _10
    ldr r2, [r10]
    ldr r3, [r10, #8]
    coop_fx_mul r0, r2, r4
    coop_fx_mul r1, r3, r4
    add r0, r0, r1
    add r4, r8, r0
    coop_fx_mul r0, r2, r5
    coop_fx_mul r1, r3, r5
    add r0, r0, r1
    add r5, r8, r0

    // A flipped cell: top <= bottom, left <= right.
    cmp r7, r6
    movlt r0, r7
    movlt r7, r6
    movlt r6, r0
    cmp r5, r4
    movlt r0, r5
    movlt r5, r4
    movlt r4, r0

    // The margin r1: wide_side pixels on the top screen's surface (NNSG2dRenderSurface.type,
    // NNS_G2D_SURFACETYPE_MAIN2D), none elsewhere.
    ldr r1, =COOP_WIDE_SIDE_ADDR
    ldrb r1, [r1]
    ldr r2, [r11, #0x14]
    cmp r2, #1
    movne r1, #0
    mov r1, r1, lsl #12

    // Visible: bottom > 0, top < height, right > -margin, left < width + margin.
    mov r0, #0
    cmp r7, #0
    ble @@done
    ldr r2, [r11, #0xC]
    cmp r6, r2
    bge @@done
    rsb r3, r1, #0
    cmp r5, r3
    ble @@done
    ldr r2, [r11, #8]
    add r2, r2, r1
    cmp r4, r2
    bge @@done
    mov r0, #1
@@done:
    ldmfd sp!, {r4-r11, lr}
    bx lr
.pool
.if . > COOP_CULL_END
    .error "coop_wide.s: the culling outgrew the original's space"
.endif
.close

// A battle's trainer sliding out (pret asm/overlay_12_battle_controller_opponent.s
// ov12_0225D138, 5 pixels a frame) is deleted once its centre reaches x <= -104
// instead of -40: past the widest side strip (64 pixels) with its 80-pixel width.
// "sub r0, #0x2a" (2 - 42 = -40) becomes "sub r0, #0x6a". The same for every
// player (the DS's picture never shows those 13 frames), so battles keep one timing.
.if readu16("base/overlay/overlay_0012.bin", 0x0225D494 - 0x022378C0) != 0x382A && readu16("base/overlay/overlay_0012.bin", 0x0225D494 - 0x022378C0) != 0x386A
    .error "coop_wide.s: base/overlay/overlay_0012.bin 0x0225D494 is not sub r0, #0x2a (or #0x6a): the patch is out of date"
.endif
.open "base/overlay/overlay_0012.bin", 0x022378C0
.org 0x0225D494
.thumb
    sub r0, #0x6a
.close

// Where battle slide-ins start (pret asm/overlay_07.s ov07_022377F4, ccd
// gBattlerPokepicPositions[2][6][3]: x, y, z by battler type; every x is a start
// off the DS's screen): 24 pixels further out, past the side strip (43 pixels),
// so a platform, Pokemon or trainer no longer waits in view before sliding in.
// Every slide (8 pixels a frame) runs to its target, 3 frames longer, the same
// for every player. Enemy side (x < 0) further left, player side further right.
COOP_SLIDE_TABLE equ 0x022377F4
COOP_OV7 equ 0x0221BE20 // Overlay 7's address (base/overarm9.bin)
.macro coop_slide_x, row, vanilla, wide
    .if readu16("base/overlay/overlay_0007.bin", COOP_SLIDE_TABLE + row * 6 - COOP_OV7) != (vanilla & 0xFFFF) && readu16("base/overlay/overlay_0007.bin", COOP_SLIDE_TABLE + row * 6 - COOP_OV7) != (wide & 0xFFFF)
        .error "coop_wide.s: base/overlay/overlay_0007.bin's slide-in table is not vanilla: the patch is out of date"
    .endif
    .org COOP_SLIDE_TABLE + row * 6
    .halfword wide & 0xFFFF
.endmacro
.open "base/overlay/overlay_0007.bin", COOP_OV7
    coop_slide_x 0, 336, 360
    coop_slide_x 1, -80, -104
    coop_slide_x 2, 316, 340
    coop_slide_x 3, -60, -84
    coop_slide_x 4, 356, 380
    coop_slide_x 5, -100, -124
    coop_slide_x 6, -80, -104
    coop_slide_x 7, 336, 360
    coop_slide_x 8, -80, -104
    coop_slide_x 9, 336, 360
    coop_slide_x 10, -80, -104
    coop_slide_x 11, 336, 360
.close

// A neighbouring area's map blocks (src/coop_ov1/coop_neighbour.c): its code, linked as Overlay
// 153 at the address of Overlay 1's unused billboard effect (src/coop_ov1/linker.ld), copied
// there; the two places a map block's model is bound call it instead of GF3dRender_BindModelSet
// (pret asm/overlay_01_021F4704.s ov01_021F6620, the background load task, and ov01_021F67B4,
// the load at once); and a building whose model isn't loaded is no longer drawn as model 0
// (pret 94102e517^ asm/overlay_01_021F3610.s ov01_021F3744, MapPropManager_LoadFromNARC).
COOP_OV1 equ 0x021E5900 // Overlay 1's address (pret asm/overlay_01_021E5900.s)
COOP_OV1_FREE equ 0x021FFCA8
COOP_OV1_BIN equ "build/output_coop_ov1.bin"
.if filesize(COOP_OV1_BIN) > 0x224
    .error "coop_wide.s: build/output_coop_ov1.bin outgrew Overlay 1's unused space"
.endif
// The unused effect's init starts push {r3-r6, lr}; a rebuild sees the copy.
.if readu16("base/overlay/overlay_0001.bin", COOP_OV1_FREE - COOP_OV1) != 0xB578 && readu16("base/overlay/overlay_0001.bin", COOP_OV1_FREE - COOP_OV1) != readu16(COOP_OV1_BIN, 0)
    .error "coop_wide.s: base/overlay/overlay_0001.bin 0x021FFCA8 is not ov01_021FFCA8: the patch is out of date"
.endif
.macro coop_ov1_is_bl, address
    .if (readu16("base/overlay/overlay_0001.bin", address - COOP_OV1) & 0xF800) != 0xF000 || (readu16("base/overlay/overlay_0001.bin", address + 2 - COOP_OV1) & 0xF800) != 0xF800
        .error "coop_wide.s: base/overlay/overlay_0001.bin has no bl where the patch expects one: it is out of date"
    .endif
.endmacro
coop_ov1_is_bl 0x021F66CA
coop_ov1_is_bl 0x021F67D2
// mov r0, #0; str r0, [r4] (or the bl that replaced them).
.if readu32("base/overlay/overlay_0001.bin", 0x021F37CC - COOP_OV1) != 0x60202000
    coop_ov1_is_bl 0x021F37CC
.endif
.open "base/overlay/overlay_0001.bin", COOP_OV1
.org COOP_OV1_FREE
.incbin COOP_OV1_BIN
.thumb
.org 0x021F66CA
    bl 0x021FFCA8 // CoopOv1_BindMapModel
.org 0x021F67D2
    bl 0x021FFCA8
.org 0x021F37CC
    bl 0x021FFCAA // CoopOv1_HidePlaceholder
.close
