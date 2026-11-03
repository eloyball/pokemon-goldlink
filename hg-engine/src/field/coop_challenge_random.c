#include "config.h"
#include "types.h"

#include "constants/maps.h"
#include "constants/species.h"
#include "battle.h"
#include "pokemon.h"
#include "save.h"
#include "script.h"
#include "coop_sync.h"
#include "coop_challenge.h"

// ---------------------------------------------------------------------------
// Milestone 5 Phase 2: random encounters (docs/MILESTONE5_PLAN.md §5.1-§5.3),
// Overlay 131. Everything is decided from the save's latched rules and seed,
// so every player of a room sees the same route (Monolocke changes the key per
// type), and a reload gives the same Pokemon.
// ---------------------------------------------------------------------------

extern u32 space_for_setmondata; // the wild slot's form, applied by hg-engine's wild hooks

// Statics come from map scripts (a task runs); grass and surf encounters from
// a step (none does). Fishing, Rock Smash and Headbutt run as tasks too, so a
// static also needs its site (docs/MILESTONE5_PLAN.md §12.4).
_Static_assert(__builtin_offsetof(FieldSystem, taskman) == 0x10, "pret FieldSystem.taskman");

// Script statics (Phase 0 dump, §11.2): the map and its species; 0 = the
// script picks the species (the Bell Tower summit, Whirl Islands, Pewter).
static const struct { u16 map; u16 species; } sStatics[] = {
    { MAP_D11R0105, SPECIES_ARTICUNO }, { MAP_D41R0105, SPECIES_MOLTRES }, { MAP_R10, SPECIES_ZAPDOS },
    { MAP_D03R0103, SPECIES_MEWTWO }, { MAP_D18R0102, SPECIES_SUICUNE }, { MAP_R25, SPECIES_SUICUNE },
    { MAP_D52R0101, SPECIES_GROUDON }, { MAP_D52R0102, SPECIES_KYOGRE }, { MAP_D52R0103, SPECIES_RAYQUAZA },
    { MAP_D25R0103, SPECIES_LAPRAS }, { MAP_D35R0103, SPECIES_ELECTRODE }, { MAP_R11, SPECIES_SNORLAX },
    { MAP_R12, SPECIES_SNORLAX }, { MAP_R36, SPECIES_SUDOWOODO }, { MAP_T29, SPECIES_GYARADOS },
    { MAP_D17R0110, 0 }, { MAP_D40R0107, 0 }, { MAP_T03, 0 },
};

// The four vanilla roamers by roamer index (hg-engine src/field_roamer.c).
static const u16 sRoamerSpecies[] = { SPECIES_RAIKOU, SPECIES_ENTEI, SPECIES_LATIAS, SPECIES_LATIOS };

#define COOP_KEY_STARTER 0x53544152 // "STAR"
#define COOP_KEY_GIFT    0x47494654 // "GIFT"
#define COOP_KEY_ROAMER  0x524F414D // "ROAM"
#define COOP_KEY_WILD    0x57494C44 // "WILD"

// 32-bit mix (murmur3's finalizer over a running combination).
static u32 Coop_Hash(u32 a, u32 b, u32 c, u32 d)
{
    u32 h = a ^ 0x9E3779B9;
    u32 parts[3] = { b, c, d };
    u32 i;

    for (i = 0; i < 3; i++)
    {
        h ^= parts[i] + 0x9E3779B9 + (h << 6) + (h >> 2);
        h ^= h >> 16;
        h *= 0x85EBCA6B;
        h ^= h >> 13;
        h *= 0xC2B2AE35;
        h ^= h >> 16;
    }
    return h;
}

static u32 Coop_RandomU32(void)
{
    return ((u32)gf_rand() << 16) | gf_rand();
}

