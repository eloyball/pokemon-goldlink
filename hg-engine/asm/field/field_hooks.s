.text
.align 2
.thumb

.global DetermineRockSmashItem_hook
DetermineRockSmashItem_hook:
push {r1-r3}
mov r1, r4
ldr r2, =gFieldSysPtr
bl DetermineRockSmashItem
pop {r1-r3}
mov r2, r0 // move result to where it's expected to be

ldr r0, =0x02204E66 | 1
bx r0

// Co-op custom avatars (docs/KNOWN_RISKS.md #82): the field move cut-in's trainer set.
// hooks: 0002 at 0x02249468 in FieldMoveCutIn_Start (pret ov02_02249458), r4 = the cut-in,
// r5-r7 = kind, mon, gender. The jump replaces these four, done here first:
//   str r6, [r4, #0x5c]; strh r7, [r4, #0xc]; ldrh r0, [r4, #0xc]; strh r0, [r4, #0xe]
// The last two (trainerGraphics = playerGender) become Coop_CutInTrainerGraphics
// (src/field/coop_ghosts.c). r0-r3 are free there (vanilla reloads them), lr is saved.
.global Coop_CutInStart_hook
Coop_CutInStart_hook:
str r6, [r4, #0x5c]
strh r7, [r4, #0xc]
mov r0, r4
bl Coop_CutInTrainerGraphics
ldr r1, =0x02249470 | 1
bx r1
.pool

// UC15, roaming spirits (src/field/coop_spirit.c, Coop_SpiritFilterInput): the hook replaces the
// first four instructions of pret FieldInput_Process (ov01_021E6AF4, the label before
// 07b9c4fd0 in asm/overlay_01_021E6880.s), done here around the filter:
//   push {r3-r7, lr}; add r5, r0, #0; ldrh r0, [r5]; add r4, r1, #0
// r0 = the FieldInput, r1 = the FieldSystem; r2 is free (the hook's register).
.global Coop_FieldInputProcess_hook
Coop_FieldInputProcess_hook:
push {r3-r7, lr}
add r5, r0, #0
add r4, r1, #0
bl Coop_SpiritFilterInput
ldrh r0, [r5]
ldr r2, =0x021E6AFC | 1
bx r2
.pool
