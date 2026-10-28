#include "config.h"
#include "types.h"

#include "constants/item.h"
#include "constants/maps.h"

#include "save.h"
#include "script.h"
#include "coop_marts.h"

// ---------------------------------------------------------------------------
// Badge-gated department store counters (docs/GAME_REBALANCE_UPDATE.md §8.2,
// §8.3, §9.1), Overlay 131. Vanilla's special marts read their item lists
// through hg-engine's repointed tables (`repoints`, src/field/mart.c), with
// no gate. While the player is on one of these floors, every field frame
// rebuilds its list from the save: the badge count for the evolution items
// and TMs, the Key Stone (FLAG_MEGA_EVOLUTION_ENABLED) and the Kanto badges
// for the Mega Stones. The lists live in this overlay, which is reloaded
// from the ROM with every field, so they are rebuilt each time.
// ---------------------------------------------------------------------------

struct CoopMartEntry
{
    u16 item;
    u8 gate; // badges (all sixteen) for the TM / evolution counters; Kanto badges for the Mega Stones
};

// Goldenrod Dept. Store 4F: the vitamins, then the evolution counter.
static const struct CoopMartEntry sGoldenrod4F[] = {
    { ITEM_PROTEIN, 0 }, { ITEM_IRON, 0 }, { ITEM_CALCIUM, 0 }, { ITEM_ZINC, 0 }, { ITEM_CARBOS, 0 }, { ITEM_HP_UP, 0 },
    { ITEM_FIRE_STONE, 0 }, { ITEM_WATER_STONE, 0 }, { ITEM_THUNDER_STONE, 0 }, { ITEM_LEAF_STONE, 0 },
    { ITEM_MOON_STONE, 0 }, { ITEM_SUN_STONE, 0 }, { ITEM_ICE_STONE, 0 }, { ITEM_LINKING_CORD, 0 },
    { ITEM_SHINY_STONE, 4 }, { ITEM_DUSK_STONE, 4 }, { ITEM_DAWN_STONE, 4 }, { ITEM_OVAL_STONE, 4 },
    { ITEM_KINGS_ROCK, 4 }, { ITEM_METAL_COAT, 4 }, { ITEM_TART_APPLE, 4 }, { ITEM_SWEET_APPLE, 4 },
    { ITEM_SYRUPY_APPLE, 4 }, { ITEM_AUSPICIOUS_ARMOR, 4 }, { ITEM_MALICIOUS_ARMOR, 4 },
    { ITEM_STRAWBERRY_SWEET, 4 }, { ITEM_BERRY_SWEET, 4 }, { ITEM_LOVE_SWEET, 4 }, { ITEM_STAR_SWEET, 4 },
    { ITEM_CLOVER_SWEET, 4 }, { ITEM_FLOWER_SWEET, 4 }, { ITEM_RIBBON_SWEET, 4 },
    { ITEM_SEA_INCENSE, 4 }, { ITEM_LAX_INCENSE, 4 }, { ITEM_ODD_INCENSE, 4 }, { ITEM_ROCK_INCENSE, 4 }, { ITEM_FULL_INCENSE, 4 },
    { ITEM_WAVE_INCENSE, 4 }, { ITEM_ROSE_INCENSE, 4 }, { ITEM_LUCK_INCENSE, 4 }, { ITEM_PURE_INCENSE, 4 },
};