// The pool entry for an encounter value (species | form << 11), falling back
// to the species' base entry for a form the pool doesn't list.
const struct CoopPoolEntry *Coop_PoolFind(u32 value)
{
    u32 lo = 0, hi = gCoopPoolCount;

    while (lo < hi)
    {
        u32 mid = (lo + hi) / 2;
        if (gCoopPool[mid].value == value)
            return &gCoopPool[mid];
        if (gCoopPool[mid].value < value)
            lo = mid + 1;
        else
            hi = mid;
    }
    return (value >> 11) != 0 ? Coop_PoolFind(value & 0x7FF) : NULL;
}

struct CoopPoolFilter
{
    u8 legends;   // COOP_POOL_LEGEND_*: legends of these pools qualify
    u8 plain;     // TRUE: wild-eligible non-legends qualify
    u8 tier;      // 0 any, else non-legends of this tier only
    u8 baseOnly;  // only base forms
    u8 formZero;  // only form 0 (roamers store a species, no form)
    u8 monoType;  // COOP_MONO_TYPE_NONE: any
    const u16 *exclude; // values never picked
    u8 excludeCount;
    const u8 *families; // dupes clause: families never picked (pool family bits), NULL: none
};

static BOOL Coop_PoolMatches(const struct CoopPoolEntry *e, const struct CoopPoolFilter *f)
{
    u32 i;

    if (e->flags & COOP_POOL_LEGENDS)
    {
        if (!(e->flags & f->legends))
            return FALSE;
    }
    else if (!f->plain || !(e->flags & COOP_POOL_PICK) || (f->tier != 0 && e->tier != f->tier))
        return FALSE;
    if (f->baseOnly && !(e->flags & COOP_POOL_BASE))
        return FALSE;
    if (f->formZero && (e->value >> 11) != 0)
        return FALSE;
    if (f->monoType != COOP_MONO_TYPE_NONE && e->type1 != f->monoType && e->type2 != f->monoType)
        return FALSE;
    for (i = 0; i < f->excludeCount; i++)
        if (f->exclude[i] == e->value)
            return FALSE;
    if (f->families != NULL && ((f->families[e->family >> 3] >> (e->family & 7)) & 1))
        return FALSE;
    return TRUE;
}

// The key-th match (modulo their count), or 0 when nothing matches. Two passes
// over the table: no rejection sampling, one bounded cost per encounter.
static u16 Coop_PoolPick(const struct CoopPoolFilter *f, u32 key)
{
    u32 count = 0, i, n;

    for (i = 0; i < gCoopPoolCount; i++)
        if (Coop_PoolMatches(&gCoopPool[i], f))
            count++;
    if (count == 0)
        return 0;
    n = key % count;
    for (i = 0; i < gCoopPoolCount; i++)
        if (Coop_PoolMatches(&gCoopPool[i], f) && n-- == 0)
            return gCoopPool[i].value;
    return 0;
}

static u8 Coop_MonoFilterType(const struct CoopChallengeSave *save)
{
    return (save->rules & COOP_RULE_MONOLOCKE) && save->mono_type < COOP_MONO_TYPE_COUNT ? save->mono_type
                                                                                        : COOP_MONO_TYPE_NONE;
}

static u32 Coop_CurrentMap(void)
{
    FieldSystem *fsys = gFieldSysPtr;

    return fsys != NULL && fsys->location != NULL ? fsys->location->mapId : 0;
}

// A static: a script is running, on one of the dump's sites, with its species
// (or, where the script picks it, a legend: those sites only give legends,
// and fishing in Lugia's chamber must stay a regular encounter).
static BOOL Coop_IsStatic(BOOL taskRunning, u32 map, u32 value)
{
    const struct CoopPoolEntry *e;
    u32 species = value & 0x7FF;
    u32 i;

    if (!taskRunning)
        return FALSE;
    for (i = 0; i < NELEMS(sStatics); i++)
    {
        if (sStatics[i].map != map)
            continue;
        if (sStatics[i].species == species)
            return TRUE;
        if (sStatics[i].species == 0 && (e = Coop_PoolFind(value)) != NULL && (e->flags & COOP_POOL_LEGENDS))
            return TRUE;
    }
    return FALSE;
}

