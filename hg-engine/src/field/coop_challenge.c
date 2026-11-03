#include "config.h"
#include "types.h"

#include "save.h"
#include "script.h"
#include "coop_sync.h"
#include "coop_challenge.h"

// ---------------------------------------------------------------------------
// Milestone 5 challenge runs, Overlay 131 (docs/MILESTONE5_PLAN.md §4).
// Phase 1: the room's rules reach the save at the adventure start, and the
// ledgers travel between the save and the relay. The rules themselves act
// from Phase 2 on.
// ---------------------------------------------------------------------------

struct CoopChallengeSync gCoopChallengeSync = {
    .magic = COOP_CHALLENGE_SYNC_MAGIC,
    .version = COOP_CHALLENGE_SYNC_VERSION,
    .size = sizeof(struct CoopChallengeSync),
};

static const char *const sRandomModes[] = { "", "Semi-random!", "Preserve difficulty!", "Fully random!" };

// hg-engine type order (constants/battle_constants.h): Fairy took slot 9.
static const char *const sTypeNames[COOP_MONO_TYPE_COUNT] = {
    "NORMAL", "FIGHTING", "FLYING", "POISON", "GROUND", "ROCK", "BUG", "GHOST", "STEEL",
    "FAIRY", "FIRE", "WATER", "GRASS", "ELECTRIC", "PSYCHIC", "ICE", "DRAGON", "DARK",
};

struct CoopChallengeSave *Coop_ChallengeSave(void)
{
    return Coop_ChallengeSaveOf(SaveBlock2_get());
}

u16 Coop_ChallengeRules(void)
{
    struct CoopChallengeSave *save = Coop_ChallengeSave();

    return save != NULL ? save->rules : 0;
}

// The relay's trailer as the bridge wrote it: only trusted with its magic,
// version and checksum, and its values are clamped again here.
static const struct CoopChallengeDown *Coop_ChallengeIn(void)
{
    const struct CoopChallengeDown *in = &gCoopChallengeSync.in;
    const u8 *bytes = (const u8 *)in;
    u16 sum = 0;
    u32 i;

    if (in->magic != COOP_CHALLENGE_WIRE_MAGIC || in->version != COOP_CHALLENGE_WIRE_VERSION)
        return NULL;
    for (i = 0; i < __builtin_offsetof(struct CoopChallengeDown, checksum); i++)
        sum += bytes[i];
    if (sum != in->checksum)
        return NULL;
    return in;
}

static u16 Coop_SanitizeRules(u16 raw)
{
    u16 rules = raw & COOP_RULES_VALID;
    u32 soulLink = (rules & COOP_RULE_SOUL_LINK_MASK) >> COOP_RULE_SOUL_LINK_SHIFT;

    if (soulLink > COOP_SOUL_LINK_DUO || !(rules & COOP_RULE_NUZLOCKE))
        rules &= ~COOP_RULE_SOUL_LINK_MASK;
    if (!(rules & COOP_RULE_RANDOM_MASK))
        rules &= ~(COOP_RULE_LEGENDS_WILD | COOP_RULE_RANDOM_ABILITIES);
    return rules;
}

// Writes the block once the roster is locked, from the relay's trailer: at
// the adventure start (the save that follows carries it), or later when the
// block was lost (docs/MILESTONE5_PLAN.md E45). A room without rules latches
// too, so its saves keep "no rules" for good.
static struct CoopChallengeSave *Coop_ChallengeLatch(void)
{
    const struct CoopChallengeDown *in;
    struct CoopChallengeSave *save = Coop_ChallengeSave();
    u8 *walker;
    u32 i;

    if (save != NULL)
        return save;
    if (!Coop_IsAdventureStarted() || (in = Coop_ChallengeIn()) == NULL)
        return NULL;
    walker = SaveArray_Get(SaveBlock2_get(), COOP_SAVE_POKEWALKER);
    if (walker == NULL)
        return NULL;
    save = (struct CoopChallengeSave *)(walker + COOP_CHALLENGE_SAVE_OFFSET);
    for (i = 0; i < sizeof(*save); i++)
        ((u8 *)save)[i] = 0;
    save->version = COOP_CHALLENGE_SAVE_VERSION;
    save->rules = Coop_SanitizeRules(in->rules);
    save->mono_type = (save->rules & COOP_RULE_MONOLOCKE) && in->mono_type < COOP_MONO_TYPE_COUNT
                          ? in->mono_type
                          : COOP_MONO_TYPE_NONE;
    save->seed = gCoopCommBlock.gates.room_uid;
    save->pending_loc = 0xFF;
    Coop_ChallengeStarters(save); // Phase 2: Elm's choices (the starter app runs without this overlay)
    save->state = COOP_CHALLENGE_LATCHED;
    save->magic = COOP_CHALLENGE_SAVE_MAGIC; // last: the block is valid from here on
    return save;
}