// Goldenrod Dept. Store 5F: the vanilla TMs, then the expansion TMs (and TRs
// for moves hg-engine has no TM for) by badges.
static const struct CoopMartEntry sGoldenrod5F[] = {
    { ITEM_TM070, 0 }, { ITEM_TM017, 0 }, { ITEM_TM054, 0 }, { ITEM_TM083, 0 }, { ITEM_TM016, 0 }, { ITEM_TM033, 0 },
    { ITEM_TM022, 0 }, { ITEM_TM052, 0 }, { ITEM_TM038, 0 }, { ITEM_TM025, 0 }, { ITEM_TM014, 0 }, { ITEM_TM015, 0 },
    { ITEM_TM211, 2 }, // Electroweb
    { ITEM_TM182, 2 }, // Bug Bite
    { ITEM_TM196, 4 }, // Flip Turn
    { ITEM_TM188, 4 }, // Scald
    { ITEM_TR99, 4 }, // Body Press
    { ITEM_TM110, 4 }, // Liquidation
    { ITEM_TR59, 4 }, // Seed Bomb
    { ITEM_TR92, 4 }, // Dazzling Gleam
    { ITEM_TM209, 4 }, // Muddy Water
    { ITEM_TM118, 6 }, // Heat Wave
    { ITEM_TM101, 6 }, // Power Gem
    { ITEM_TM099, 6 }, // Iron Head
    { ITEM_TM108, 6 }, // Crunch
    { ITEM_TM106, 6 }, // Drill Run
    { ITEM_TM112, 6 }, // Aura Sphere
    { ITEM_TM113, 6 }, // Tailwind
    { ITEM_TM109, 6 }, // Trick
    { ITEM_TM127, 6 }, // Play Rough
    { ITEM_TR50, 6 }, // Leaf Blade
    { ITEM_TM162, 6 }, // Bug Buzz
    { ITEM_TM133, 8 }, // Earth Power
    { ITEM_TM102, 8 }, // Gunk Shot
    { ITEM_TM167, 8 }, // Close Combat
    { ITEM_TM160, 8 }, // Hurricane
    { ITEM_TM142, 8 }, // Hydro Pump
    { ITEM_TM169, 8 }, // Draco Meteor
    { ITEM_TM165, 8 }, // Flare Blitz
    { ITEM_TM159, 8 }, // Leaf Storm
    { ITEM_TM156, 8 }, // Outrage
    { ITEM_TM186, 8 }, // High Horsepower
    { ITEM_TM218, 8 }, // Expanding Force
    { ITEM_TM100_SV, 8 }, // Dragon Dance
    { ITEM_TM140, 8 }, // Nasty Plot
    { ITEM_TM164, 8 }, // Brave Bird
    { ITEM_TM147, 8 }, // Wild Charge
    { ITEM_TM214, 8 }, // Sludge Wave
};

// Celadon Dept. Store 3F: the vanilla TMs, then the Kanto expansion TMs.
static const struct CoopMartEntry sCeladon3F[] = {
    { ITEM_TM021, 0 }, { ITEM_TM027, 0 }, { ITEM_TM087, 0 }, { ITEM_TM078, 0 }, { ITEM_TM012, 0 }, { ITEM_TM041, 0 },
    { ITEM_TM020, 0 }, { ITEM_TM028, 0 }, { ITEM_TM076, 0 }, { ITEM_TM055, 0 }, { ITEM_TM072, 0 }, { ITEM_TM079, 0 },
    { ITEM_TM220, 12 }, // Meteor Beam
    { ITEM_TM198, 12 }, // Poltergeist
    { ITEM_TM121, 12 }, // Heavy Slam
    { ITEM_TM095, 12 }, // Leech Life
    { ITEM_TM104, 12 }, // Iron Defense
    { ITEM_TM132, 12 }, // Baton Pass
    { ITEM_TM128, 12 }, // Amnesia
    { ITEM_TM215, 12 }, // Scorching Sands
    { ITEM_TM170, 12 }, // Steel Beam
    { ITEM_TR82, 12 }, // Stored Power
    { ITEM_TR81, 12 }, // Foul Play
    { ITEM_TR72, 12 }, // Power Whip
    { ITEM_TM171, 16 }, // Tera Blast
    { ITEM_TM190, 16 }, // Solar Blade
    { ITEM_TM174, 16 }, // Haze
    { ITEM_TM122, 16 }, // Encore
    { ITEM_TM181, 16 }, // Knock Off
    { ITEM_TR54, 16 }, // Toxic Spikes
};