// The replacement for one encounter value, or `original` when there is none.
// Legends always stay in their own pool (Q16), in every mode.
// The replacement for one encounter value, or `original` when there is none.
// `families`: the dupes clause's caught families (a first encounter), or NULL.
static u32 Coop_ReplaceWild(const struct CoopChallengeSave *save, u32 original, u32 key, const u8 *families)
{
    const struct CoopPoolEntry *orig = Coop_PoolFind(original);
    u32 mode = save->rules & COOP_RULE_RANDOM_MASK;
    struct CoopPoolFilter f = { 0 };
    u16 pick;

    f.monoType = Coop_MonoFilterType(save);
    f.families = families;
    if (orig != NULL && (orig->flags & COOP_POOL_LEGENDS))
    {
        f.legends = orig->flags & COOP_POOL_LEGENDS;
        pick = Coop_PoolPick(&f, key);
        return pick != 0 ? pick : original; // no legend of this type: the original stays (E19)
    }
    f.plain = TRUE;
    if (mode == COOP_RANDOM_PRESERVE && orig != NULL)
        f.tier = orig->tier;
    if (save->rules & COOP_RULE_LEGENDS_WILD)
    {
        // Pool A joins tier 4 (and every tier without tier matching); Pool B
        // only Fully random (§5.1.4).
        if (mode != COOP_RANDOM_PRESERVE || f.tier == 4)
            f.legends |= COOP_POOL_LEGEND_A;
        if (mode == COOP_RANDOM_FULL)
            f.legends |= COOP_POOL_LEGEND_B;
    }
    pick = Coop_PoolPick(&f, key);
    if (pick == 0 && f.tier != 0)
    {
        f.tier = 0; // nothing of this type in the tier: any tier (§5.1.3)
        pick = Coop_PoolPick(&f, key);
    }
    if (pick == 0 && f.families != NULL)
    {
        f.families = NULL; // every candidate's family is caught: the dupes clause gives way
        pick = Coop_PoolPick(&f, key);
    }
    return pick != 0 ? pick : original;
}

static BOOL Coop_HasType(u32 value, u8 type)
{
    const struct CoopPoolEntry *e = Coop_PoolFind(value);

    return e != NULL && (e->type1 == type || e->type2 == type);
}

