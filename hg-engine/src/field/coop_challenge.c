#include "config.h"
#include "types.h"

#include "constants/species.h"
#include "pokemon.h"
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
const struct CoopChallengeDown *Coop_ChallengeIn(void)
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
    // Phase 2: Elm's choices (the starter app runs without this overlay); Fully
    // random gives each trainer their own (Milestone 6 §3.2).
    Coop_ChallengeStarters(save, PlayerProfile_GetTrainerID_VisibleHalf(Sav2_PlayerData_GetProfileAddr(SaveBlock2_get())));
    save->state = COOP_CHALLENGE_LATCHED;
    save->magic = COOP_CHALLENGE_SAVE_MAGIC; // last: the block is valid from here on
    return save;
}

// ---------------------------------------------------------------------------
// Trades (docs/MILESTONE5_PLAN.md §5.7, §5.9)
// ---------------------------------------------------------------------------

static void Coop_CopyAscii(char *buf, u32 size, const char *const *parts, u32 count);

static BOOL Coop_FormHasType(u32 species, u32 form, u32 type)
{
    return PokeFormNoPersonalParaGet(species, form, PERSONAL_TYPE_1) == type
        || PokeFormNoPersonalParaGet(species, form, PERSONAL_TYPE_2) == type;
}

const char *Coop_ChallengeTradeOfferRule(struct PartyPokemon *mon)
{
    const struct CoopChallengeSave *save = Coop_ChallengeSave();

    if (save != NULL && (save->rules & COOP_RULE_NUZLOCKE) && (GetMonData(mon, MON_DATA_RESERVED_114, NULL) & COOP_MON_DEAD))
        return "It has fallen. It can't\nbe traded.";
    // Phase 6 (E17): trading would sever the link and shield the Pokemon.
    if (Coop_ChallengeSoulLinkOn(save) && (GetMonData(mon, MON_DATA_RESERVED_114, NULL) & COOP_MON_LINKED))
        return "Its fate is linked to its\nplace. It can't be traded.";
    return NULL;
}

const char *Coop_ChallengeTradeRefusal(struct Party *party, struct PartyPokemon *incoming)
{
    static char sText[64]; // this overlay's data: kept while the trade task prints it
    const struct CoopChallengeSave *save = Coop_ChallengeSave();
    const char *parts[3];
    u32 species, target;
    int method = 0;

    if (save == NULL || !(save->rules & COOP_RULE_MONOLOCKE) || save->mono_type >= COOP_MONO_TYPE_NONE)
        return NULL;
    species = GetMonData(incoming, MON_DATA_SPECIES, NULL);
    if (Coop_FormHasType(species, GetMonData(incoming, MON_DATA_FORM, NULL), save->mono_type))
        return NULL;
    // Its trade evolution counts too (Onix holding a Metal Coat to a Steel player, E18).
    target = GetMonEvolution(party, incoming, EVOCTX_TRADE, 0, &method);
    if (target != SPECIES_NONE && target <= MAX_SPECIES_INCLUDING_FORMS && Coop_FormHasType(target, 0, save->mono_type))
        return NULL;
    parts[0] = "You can only receive\n";
    parts[1] = sTypeNames[save->mono_type];
    parts[2] = "-type Pokémon.\nThe trade was cancelled.";
    Coop_CopyAscii(sText, sizeof(sText), parts, 3);
    return sText;
}

void Coop_ChallengeTradeReceived(struct PartyPokemon *mon)
{
    // Another player's catch: not this player's wild catch (dupes clause) nor
    // linked to one of this player's locations.
    u16 bits = GetMonData(mon, MON_DATA_RESERVED_114, NULL);

    if (bits & (COOP_MON_WILD_CATCH | COOP_MON_LINKED))
    {
        bits &= ~(COOP_MON_WILD_CATCH | COOP_MON_LINKED);
        SetMonData(mon, MON_DATA_RESERVED_114, &bits);
    }
}

// ---------------------------------------------------------------------------
// Level cap (docs/MILESTONE5_PLAN.md §5.8): hg-engine's IMPLEMENT_LEVEL_CAP reads
// LEVEL_CAP_VARIABLE (0: no cap). The cap is the next boss's ace (Milestone 4's
// levels, docs/GAME_REBALANCE_UPDATE.md).
// ---------------------------------------------------------------------------

#define COOP_FLAG_GAME_CLEAR    2404             // vanilla FLAG_GAME_CLEAR: Lance (round 1) beaten
#define COOP_FLAG_LANCE_ROUND_2 (0x550 + 701)    // pret TRAINER_FLAG_BASE + TRAINER_CHAMPION_LANCE_2

static u32 Coop_CountBadges(u32 bits)
{
    u32 n = 0;

    for (; bits != 0; bits &= bits - 1)
        n++;
    return n;
}

