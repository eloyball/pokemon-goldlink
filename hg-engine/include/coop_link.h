#ifndef COOP_LINK_H
#define COOP_LINK_H

#include "types.h"

// Virtual link (src/coop_link.c, Overlay 129; docs/MILESTONE3_PLAN.md §15).
// Delivers the virtual radio's callbacks and messages; once a frame, from the
// co-op SysTask, in and out of the field.
void LONG_CALL Coop_LinkUpdate(void);

// Ends a co-op link battle whose link was lost (gCoopCommBlock.
// track_link_battle); once a frame, from the co-op SysTask.
struct FieldSystem;
void LONG_CALL Coop_LinkBattleWatchdog(struct FieldSystem *fsys);

// The parent's children in the running link battle (1; 3 in a team battle):
// fewer means the link is lost.
extern u8 gCoopLinkBattleChildren;

// TRUE when the running link battle's link is lost (src/coop_link.c).
BOOL LONG_CALL Coop_LinkBattleIsLost(void);

// The battle extension's header (Overlay 130, src/battle/coop_link_battle.c),
// first in the overlay: what resident code calls while the battle app runs.
// The field extension and the Pokegear's share the address: check the app
// and the magic first.
#define COOP_BATTLE_EXT_MAGIC 0x31544243 // "CBT1"

struct CoopBattleExt
{
    u32 magic;
    void (*linkWatchdog)(u8 *app); // every frame of a co-op link battle's app
};

#define COOP_BATTLE_EXT ((const struct CoopBattleExt *)0x023C4000)

#endif
