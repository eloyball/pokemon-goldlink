#include "config.h"
#include "types.h"

#include "constants/item.h"
#include "constants/moves.h"

#include "bag.h"
#include "pokemon.h"
#include "save.h"
#include "script.h"
#include "coop_hm.h"

// ---------------------------------------------------------------------------
// HMs without HM moves (docs/MILESTONE3_PLAN.md §21), Overlay 131: the two
// ways the overworld asks "which party Pokemon knows move X", both hooked
// (`hooks`) and only called in the field:
//   - GetIdxOfFirstPartyMonWithMove (pret script_pokemon_util.c, 0x020542E8):
//     Surf at water, Waterfall (pret field_control.c);
//   - script command 141, ScrCmd_GetPartySlotWithMove (pret scrcmd_party.c,
//     0x0204D3CC): Cut trees, Rock Smash rocks, Strength boulders,
//     Whirlpools, Rock Climb walls.
// Vanilla's answer comes first. When no Pokemon knows the move and it is an
// unlocked HM (Coop_HmUnlocked), the answer is a helper: the first Pokemon
// that is not an egg, which then plays the field move (its cut-in).
// ---------------------------------------------------------------------------

#define COOP_NO_SLOT        0xFF

#define COOP_BADGE_ZEPHYR  0 // pret constants/badge.h
#define COOP_BADGE_HIVE    1
#define COOP_BADGE_PLAIN   2
#define COOP_BADGE_FOG     3
#define COOP_BADGE_STORM   4
#define COOP_BADGE_GLACIER 6
#define COOP_BADGE_RISING  7
#define COOP_BADGE_EARTH   15

// For each of gCoopHmMenuMoves: its HM (hg-engine data/MachineMoves.c, as
// vanilla) and the badge its field move needs (vanilla's own checks, pret
// src/field_move.c FieldMove_Check*).
static const struct
{
    u16 item;
    u8 badge;
} sCoopHms[COOP_HM_COUNT] = {
    { ITEM_HM02, COOP_BADGE_STORM },   // Fly
    { ITEM_HM03, COOP_BADGE_FOG },     // Surf
    { ITEM_HM04, COOP_BADGE_PLAIN },   // Strength
    { ITEM_HM01, COOP_BADGE_HIVE },    // Cut
    { ITEM_HM06, COOP_BADGE_ZEPHYR },  // Rock Smash
    { ITEM_HM07, COOP_BADGE_RISING },  // Waterfall
    { ITEM_HM05, COOP_BADGE_GLACIER }, // Whirlpool
    { ITEM_HM08, COOP_BADGE_EARTH },   // Rock Climb
};

// One pass over the TM / HM pocket (Bag_HasItem would read each item's data
// to find its pocket: file reads), then the badges. Cheap enough for every
// frame (the overworld asks about Waterfall on each input frame).
u32 Coop_HmUnlockedMask(void *saveData)
{
    BAG_DATA *bag;
    struct PlayerProfile *profile;
    u32 held = 0, mask = 0, i, j;

    if (saveData == NULL)
        return 0;
    bag = Sav2_Bag_get(saveData);
    profile = Sav2_PlayerData_GetProfileAddr(saveData);
    for (i = 0; i < NUM_BAG_TMS_HMS; i++)
    {
        if (bag->TMsHMs[i].quantity == 0)
            continue;
        for (j = 0; j < COOP_HM_COUNT; j++)
            if (bag->TMsHMs[i].id == sCoopHms[j].item)
                held |= 1u << j;
    }
    for (j = 0; j < COOP_HM_COUNT; j++)
        if ((held & (1u << j)) && PlayerProfile_TestBadgeFlag(profile, sCoopHms[j].badge))
            mask |= 1u << j;
    return mask;
}

BOOL Coop_HmUnlocked(void *saveData, u32 move)
{
    u32 i;

    for (i = 0; i < COOP_HM_COUNT; i++)
        if (gCoopHmMenuMoves[i] == move)
            return (Coop_HmUnlockedMask(saveData) >> i) & 1;
    return FALSE;
}

static u32 Coop_PartySlotForMove(struct Party *party, u32 move)
{
    u32 i, j, helper = COOP_NO_SLOT;

    for (i = 0; i < (u32)party->count; i++)
    {
        struct PartyPokemon *mon = Party_GetMonByIndex(party, i);

        if (GetMonData(mon, MON_DATA_IS_EGG, NULL))
            continue;
        if (helper == COOP_NO_SLOT)
            helper = i;
        for (j = 0; j < 4; j++)
            if (GetMonData(mon, MON_DATA_MOVE1 + j, NULL) == move)
                return i;
    }
    if (helper != COOP_NO_SLOT && gFieldSysPtr != NULL && Coop_HmUnlocked(gFieldSysPtr->savedata, move))
        return helper;
    return COOP_NO_SLOT;
}

// Replaces GetIdxOfFirstPartyMonWithMove: the slot, or 0xFF.
u32 LONG_CALL Coop_GetIdxOfFirstPartyMonWithMove(struct Party *party, u16 move)
{
    return Coop_PartySlotForMove(party, move);
}

// Replaces ScrCmd_GetPartySlotWithMove (checkpartymove var, move): the slot,
// or 6 (PARTY_SIZE) for none, as vanilla.
BOOL ScrCmd_GetPartySlotWithMove(SCRIPTCONTEXT *ctx)
{
    u16 *slot = ScriptGetVarPointer(ctx);
    u16 move = ScriptGetVar(ctx);
    u32 found = Coop_PartySlotForMove(SaveData_GetPlayerPartyPtr(ctx->fsys->savedata), move);

    *slot = found == COOP_NO_SLOT ? 6 : (u16)found;
    return FALSE;
}
