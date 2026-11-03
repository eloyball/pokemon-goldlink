#include "config.h"
#include "types.h"

#include "constants/battle_constants.h"
#include "constants/item.h"
#include "bag.h"
#include "battle.h"
#include "pokemon.h"
#include "coop_challenge.h"

// ---------------------------------------------------------------------------
// Milestone 5 Phase 3: Nuzlocke, battle side (docs/MILESTONE5_PLAN.md §5.4).
// Battle extension. The field side (locations, the Graveyard) is in
// src/field/coop_challenge_nuzlocke.c.
// ---------------------------------------------------------------------------

#define COOP_HEAP_BATTLE 5

static struct CoopChallengeSave *Coop_NuzlockeSave(void)
{
    struct CoopChallengeSave *save = Coop_ChallengeSaveOf(SaveBlock2_get());

    return save != NULL && (save->rules & COOP_RULE_NUZLOCKE) ? save : NULL;
}

static void Coop_MarkDead(struct PartyPokemon *mon)
{
    u16 bits;

    if (mon == NULL || !GetMonData(mon, MON_DATA_SPECIES_EXISTS, NULL) || GetMonData(mon, MON_DATA_IS_EGG, NULL))
        return;
    bits = GetMonData(mon, MON_DATA_RESERVED_114, NULL) | COOP_MON_DEAD;
    SetMonData(mon, MON_DATA_RESERVED_114, &bits);
}

// No player Pokemon fights in the Safari Zone or the Bug Contest.
static BOOL Coop_PlayerFights(struct BattleSystem *bsys)
{
    return !(BattleTypeGet(bsys) & (BATTLE_TYPE_SAFARI | BATTLE_TYPE_BUG_CONTEST));
}

void LONG_CALL Coop_ChallengeMarkFainted(struct BattleSystem *bsys, struct BattleStruct *ctx, int battlerId)
{
    // Link battles: the host runs the scripts for everyone; each machine marks
    // its own party at the end instead (Coop_ChallengeMarkBattleEnd).
    if ((battlerId & 1) || Coop_NuzlockeSave() == NULL || (BattleTypeGet(bsys) & BATTLE_TYPE_LINK) || !Coop_PlayerFights(bsys))
        return;
    Coop_MarkDead(BattleWorkPokemonParamGet(bsys, battlerId, ctx->sel_mons_no[battlerId]));
}

void LONG_CALL Coop_ChallengeMarkBattleEnd(struct BattleSystem *bsys)
{
    u32 type = BattleTypeGet(bsys);
    int client, i, last;

    if (Coop_NuzlockeSave() == NULL || !Coop_PlayerFights(bsys))
        return;
    // The player side: party 0, and party 2 when it is a separate one (a
    // multi or a tag); marking a partner's copy is harmless, only the local
    // party goes back to the save. PvP link battles never write it back.
    last = (type & (BATTLE_TYPE_MULTI | BATTLE_TYPE_TAG)) ? 2 : 0;
    for (client = 0; client <= last; client += 2)
        for (i = 0; i < BattleWorkPokeCountGet(bsys, client); i++)
        {
            struct PartyPokemon *mon = BattleWorkPokemonParamGet(bsys, client, i);
            if (GetMonData(mon, MON_DATA_HP, NULL) == 0)
                Coop_MarkDead(mon);
        }
}

BOOL LONG_CALL Coop_ChallengeBallBlocked(struct BattleSystem *bsys)
{
    struct CoopChallengeSave *save = Coop_NuzlockeSave();
    u32 type = BattleTypeGet(bsys);
    struct BattleStruct *ctx;

    if (save == NULL || (type & (BATTLE_TYPE_TRAINER | BATTLE_TYPE_LINK)) || !(save->pending_flags & COOP_PENDING_OPEN))
        return FALSE;
    if (save->pending_flags & (COOP_PENDING_FIRST | COOP_PENDING_EXEMPT))
        return FALSE;
    if (MonIsShiny(BattleWorkPokemonParamGet(bsys, 1, 0)))
        return FALSE; // the shiny clause (§5.4.2)
    // The ball left the bag when it was chosen: give it back. Safari and
    // Sport Balls are counters, not bag items (KNOWN_RISKS).
    ctx = bsys->sp;
    if (!(type & (BATTLE_TYPE_SAFARI | BATTLE_TYPE_BUG_CONTEST)) && ctx->item_work != ITEM_NONE)
        Bag_AddItem(bsys->bag, (u16)ctx->item_work, 1, COOP_HEAP_BATTLE);
    return TRUE;
}

BOOL LONG_CALL Coop_ChallengeForceNickname(void)
{
    return Coop_NuzlockeSave() != NULL;
}
