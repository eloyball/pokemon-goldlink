#ifndef COOP_WILD_H
#define COOP_WILD_H

#include "types.h"

// Kanto wild levels (docs/GAME_REBALANCE_UPDATE.md §3.3), Overlay 131: a wild
// Pokemon's level, plus 2 per Kanto badge on a Kanto map. Called by
// hg-engine's wild-creation hooks (asm/other_hook.s).
u32 Coop_WildLevelAdjust(u32 level);

// Kanto maps whose ordinary trainers scale the same way (Coop_ScaleBossParty).
BOOL Coop_IsKantoTrainerMap(u32 map);

#endif
