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

// Fly's landing (KNOWN_RISKS #135, src/field/coop_ghosts.c Coop_FlyLandingKind): hooks: 0002 at
// 0x0224B536 in pret ov02_0224B4AC, the landing's loader, r5 = the landing, r4 = the NARC. The
// jump (10 bytes, unaligned) replaces its gender check and Ethan's branch's first two:
//   ldr r0, [r5, #0xc]; cmp r0, #0; bne 0x0224B57C; add r0, r5, #0; add r0, #0x18
// r0-r3 are free there (each branch sets them). 0: Ethan's branch; 1: Lyra's (0x0224B57C); 2: the
// avatar's image and palette are loaded, so Lyra's branch after those two loads (0x0224B598).
.global Coop_FlyLanding_hook
Coop_FlyLanding_hook:
mov r0, r5
mov r1, r4
bl Coop_FlyLandingKind
ldr r2, =0x0224B57C | 1
cmp r0, #1
beq .Lfly_go
ldr r2, =0x0224B598 | 1
bhi .Lfly_go
add r0, r5, #0
add r0, #0x18
ldr r2, =0x0224B540 | 1
.Lfly_go:
bx r2
.pool
