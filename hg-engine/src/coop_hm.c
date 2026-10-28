#include "config.h"
#include "types.h"

#include "constants/moves.h"

#include "coop_hm.h"

// HMs without HM moves (docs/MILESTONE3_PLAN.md §21), Overlay 129: only what
// the party menu reads (it runs with Overlay 131 gone); the checks live in
// src/field/coop_hm_field.c.

const u16 gCoopHmMenuMoves[COOP_HM_COUNT] = {
    MOVE_FLY, MOVE_SURF, MOVE_STRENGTH, MOVE_CUT, MOVE_ROCK_SMASH, MOVE_WATERFALL, MOVE_WHIRLPOOL, MOVE_ROCK_CLIMB,
};

// Initialized: in .data (Overlay 129 is not zeroed on load).
u8 gCoopHmMask __attribute__((section(".data"))) = 0;