void Coop_ChallengeWildMon(struct PartyPokemon *pp, struct BATTLE_PARAM *bp)
{
    struct CoopChallengeSave *save = Coop_ChallengeSave();
    FieldSystem *fsys = gFieldSysPtr;
    BOOL task = fsys != NULL && fsys->taskman != NULL;
    BOOL nuzlocke = save != NULL && (save->rules & COOP_RULE_NUZLOCKE);
    BOOL first = FALSE;
    u32 species, original, result, map, key, level, pid, otid, kind, loc = 0xFF;
    u32 forceLevel = 0;
    u16 ability = 0;

    Coop_ChallengeRecordPrebattle(fsys); // Phase 5: the spirit's anchor if this battle wipes the party
    species = GetMonData(pp, MON_DATA_SPECIES, NULL);
    original = species | (space_for_setmondata << 11);
    map = Coop_CurrentMap();
    kind = (bp->fight_type & (BATTLE_TYPE_SAFARI | BATTLE_TYPE_BUG_CONTEST)) ? 2
         : Coop_IsStatic(task, map, original)                                ? 1
                                                                             : 0;
    result = original;

    // A battle still pending (another one came before a quiet field frame).
    if (nuzlocke && fsys != NULL && (save->pending_flags & COOP_PENDING_OPEN))
        Coop_ChallengeResolvePending(fsys, save);

    // Nuzlocke (§5.4.2): a static has no location; elsewhere the first
    // encounter of an unspent location brings the dupes clause along.
    if (nuzlocke && kind != 1)
    {
        loc = Coop_ChallengeLocation(fsys, bp->fight_type, map);
        first = loc < COOP_LOCATION_BYTES * 8 && !((save->spent[loc >> 3] >> (loc & 7)) & 1);
    }

    if (save != NULL && (save->rules & COOP_RULE_RANDOM_MASK) && species != 0 && species != SPECIES_EGG)
    {
        if ((save->rules & COOP_RULE_RANDOM_MASK) == COOP_RANDOM_FULL)
            key = Coop_RandomU32();
        else
            key = Coop_Hash(save->seed ^ COOP_KEY_WILD, map, original, Coop_MonoFilterType(save));
        result = Coop_ReplaceWild(save, original, key, first ? Coop_ChallengeFamilyMask() : NULL);
    }
    else if (first && Coop_ChallengeFamilyCaught(original))
    {
        u32 reroll = Coop_ChallengeTableReroll(fsys, original); // dupes clause, randomizer off (§5.4.3)
        if (reroll != 0)
            result = reroll;
    }

#if COOP_DEBUG_TOOLS
    if (gCoopDebugMonBuffer.team_battle_armed == 4) // probes: this one becomes buffer.species at buffer.level (docs/DEBUG_TOOLS.md)
    {
        gCoopDebugMonBuffer.team_battle_armed = 0;
        result = gCoopDebugMonBuffer.species;
        forceLevel = gCoopDebugMonBuffer.level;
    }
#endif

    if (result != original || forceLevel != 0)
    {
        // Same level, personality and trainer ID: shininess, nature and
        // gender bits stay; hg-engine's hook then applies the new form
        // (space_for_setmondata), held item and ability as usual.
        level = forceLevel != 0 ? forceLevel : GetMonData(pp, MON_DATA_LEVEL, NULL);
        pid = GetMonData(pp, MON_DATA_PERSONALITY, NULL);
        otid = GetMonData(pp, MON_DATA_OTID, NULL);
        space_for_setmondata = result >> 11;
        PokeParaSet(pp, result & 0x7FF, level, 32, TRUE, pid, OT_ID_PRESET, otid);
    }

    if (save != NULL && (save->rules & COOP_RULE_RANDOM_MASK) && species != 0 && species != SPECIES_EGG)
    {
        if ((save->rules & COOP_RULE_RANDOM_ABILITIES) && gCoopAbilityPoolCount != 0)
        {
            u16 locked;

            ability = gCoopAbilityPool[Coop_RandomU32() % gCoopAbilityPoolCount];
            SetMonData(pp, MON_DATA_ABILITY, &ability);
            locked = GetMonData(pp, MON_DATA_RESERVED_114, NULL) | COOP_MON_ABILITY_LOCKED;
            SetMonData(pp, MON_DATA_RESERVED_114, &locked);
        }
    }

    if (nuzlocke)
    {
        u8 flags = COOP_PENDING_OPEN;

        if (kind == 1 || loc == 0xFF)
            flags |= COOP_PENDING_EXEMPT; // a static: catchable, spends nothing (E6)
        else if (first)
        {
            // Monolocke with the randomizer off: a Pokemon of another type
            // doesn't spend the location (§6.2 of the draft).
            BOOL valid = !((save->rules & COOP_RULE_MONOLOCKE) && !(save->rules & COOP_RULE_RANDOM_MASK)
                           && !Coop_HasType(result, save->mono_type));
            if (valid)
            {
                save->spent[loc >> 3] |= (u8)(1 << (loc & 7));
                flags |= COOP_PENDING_FIRST;
            }
        }
        save->pending_pid = GetMonData(pp, MON_DATA_PERSONALITY, NULL);
        save->pending_loc = (u8)loc;
        save->pending_flags = flags;
    }

#if COOP_DEBUG_TOOLS
    gCoopDebugMonBuffer.wild_original = original; // probes (docs/DEBUG_TOOLS.md)
    gCoopDebugMonBuffer.wild_result = result;
    gCoopDebugMonBuffer.wild_map = map;
    gCoopDebugMonBuffer.wild_kind = kind;
    gCoopDebugMonBuffer.wild_task = (u8)task;
    gCoopDebugMonBuffer.wild_ability = ability;
    gCoopDebugMonBuffer.wild_count++;
#else
    (void)kind;
    (void)ability;
#endif
}