u32 Coop_ChallengeLevelCapFor(u32 johto, u32 kanto, BOOL gameClear, BOOL round2Beaten)
{
    // Falkner, Bugsy, Whitney, Morty, then the badge-scaled gyms 5-7 (35, 39, 43),
    // Clair 50, Lance (round 1) 61.
    static const u8 sJohto[9] = { 14, 19, 24, 29, 35, 39, 43, 50, 61 };

    if (round2Beaten)
        return 90; // Red
    if (kanto >= 8)
        return 86; // the Elite Four's round 2
    if (gameClear)
        return kanto == 7 ? 78 : 63 + 2 * kanto; // Kanto leaders scale with Kanto badges; Blue 78
    return sJohto[johto > 8 ? 8 : johto];
}

static void Coop_ChallengeLevelCap(struct FieldSystem *fsys, const struct CoopChallengeSave *save)
{
    struct PlayerProfile *profile;
    u32 cap = 0;

    if (save->rules & COOP_RULE_LEVEL_CAP)
    {
        profile = Sav2_PlayerData_GetProfileAddr(fsys->savedata);
        if (profile == NULL)
            return;
        cap = Coop_ChallengeLevelCapFor(Coop_CountBadges(profile->johtoBadges), Coop_CountBadges(profile->kantoBadges),
                                        CheckScriptFlag(COOP_FLAG_GAME_CLEAR), CheckScriptFlag(COOP_FLAG_LANCE_ROUND_2));
    }
    if (VarGet(fsys, LEVEL_CAP_VARIABLE) != cap)
        VarSet(fsys, LEVEL_CAP_VARIABLE, (u16)cap);
}

void Coop_ChallengeFieldFrame(struct FieldSystem *fsys)
{
    struct CoopChallengeSave *save = Coop_ChallengeLatch();
    const struct CoopChallengeDown *in = Coop_ChallengeIn();
    struct CoopChallengeUp *out = &gCoopChallengeSync.out;
    u32 i;

    if (save != NULL)
        Coop_ChallengeStarterFrame(fsys, save); // Milestone 6: the starter's ability and Soul-Link
    if (save != NULL && (save->rules & COOP_RULE_NUZLOCKE))
        Coop_ChallengeNuzlockeFrame(fsys, save); // Phase 3
    if (save != NULL)
        Coop_ChallengeLevelCap(fsys, save); // Phase 4
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
            parts[0] = soulLink == COOP_SOUL_LINK_ROOM_WIDE ? "Nuzlocke rules, with a\nworld-wide Soul-Link!"
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
    if (rules & COOP_RULE_RANDOM_ITEMS)
    {
        if (page++ == index)
        {
            parts[0] = "Item balls hold\nrandom items!";
            Coop_CopyAscii(buf, size, parts, 1);
            return TRUE;
        }
    }
    return FALSE;
}

// ---------------------------------------------------------------------------
// Milestone 6 §3.2: Nuzlocke forces the starter's nickname. Elm asks "Give a
// nickname?" with GetMenuChoice (script command 748); its callback (pret
// sub_020477C0, replaced through `hooks`) writes the answer once the Yes / No
// menu is done, and No or B (both non-zero) skip NicknameInput. In Elm's
// starter script (after ChooseStarter, before the script's end) the first
// answer counts as Yes; the "So, NAME is it?" after the naming screen keeps
// its own (No goes back to the naming screen, as vanilla).
// ---------------------------------------------------------------------------

int LONG_CALL ov01_021F6B00(FieldSystem *fsys);
int LONG_CALL ov01_021F6AEC(FieldSystem *fsys);

// TRUE for the first Yes / No of Elm's starter script under Nuzlocke (once per save).
static BOOL Coop_IsStarterNicknameQuestion(FieldSystem *fsys)
{
    struct CoopChallengeSave *save = Coop_ChallengeSave();

    if (save == NULL || !(save->rules & COOP_RULE_NUZLOCKE) || (save->flags & COOP_FLAG_STARTER_NAMED))
        return FALSE;
    if (fsys->location == NULL || fsys->location->mapId != COOP_MAP_ELMS_LAB || !CheckScriptFlag(COOP_FLAG_GOT_STARTER)
        || VarGet(fsys, COOP_VAR_SCENE_ELMS_LAB) != 0)
        return FALSE;
    save->flags |= COOP_FLAG_STARTER_NAMED; // whatever the answer: the confirmation keeps its own
    return TRUE;
}

BOOL Coop_ScrNative_GetMenuChoice(SCRIPTCONTEXT *ctx)
{
    FieldSystem *fsys = ctx->fsys;
    u16 *ret = GetVarPointer(fsys, (u16)ctx->data[0]);
    u16 answer;

    if (ov01_021F6B00(fsys) != 3 || ov01_021F6AEC(fsys) != 6)
        return FALSE; // the menu is still open
    answer = (u16)ctx->data[1];
    if (Coop_IsStarterNicknameQuestion(fsys))
        answer = 0; // Yes
    *ret = answer;
    return TRUE;
}
