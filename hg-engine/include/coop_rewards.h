#ifndef COOP_REWARDS_H
#define COOP_REWARDS_H

#include "types.h"

struct FieldSystem;

// Team boss rewards (docs/MILESTONE3_PLAN.md §20), Overlay 131
// (src/field/coop_rewards.c). A team boss battle is a link battle, which
// gives no experience, prize money or level-ups; after a win each game pays
// them to its own save instead, from what the battle recorded in gCoopBoss.

#define COOP_REWARD_PENDING_MAX 12

struct CoopRewards
{
    u32 money;         // prize money added
    u32 exp[6];        // experience added, per party slot
    u8 gainedExp;      // party slot bits that got experience
    u8 leveled;        // party slot bits that gained a level
    u8 learned;        // party slot bits that learned a move
    u8 couldNotLearn;  // party slot bits that had a move to learn and four already
    u8 newLevel[6];    // per slot, when leveled
    u8 evolveChecked;  // party slot bits already offered their evolution
    u8 pad[3];
    // Moves a Pokemon with four moves could not learn, in the order the
    // levels came (2026-11-02): offered afterwards with the tutor's screen.
    u16 pendingMove[COOP_REWARD_PENDING_MAX];
    u8 pendingSlot[COOP_REWARD_PENDING_MAX];
    u8 pendingCount;
    u8 pendingNext;    // the next one to offer
};

// Before a team boss battle: the party's HP and PP, and its lead (what
// changed afterwards counts as having battled).
void Coop_SnapshotPartyForRewards(struct FieldSystem *fsys);

// Pays a won team boss battle's rewards once (gCoopBoss.won, cleared here):
// the prize, and experience for every healthy party member, scaled like
// hg-engine's in-battle experience and shared like the global Exp. Share
// (full for the Pokemon that battled a boss Pokemon, half for the rest).
// Levels are gained one at a time, learning each level's moves into a free
// slot. FALSE when there was nothing to pay.
BOOL Coop_PayBossRewards(struct FieldSystem *fsys, struct CoopRewards *out);

// The next Pokemon that leveled up and now evolves by level: its party slot
// (marked as offered), or -1. *species and *method are for the scene.
int Coop_NextRewardEvolution(struct FieldSystem *fsys, struct CoopRewards *rewards, u16 *species, int *method);

#endif
