#include "config.h"
#include "types.h"

#include "constants/item.h"
#include "constants/species.h"

#include "map_events_internal.h"
#include "pokemon.h"
#include "save.h"
#include "coop_sync.h"
#include "coop_boss.h"
#include "coop_rewards.h"

// ---------------------------------------------------------------------------
// Team boss rewards (docs/MILESTONE3_PLAN.md §20), Overlay 131.
//
// A team boss battle is a link multi (src/coop_boss.c): vanilla gives no
// experience and no prize money in a link battle, and only the host runs the
// battle's logic, so nothing can be recorded there for the other game. Each
// game knows the boss's whole team (Coop_BossSplitParty; a win beats all of
// it) and its own party before the battle (Coop_SnapshotPartyForRewards): the
// lead and every Pokemon whose HP or PP changed battled. After a win each game
// pays its own save here, once the boss's script is over: vanilla's prize for
// the boss, and experience as hg-engine's battle would have given it with the
// global Exp. Share (config.h GLOBAL_EXP_SHARE).
// ---------------------------------------------------------------------------

#define COOP_MAX_MONEY 999999 // pret MAX_MONEY
#define COOP_PARTY_SIZE 6

#define COOP_LEADER_MULTIPLIER 30 // the gym leaders' and the Elite Four's, without hg-engine's table

#ifdef EXPAND_TRAINER_PRIZE_MONEY
// hg-engine src/trainermoney.c.
struct TrainerMoney
{
    u16 class;
    u16 multiplier;
};
extern struct TrainerMoney PrizeMoney[];
extern const u32 PrizeMoneyCount;
#endif

// hg-engine src/pokemon.c: 0 no more moves at this level, MOVE_APPEND_FULL,
// 0xFFFE already known, else the move learned.
u32 LONG_CALL MonTryLearnMoveOnLevelUp(struct PartyPokemon *mon, int *last_i, u16 *move);

// pret CalcPrizeMoney for one trainer: its last Pokemon's level * 4 * the
// class's multiplier, twice for a double battle trainer, twice with an
// Amulet Coin (vanilla: if its holder battled; here: if one is held).
static u32 Coop_BossPrize(struct Party *party)
{
    u32 multiplier = COOP_LEADER_MULTIPLIER, i, prize;

#ifdef EXPAND_TRAINER_PRIZE_MONEY
    for (i = 0; i < PrizeMoneyCount; i++)
    {
        if (PrizeMoney[i].class == gCoopBoss.prizeClass)
        {
            multiplier = PrizeMoney[i].multiplier;
            break;
        }
    }
#endif
    prize = gCoopBoss.prizeLevel * 4 * multiplier;
    if (gCoopBoss.prizeDouble)
        prize *= 2;
    for (i = 0; i < (u32)party->count; i++)
    {
        if (GetMonData(Party_GetMonByIndex(party, i), MON_DATA_HELD_ITEM, NULL) == ITEM_AMULET_COIN)
        {
            prize *= 2;
            break;
        }
    }
    return prize;
}

// hg-engine Task_DistributeExp_Extend, scaled formula (EXPERIENCE_FORMULA_GEN
// 5 / 7+): base * L / 5 * ((2L + 10) / (L + Lp + 10))^2.5.
static u32 Coop_ScaledExp(u32 base, u32 level, u32 Lp)
{
    u32 totalexp = (base * level) / 5;
    u32 top = (2 * level + 10) * (2 * level + 10) * sqrt(2 * level + 10);
    u32 bottom = (level + Lp + 10) * (level + Lp + 10) * sqrt(level + Lp + 10);
    u32 result = top * totalexp;

    if ((result / top) != totalexp) // overflowed (see the original)
    {
        totalexp = result + 1;
        return (totalexp / bottom) + (-1u / bottom) + 1;
    }
    return totalexp * top / bottom;
}

