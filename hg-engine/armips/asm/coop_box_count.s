// Co-op (docs/KNOWN_RISKS.md #60): hg-engine has 30 PC boxes, and its
// PCStorage_FindFirstBoxWithEmptySlot returns 30 (NUM_PC_BOXES) when every box
// is full. Three vanilla callers still compare with 18 (vanilla's box count):
// they misfire when boxes 1-18 are full but a later one has room, and miss
// the real "all full". Each `cmp r0, #0x12` after the call becomes 30.
// Addresses from pret's asm before decompilation (labels are addresses).

.nds
.thumb

// The battle bag: a ball with a full party and no box room ("The Box is full").
// pret asm/overlay_08.s, ov08_02222B8C (ccd BattleBagMenu_TryUseItem).
.if readu16("base/overlay/overlay_0008.bin", 0x02222D48 - 0x0221BE20) != 0x2812 && readu16("base/overlay/overlay_0008.bin", 0x02222D48 - 0x0221BE20) != 0x281E
    .error "coop_box_count.s: base/overlay/overlay_0008.bin 0x02222D48 is not cmp r0, #0x12 (or #30): the patch is out of date"
.endif
.open "base/overlay/overlay_0008.bin", 0x0221BE20
.org 0x02222D48
cmp r0, #30
.close

// The Safari Zone ends when no catch can be stored (battle script command).
// pret 548c1c4eb^ asm/overlay_12_battle_command.s, ov12_022452EC (ccd BtlCmd_CheckSafariGameDone).
.if readu16("base/overlay/overlay_0012.bin", 0x02245314 - 0x022378C0) != 0x2812 && readu16("base/overlay/overlay_0012.bin", 0x02245314 - 0x022378C0) != 0x281E
    .error "coop_box_count.s: base/overlay/overlay_0012.bin 0x02245314 is not cmp r0, #0x12 (or #30): the patch is out of date"
.endif
.open "base/overlay/overlay_0012.bin", 0x022378C0
.org 0x02245314
cmp r0, #30
.close

// After a Safari battle: "no room" script (std_safari_storage_out).
// pret 50542bb40^ asm/unk_02050660.s (ccd encounter.c, the encounter task's end).
.if readu16("base/arm9.bin", 0x02050E7C - 0x02000000) != 0x2812 && readu16("base/arm9.bin", 0x02050E7C - 0x02000000) != 0x281E
    .error "coop_box_count.s: base/arm9.bin 0x02050E7C is not cmp r0, #0x12 (or #30): the patch is out of date"
.endif
.open "base/arm9.bin", 0x02000000
.org 0x02050E7C
cmp r0, #30
.close
