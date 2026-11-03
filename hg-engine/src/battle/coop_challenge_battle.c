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

// pret BattleSystem_Get/SetSafariBallCount (history labels ov12_0223B6B4 / ov12_0223B6C0).
int LONG_CALL BattleSystem_GetSafariBallCount(struct BattleSystem *bsys);
void LONG_CALL BattleSystem_SetSafariBallCount(struct BattleSystem *bsys, int count);

// Why the last ball was blocked, for PrintBallBlockedMessage (the same throw).
// The battle extension's data is reloaded with each battle.
static u8 sBallReason;
static u8 sBallType;

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

// Monolocke (§5.7): the wild Pokemon's species and form types, not its
// battle types (Soak, Burn Up...), must include the player's type.
static BOOL Coop_MonoBlocks(const struct CoopChallengeSave *save, struct PartyPokemon *wild)
{
    u32 species, form;

    if (!(save->rules & COOP_RULE_MONOLOCKE) || save->mono_type >= COOP_MONO_TYPE_NONE)
        return FALSE;
    species = GetMonData(wild, MON_DATA_SPECIES, NULL);
    form = GetMonData(wild, MON_DATA_FORM, NULL);
    return PokeFormNoPersonalParaGet(species, form, PERSONAL_TYPE_1) != save->mono_type
        && PokeFormNoPersonalParaGet(species, form, PERSONAL_TYPE_2) != save->mono_type;
}

// Nuzlocke (§5.4.2): only a location's first encounter or a static, unless shiny.
static BOOL Coop_NuzlockeBlocks(const struct CoopChallengeSave *save, struct PartyPokemon *wild)
{
    if (!(save->rules & COOP_RULE_NUZLOCKE) || !(save->pending_flags & COOP_PENDING_OPEN))
        return FALSE;
    if (save->pending_flags & (COOP_PENDING_FIRST | COOP_PENDING_EXEMPT))
        return FALSE;
    return !MonIsShiny(wild);
}

BOOL LONG_CALL Coop_ChallengeBallBlocked(struct BattleSystem *bsys)
{
    struct CoopChallengeSave *save = Coop_ChallengeSaveOf(SaveBlock2_get());
    u32 type = BattleTypeGet(bsys);
    struct PartyPokemon *wild;
    struct BattleStruct *ctx;

    sBallReason = COOP_BALL_ALLOWED;
    if (save == NULL || (type & (BATTLE_TYPE_TRAINER | BATTLE_TYPE_LINK)))
        return FALSE;
    wild = BattleWorkPokemonParamGet(bsys, 1, 0);
    if (Coop_MonoBlocks(save, wild)) // the shiny clause doesn't override it (E9)
    {
        sBallReason = COOP_BALL_MONOLOCKE;
        sBallType = save->mono_type;
    }
    else if (Coop_NuzlockeBlocks(save, wild))
        sBallReason = (save->pending_flags & COOP_PENDING_PARTNER_FAILED) ? COOP_BALL_SOUL_LINK : COOP_BALL_NUZLOCKE;
    else
        return FALSE;
    // The ball left the bag (or the Safari count) when it was chosen: give it
    // back. Sport Balls are the contest's own count (KNOWN_RISKS #67).
    ctx = bsys->sp;
    if (type & BATTLE_TYPE_SAFARI)
        BattleSystem_SetSafariBallCount(bsys, BattleSystem_GetSafariBallCount(bsys) + 1);
    else if (!(type & BATTLE_TYPE_BUG_CONTEST) && ctx->item_work != ITEM_NONE)
        Bag_AddItem(bsys->bag, (u16)ctx->item_work, 1, COOP_HEAP_BATTLE);
    return TRUE;
}

u32 LONG_CALL Coop_ChallengeBallReason(u32 *monoType)
{
    *monoType = sBallType;
    return sBallReason;
}

BOOL LONG_CALL Coop_ChallengeForceNickname(void)
{
    return Coop_NuzlockeSave() != NULL;
}