// Celadon Dept. Store 4F: the Mega Stones, once the player has the Key Stone;
// tiers 2 and 3 at 4 and 8 Kanto badges. Stones of Megas without their own
// sprite are held back (docs/KNOWN_RISKS.md #45).
static const struct CoopMartEntry sCeladon4F[] = {
    { ITEM_MANECTITE, 0 },
    { ITEM_AMPHAROSITE, 0 },
    { ITEM_HOUNDOOMINITE, 0 },
    { ITEM_BANETTITE, 0 },
    { ITEM_ABSOLITE, 0 },
    { ITEM_HERACRONITE, 0 },
    { ITEM_PINSIRITE, 0 },
    { ITEM_ABOMASITE, 0 },
    { ITEM_STEELIXITE, 0 },
    { ITEM_PIDGEOTITE, 0 },
    { ITEM_BEEDRILLITE, 0 },
    { ITEM_AUDINITE, 0 },
    { ITEM_CAMERUPTITE, 0 },
    { ITEM_SHARPEDONITE, 0 },
    { ITEM_GLALITITE, 0 },
    { ITEM_ALTARIANITE, 0 },
    { ITEM_SLOWBRONITE, 0 },
    { ITEM_SABLENITE, 0 },
    { ITEM_MEGANIUMITE, 0 },
    { ITEM_FERALIGITE, 0 },
    { ITEM_SKARMORITE, 0 },
    { ITEM_DRAGONINITE, 0 },
    { ITEM_VENUSAURITE, 4 },
    { ITEM_CHARIZARDITE_X, 4 },
    { ITEM_CHARIZARDITE_Y, 4 },
    { ITEM_BLASTOISINITE, 4 },
    { ITEM_SCEPTILITE, 4 },
    { ITEM_BLAZIKENITE, 4 },
    { ITEM_SWAMPERTITE, 4 },
    { ITEM_GARDEVOIRITE, 4 },
    { ITEM_GALLADITE, 4 },
    { ITEM_LOPUNNITE, 4 },
    { ITEM_SCIZORITE, 4 },
    { ITEM_AERODACTYLITE, 4 },
    { ITEM_GYARADOSITE, 4 },
    { ITEM_MEDICHAMITE, 4 },
    { ITEM_AGGRONITE, 4 },
    { ITEM_CHIMECHITE, 4 },
    { ITEM_STARAPTITE, 4 },
    { ITEM_FROSLASSITE, 4 },
    { ITEM_EXCADRITE, 4 },
    { ITEM_SCOLIPITE, 4 },
    { ITEM_SCRAFTINITE, 4 },
    { ITEM_EELEKTROSSITE, 4 },
    { ITEM_GOLURKITE, 4 },
    { ITEM_DELPHOXITE, 4 },
    { ITEM_GRENINJITE, 4 },
    { ITEM_BARBARACITE, 4 },
    { ITEM_DRAGALGITE, 4 },
    { ITEM_HAWLUCHANITE, 4 },
    { ITEM_CLEFABLITE, 4 },
    { ITEM_VICTREEBELITE, 4 },
    { ITEM_STARMINITE, 4 },
    { ITEM_RAICHUNITE_X, 4 },
    { ITEM_RAICHUNITE_Y, 4 },
    { ITEM_ALAKAZITE, 8 },
    { ITEM_GENGARITE, 8 },
    { ITEM_LUCARIONITE, 8 },
    { ITEM_TYRANITARITE, 8 },
    { ITEM_SALAMENCITE, 8 },
    { ITEM_METAGROSSITE, 8 },
    { ITEM_GARCHOMPITE, 8 },
    { ITEM_MAWILITE, 8 },
    { ITEM_KANGASKHANITE, 8 },
    { ITEM_DRAMPANITE, 8 },
    { ITEM_FALINKSITE, 8 },
    { ITEM_GARCHOMPITE_Z, 8 },
    { ITEM_LUCARIONITE_Z, 8 },
    { ITEM_ABSOLITE_Z, 8 },
};

static void Coop_FillMart(u16 *list, const struct CoopMartEntry *entries, u32 count, u32 have)
{
    u32 i, n = 0;

    for (i = 0; i < count && n < COOP_MART_SLOTS - 1; i++)
        if (have >= entries[i].gate)
            list[n++] = entries[i].item;
    list[n] = 0xFFFF;
}

static u32 Coop_CountBits(u32 v)
{
    u32 n = 0;

    for (; v != 0; v &= v - 1)
        n++;
    return n;
}

void Coop_RefreshGatedMarts(struct FieldSystem *fsys)
{
    Coop_RefreshGatedMart(fsys, fsys->location->mapId);
}

void Coop_RefreshGatedMart(struct FieldSystem *fsys, u32 map)
{
    struct PlayerProfile *profile;
    u32 badges, kanto;

    if (map != MAP_T25R1004 && map != MAP_T25R1005 && map != MAP_T07R0103 && map != MAP_T07R0104)
        return;
    profile = Sav2_PlayerData_GetProfileAddr(fsys->savedata);
    if (profile == NULL)
        return;
    kanto = Coop_CountBits(profile->kantoBadges);
    badges = Coop_CountBits(profile->johtoBadges) + kanto;
    if (map == MAP_T25R1004)
        Coop_FillMart(sGoldenrodDepartment4F, sGoldenrod4F, NELEMS(sGoldenrod4F), badges);
    else if (map == MAP_T25R1005)
        Coop_FillMart(sGoldenrodDepartment5F, sGoldenrod5F, NELEMS(sGoldenrod5F), badges);
    else if (map == MAP_T07R0103)
        Coop_FillMart(sCeladonDepartment3F, sCeladon3F, NELEMS(sCeladon3F), badges);
    else if (CheckScriptFlag(FLAG_MEGA_EVOLUTION_ENABLED))
        Coop_FillMart(sCeladonDepartment4F, sCeladon4F, NELEMS(sCeladon4F), kanto);
    else
        sCeladonDepartment4F[0] = 0xFFFF;
}