void Coop_SnapshotPartyForRewards(FieldSystem *fsys)
{
    struct Party *party = SaveData_GetPlayerPartyPtr(fsys->savedata);
    u32 slot, i;

    gCoopBoss.leadSlot = 0xFF;
    for (slot = 0; slot < COOP_PARTY_SIZE; slot++)
    {
        struct PartyPokemon *mon = Party_GetMonByIndex(party, slot);
        BOOL here = slot < (u32)party->count;

        gCoopBoss.hpBefore[slot] = here ? (u16)GetMonData(mon, MON_DATA_HP, NULL) : 0;
        for (i = 0; i < 4; i++)
            gCoopBoss.ppBefore[slot][i] = here ? (u8)GetMonData(mon, MON_DATA_MOVE1PP + i, NULL) : 0;
        if (here && gCoopBoss.leadSlot == 0xFF && gCoopBoss.hpBefore[slot] != 0 && !GetMonData(mon, MON_DATA_IS_EGG, NULL))
            gCoopBoss.leadSlot = (u8)slot;
    }
    if (gCoopBoss.order[0] != 0) // the three were chosen: the first of them leads
        gCoopBoss.leadSlot = (u8)(gCoopBoss.order[0] - 1);
}

// The Pokemon that battled: the lead, and every one whose HP or PP changed.
static u32 Coop_BattledMask(struct Party *party)
{
    u32 mask = gCoopBoss.leadSlot < COOP_PARTY_SIZE ? 1u << gCoopBoss.leadSlot : 0;
    u32 slot, i;

    for (slot = 0; slot < (u32)party->count && slot < COOP_PARTY_SIZE; slot++)
    {
        struct PartyPokemon *mon = Party_GetMonByIndex(party, slot);

        if (GetMonData(mon, MON_DATA_HP, NULL) != gCoopBoss.hpBefore[slot])
            mask |= 1u << slot;
        for (i = 0; i < 4; i++)
            if (GetMonData(mon, MON_DATA_MOVE1PP + i, NULL) != gCoopBoss.ppBefore[slot][i])
                mask |= 1u << slot;
    }
    return mask;
}

// What one boss Pokemon gives the Pokemon in slot: vanilla's multipliers
// (trainer battle, traded, Lucky Egg), then the Exp. Share's.
static u32 Coop_ExpFor(struct PartyPokemon *mon, u32 slot, u32 foe, u32 myId, u32 battled)
{
    u32 exp;

#if EXPERIENCE_FORMULA_GEN == 5 || EXPERIENCE_FORMULA_GEN > 6
    exp = Coop_ScaledExp(GetSpeciesBaseExp(gCoopBoss.foeSpecies[foe], gCoopBoss.foeForm[foe]), gCoopBoss.foeLevel[foe],
                         GetMonData(mon, MON_DATA_LEVEL, NULL));
#else
    exp = 255 * GetSpeciesBaseExp(gCoopBoss.foeSpecies[foe], gCoopBoss.foeForm[foe]) / 390;
    exp = exp * gCoopBoss.foeLevel[foe] / 7;
#endif
    exp = exp * 150 / 100; // trainer battle
    if (GetMonData(mon, MON_DATA_OTID, NULL) != myId)
        exp = exp * 150 / 100; // traded
    if (GetMonData(mon, MON_DATA_HELD_ITEM, NULL) == ITEM_LUCKY_EGG)
        exp = exp * 150 / 100;
    if (!(battled & (1u << slot)))
    {
#ifdef GLOBAL_EXP_SHARE
        exp = exp * GLOBAL_EXP_SHARE_PERCENT / 100;
#else
        exp = 0; // only the Pokemon that battled
#endif
    }
    return exp;
}