void Coop_ChallengeFieldFrame(struct FieldSystem *fsys)
{
    struct CoopChallengeSave *save = Coop_ChallengeLatch();
    const struct CoopChallengeDown *in = Coop_ChallengeIn();
    struct CoopChallengeUp *out = &gCoopChallengeSync.out;
    u32 i;

    if (save != NULL && (save->rules & COOP_RULE_NUZLOCKE))
        Coop_ChallengeNuzlockeFrame(fsys, save); // Phase 3
    if (save != NULL && in != NULL)
    {
        // The relay's ledgers only ever gain bits: OR them back, so a restored
        // backup or a rewritten block can't reopen a route (§4.4).
        for (i = 0; i < COOP_LOCATION_BYTES; i++)
        {
            save->spent[i] |= in->spent[i];
            save->failed[i] |= in->failed[i];
        }
    }

    // The report the bridge forwards (it sets magic, version and checksum).
    out->magic = COOP_CHALLENGE_WIRE_MAGIC;
    out->version = COOP_CHALLENGE_WIRE_VERSION;
    out->living = COOP_LIVING_UNKNOWN;
    if (save == NULL)
    {
        out->state = 0;
        out->rules = 0;
        out->deaths = 0;
        for (i = 0; i < COOP_LOCATION_BYTES; i++)
            out->spent[i] = out->failed[i] = 0;
        return;
    }
    out->state = save->state;
    out->rules = save->rules;
    out->deaths = save->deaths;
    for (i = 0; i < COOP_LOCATION_BYTES; i++)
    {
        out->spent[i] = save->spent[i];
        out->failed[i] = save->failed[i];
    }
}

static void Coop_CopyAscii(char *buf, u32 size, const char *const *parts, u32 count)
{
    u32 n = 0, i;
    const char *p;

    for (i = 0; i < count; i++)
        for (p = parts[i]; *p != '\0' && n + 1 < size; p++)
            buf[n++] = *p;
    buf[n] = '\0';
}

BOOL Coop_ChallengeStartPage(u32 index, char *buf, u32 size)
{
    struct CoopChallengeSave *save = Coop_ChallengeLatch();
    const char *parts[4];
    u16 rules;
    u32 page = 0;

    if (save == NULL || size == 0)
        return FALSE;
    rules = save->rules;

    if (rules & COOP_RULE_RANDOM_MASK)
    {
        if (page++ == index)
        {
            parts[0] = "Random encounters:\n";
            parts[1] = sRandomModes[rules & COOP_RULE_RANDOM_MASK];
            Coop_CopyAscii(buf, size, parts, 2);
            return TRUE;
        }
        if ((rules & COOP_RULE_RANDOM_ABILITIES) && page++ == index)
        {
            parts[0] = "Wild Pokémon also have\nrandom abilities!";
            Coop_CopyAscii(buf, size, parts, 1);
            return TRUE;
        }
    }
    if (rules & COOP_RULE_NUZLOCKE)
    {
        if (page++ == index)
        {
            u32 soulLink = (rules & COOP_RULE_SOUL_LINK_MASK) >> COOP_RULE_SOUL_LINK_SHIFT;
            parts[0] = soulLink == COOP_SOUL_LINK_ROOM_WIDE ? "Nuzlocke rules, with a\nroom-wide Soul-Link!"
                     : soulLink == COOP_SOUL_LINK_DUO       ? "Nuzlocke rules, with a\nSoul-Link for teams!"
                                                            : "Nuzlocke rules\nare on!";
            Coop_CopyAscii(buf, size, parts, 1);
            return TRUE;
        }
    }
    if ((rules & COOP_RULE_MONOLOCKE) && save->mono_type < COOP_MONO_TYPE_COUNT)
    {
        if (page++ == index)
        {
            parts[0] = "Monolocke! You can only\ncatch ";
            parts[1] = sTypeNames[save->mono_type];
            parts[2] = "-type Pokémon!";
            Coop_CopyAscii(buf, size, parts, 3);
            return TRUE;
        }
    }
    if (rules & COOP_RULE_LEVEL_CAP)
    {
        if (page++ == index)
        {
            parts[0] = "Levels are capped at the\nnext boss's strongest!";
            Coop_CopyAscii(buf, size, parts, 1);
            return TRUE;
        }
    }
    return FALSE;
}
