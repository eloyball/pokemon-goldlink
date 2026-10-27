#ifndef COOP_BATTLE_H
#define COOP_BATTLE_H

#include "types.h"
#include "task.h"

// 1v1 link battles (docs/MILESTONE3_PLAN.md §17), Overlay 131.

// Turns the running session task into the battle task: the same task slot
// runs the battle with a new env (the caller frees its own afterwards).
// `parent`: the requester hosts the link. `name` is the other player's
// HG-encoded name (up to 8 chars). FALSE (nothing changed) when out of memory.
BOOL LONG_CALL Coop_BattleTakeOver(TaskManager *taskman, const u16 *name, u32 peerSlot, BOOL parent);

// Team bosses (docs/MILESTONE3_PLAN.md §18): the player talked to a team
// boss while teamed (called by the resident talk hook, src/coop_boss.c).
struct LocalMapObject;
void LONG_CALL Coop_BossTalkStart(struct FieldSystem *fsys, struct LocalMapObject *boss, u16 script);

#if COOP_DEBUG_TOOLS
// Link probe: a battle without a session request; both sides use beacon id 1.
BOOL LONG_CALL Coop_DebugStartBattle(struct FieldSystem *fsys, BOOL parent);
#endif

#endif