void Coop_ChallengeStarters(struct CoopChallengeSave *save)
{
    struct CoopPoolFilter f = { 0 };
    u32 i;

    if (!(save->rules & (COOP_RULE_RANDOM_MASK | COOP_RULE_MONOLOCKE)))
        return; // vanilla starters
    f.plain = TRUE;
    f.baseOnly = TRUE;
    f.tier = 1;
    f.monoType = Coop_MonoFilterType(save);
    f.exclude = save->starters;
    for (i = 0; i < 3; i++)
    {
        u32 key = Coop_Hash(save->seed ^ COOP_KEY_STARTER, i, f.monoType, 0);
        u16 pick;

        f.excludeCount = i; // three different Pokemon
        f.tier = 1;
        pick = Coop_PoolPick(&f, key);
        if (pick == 0)
        {
            f.tier = 0; // not enough tier-1 base forms of this type
            pick = Coop_PoolPick(&f, key);
        }
        save->starters[i] = pick; // 0 keeps Elm's own for this ball
    }
}

u32 Coop_ChallengeGiftSpecies(u32 value)
{
    const struct CoopChallengeSave *save = Coop_ChallengeSave();
    const struct CoopPoolEntry *orig;
    struct CoopPoolFilter f = { 0 };
    u32 map = Coop_CurrentMap();
    u16 pick;

    if (save == NULL || !(save->rules & COOP_RULE_RANDOM_MASK))
        return value;
    if ((value & 0x7FF) == SPECIES_DRATINI && map == MAP_D44R0103)
        return value; // the Dragon's Den Dratini and its Extremespeed quiz (§5.3)
    orig = Coop_PoolFind(value);
    f.monoType = COOP_MONO_TYPE_NONE; // gifts are exempt from Monolocke
    if (orig != NULL && (orig->flags & COOP_POOL_LEGENDS))
        f.legends = orig->flags & COOP_POOL_LEGENDS;
    else
    {
        f.plain = TRUE;
        f.baseOnly = TRUE;
        f.tier = 1;
    }
    pick = Coop_PoolPick(&f, Coop_Hash(save->seed ^ COOP_KEY_GIFT, map, value, 0));
    return pick != 0 ? pick : value;
}

u32 Coop_ChallengeRoamerSpecies(u32 roamerIdx, u32 vanilla)
{
    const struct CoopChallengeSave *save = Coop_ChallengeSave();
    struct CoopPoolFilter f = { 0 };
    u16 picks[NELEMS(sRoamerSpecies)];
    u32 i;

    if (save == NULL || !(save->rules & COOP_RULE_RANDOM_MASK) || roamerIdx >= NELEMS(sRoamerSpecies))
        return vanilla;
    // Pool A, form 0 only, and four different species: vanilla finds a
    // roamer again by its species (SpeciesToRoamerIdx).
    f.legends = COOP_POOL_LEGEND_A;
    f.formZero = TRUE;
    f.monoType = Coop_MonoFilterType(save);
    f.exclude = picks;
    for (i = 0; i <= roamerIdx; i++)
    {
        f.excludeCount = i;
        picks[i] = Coop_PoolPick(&f, Coop_Hash(save->seed ^ COOP_KEY_ROAMER, i, f.monoType, 0));
        if (picks[i] == 0)
            picks[i] = sRoamerSpecies[i]; // not enough of this type: vanilla from here on
    }
    return picks[roamerIdx];
}
