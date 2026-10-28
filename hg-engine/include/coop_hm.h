#ifndef COOP_HM_H
#define COOP_HM_H

#include "types.h"

// HMs without HM moves (docs/MILESTONE3_PLAN.md §21): as in the newer games,
// an HM's field move works once the player has the HM and its badge, whether
// or not a party Pokemon knows the move.

// The HM moves in the party menu's order (Fly first: the menu is its only
// way in; the others also work from the overworld). Overlay 129.
#define COOP_HM_COUNT 8
extern const u16 gCoopHmMenuMoves[COOP_HM_COUNT];

// Bit i: gCoopHmMenuMoves[i] is unlocked. Overlay 129 (.data), written by
// Overlay 131 every field frame, read by the party menu (where the overworld
// and Overlay 131 are gone).
extern u8 gCoopHmMask;

// Overlay 131: the unlocked HMs (their HM in the bag and its badge), as a
// gCoopHmMask; and whether one move is an unlocked HM.
u32 Coop_HmUnlockedMask(void *saveData);
BOOL Coop_HmUnlocked(void *saveData, u32 move);

#endif
