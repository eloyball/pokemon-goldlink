#ifndef COOP_BATTLE_H
#define COOP_BATTLE_H

#include "types.h"
#include "task.h"

// Link battles (docs/MILESTONE3_PLAN.md §17, §19), Overlay 131.

// Turns the running session task into the battle task: the same task slot
// runs the battle with a new env (the caller frees its own afterwards).
// `parent`: the requester hosts the link; `peerSlot` is the requester's
// remote slot otherwise. `name` is the other player's (in a team battle the
// requester's, or for the requester the target's) HG-encoded name (up to 8
// chars). `team`: a 2v2 between two teams. FALSE (nothing changed) when out
// of memory.
BOOL LONG_CALL Coop_BattleTakeOver(TaskManager *taskman, const u16 *name, u32 peerSlot, BOOL parent, BOOL team);

// Team bosses (docs/MILESTONE3_PLAN.md §18): the player talked to a team
// boss while teamed (called by the resident talk hook, src/coop_boss.c).
struct LocalMapObject;
void LONG_CALL Coop_BossTalkStart(struct FieldSystem *fsys, struct LocalMapObject *boss, u16 script);

// Custom avatars, Phase 6 (src/field/coop_vs_intro.c): the next battle transition, at
// effect 33, is the PvP intro with this player's and the opponent's (remote slot) VS faces and
// the opponent's name. FALSE: nothing armed (use the battle's own transition).
BOOL Coop_VsIntroArm(u32 opponentSlot, const u16 *name);
#define COOP_VS_INTRO_EFFECT 33

#if COOP_DEBUG_TOOLS
// Link probe: a battle without a session request; both sides use beacon id 1.
BOOL LONG_CALL Coop_DebugStartBattle(struct FieldSystem *fsys, BOOL parent);
#endif

#endif