// Adds experience up to the level cap, one level at a time: each new level
// recalculates the stats and learns that level's moves into a free slot.
static void Coop_GiveExp(struct PartyPokemon *mon, u32 slot, u32 exp, struct CoopRewards *out)
{
    u32 species = GetMonData(mon, MON_DATA_SPECIES, NULL);
    u32 level = GetMonData(mon, MON_DATA_LEVEL, NULL);
    u32 cap = GetLevelCap();
    u32 total = GetMonData(mon, MON_DATA_EXPERIENCE, NULL);
    u32 max, next;

    if (cap > 100)
        cap = 100;
    if (level >= cap)
        return;
    max = PokeLevelExpGet(species, cap);
    out->exp[slot] = (exp > max - total) ? max - total : exp;
    total += out->exp[slot];
    out->gainedExp |= 1u << slot;
    while (level < cap && total >= (next = PokeLevelExpGet(species, level + 1)))
    {
        int last = 0;
        u16 move;
        u32 result;

        SetMonData(mon, MON_DATA_EXPERIENCE, &next);
        RecalcPartyPokemonStats(mon);
        level++;
        out->leveled |= 1u << slot;
        out->newLevel[slot] = (u8)level;
        while ((result = MonTryLearnMoveOnLevelUp(mon, &last, &move)) != 0)
        {
            if (result == MOVE_APPEND_FULL)
                out->couldNotLearn |= 1u << slot;
            else if (result != 0xFFFE)
                out->learned |= 1u << slot;
        }
    }
    SetMonData(mon, MON_DATA_EXPERIENCE, &total);
    RecalcPartyPokemonStats(mon);
}

BOOL Coop_PayBossRewards(FieldSystem *fsys, struct CoopRewards *out)
{
    struct Party *party = SaveData_GetPlayerPartyPtr(fsys->savedata);
    struct PlayerProfile *profile = Sav2_PlayerData_GetProfileAddr(fsys->savedata);
    u32 slot, foe, exp, battled;

    __builtin_memset(out, 0, sizeof(*out));
    if (!gCoopBoss.won)
        return FALSE;
    gCoopBoss.won = 0;

    if (gCoopBoss.prizeLevel != 0)
    {
        out->money = Coop_BossPrize(party);
        profile->money = (profile->money + out->money > COOP_MAX_MONEY) ? COOP_MAX_MONEY : profile->money + out->money;
    }
    battled = Coop_BattledMask(party);
    for (slot = 0; slot < (u32)party->count && slot < COOP_PARTY_SIZE; slot++)
    {
        struct PartyPokemon *mon = Party_GetMonByIndex(party, slot);

        if (GetMonData(mon, MON_DATA_SPECIES, NULL) == SPECIES_NONE || GetMonData(mon, MON_DATA_IS_EGG, NULL)
            || GetMonData(mon, MON_DATA_HP, NULL) == 0)
            continue;
        exp = 0;
        for (foe = 0; foe < gCoopBoss.foeCount && foe < COOP_BOSS_MAX_FOES; foe++)
            exp += Coop_ExpFor(mon, slot, foe, profile->id, battled);
        if (exp != 0)
            Coop_GiveExp(mon, slot, exp, out);
    }
    gCoopBoss.foeCount = 0;
    return TRUE;
}

int Coop_NextRewardEvolution(FieldSystem *fsys, struct CoopRewards *rewards, u16 *species, int *method)
{
    struct Party *party = SaveData_GetPlayerPartyPtr(fsys->savedata);
    u32 slot;

    for (slot = 0; slot < (u32)party->count && slot < COOP_PARTY_SIZE; slot++)
    {
        if (!(rewards->leveled & (1u << slot)) || (rewards->evolveChecked & (1u << slot)))
            continue;
        rewards->evolveChecked |= 1u << slot;
        *method = 0;
        *species = GetMonEvolution(party, Party_GetMonByIndex(party, slot), EVOCTX_LEVELUP, 0, method);
        if (*species != SPECIES_NONE && *species <= MAX_SPECIES_INCLUDING_FORMS)
            return (int)slot;
    }
    return -1;
}
