#ifndef COOP_MARTS_H
#define COOP_MARTS_H

#include "types.h"

// Badge-gated department store counters (docs/GAME_REBALANCE_UPDATE.md §8.2),
// Overlay 131. The four lists are hg-engine's repointed special marts
// (src/field/mart.c), rebuilt by Coop_RefreshGatedMarts while the player is on
// their floor. COOP_MART_SLOTS includes the 0xFFFF terminator.
#define COOP_MART_SLOTS 80

extern u16 sGoldenrodDepartment4F[COOP_MART_SLOTS];
extern u16 sGoldenrodDepartment5F[COOP_MART_SLOTS];
extern u16 sCeladonDepartment3F[COOP_MART_SLOTS];
extern u16 sCeladonDepartment4F[COOP_MART_SLOTS];

struct FieldSystem;
void Coop_RefreshGatedMarts(struct FieldSystem *fsys);
// One floor's list, as if the player stood there (map: MAP_T25R1004 / 1005, MAP_T07R0103 / 0104).
void Coop_RefreshGatedMart(struct FieldSystem *fsys, u32 map);

#endif
