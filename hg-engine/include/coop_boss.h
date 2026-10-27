#ifndef COOP_BOSS_H
#define COOP_BOSS_H

#include "types.h"
#include "task.h"

// Team bosses (docs/MILESTONE3_PLAN.md §18).
//
// Overlay 129 (src/coop_boss.c, resident): the two vanilla hooks and the
// battle task, which runs while the overworld (and Overlay 131) is gone.
// Overlay 131 (src/field/coop_battle.c): the waiting, the link session and
// the boss's script.

struct FieldSystem;
struct LocalMapObject;
struct BATTLE_PARAM;

// What the resident hooks need from the field. Set by Overlay 131 each field
// frame; read by the hooks, which run while Overlay 131 is loaded (talking
// and scripts happen in the field).
struct CoopBossState
{
    u32 magic;  // COOP_BOSS_MAGIC: an initialized global lands in .data (Overlay 129 is not zeroed)
    struct LocalMapObject *talkObj; // a team boss the player could talk to now; NULL: none
    // Overlay 131: starts the waiting task instead of the boss's script.
    void (*talkStart)(struct FieldSystem *fsys, struct LocalMapObject *boss, u16 script);
    u8 armed;      // the next trainer battle is the team boss battle
    u8 bossIndex;  // its sGatedBosses index
    u8 fought;     // set by the battle hook: the battle ran
    u8 pad;
    u32 assertCaller; // the last failed GF_ASSERT's return address (Coop_GF_AssertFail)
    u32 assertCount;
};

#define COOP_BOSS_MAGIC 0x53534F42 // "BOSS"

extern struct CoopBossState gCoopBoss;

// Overlay 129: what talking to the object does in vanilla (its script).
void LONG_CALL Coop_StartVanillaTalk(struct FieldSystem *fsys, u16 script, struct LocalMapObject *obj);

// Overlay 131: the boss's party, split across the two enemy seats (seat 1 the
// first half, seat 3 the rest). Called by hg-engine MakeTrainerPokemonParty.
void LONG_CALL Coop_BossSplitParty(struct BATTLE_PARAM *bp, int num);

// Overlay 131, once a field frame: which team boss the player faces (for
// the talk hook), and the waiting task's start.
void LONG_CALL Coop_UpdateBossWatch(struct FieldSystem *fsys);

// Overlay 131: the sGatedBosses index of a boss the player still needs to
// beat (no badge yet), or -1.
int LONG_CALL Coop_FindGatedBoss(struct FieldSystem *fsys, struct LocalMapObject *obj);

#endif
