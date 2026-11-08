#include "config.h"
#include "types.h"
#include "constants/item.h"
#include "map_events_internal.h"
#include "save.h"
#include "script.h"
#include "coop_sync.h"
#include "coop_challenge.h"

// ---------------------------------------------------------------------------
// Milestone 6 §5.1: random items, Overlay 131. Under COOP_RULE_RANDOM_ITEMS an
// item ball gives an item picked from the world seed and the ball's own event
// flag, so every player and every reload get the same one. The pool is tiered
// by badges (Johto and Kanto together; the world shares them), so a Rare Candy
// doesn't turn up on Route 30. Balls holding a TM, an HM or a key item keep
// it: the TMs are unique and the moves are part of the rebalance, and the one
// key item (the Coin Case) opens the Game Corner.
//
// Only HeartGold's own items (IDs up to 536) are in the pool: they all have
// vanilla data, icons and a pocket. No Mail, key items, plot items, fossils,
// Shards, Apricorn Balls or items without a use here.
// ---------------------------------------------------------------------------

#define COOP_SCRIPT_ITEM_BALL_FIRST 7000 // pret scr_seq_0141.s
#define COOP_SCRIPT_ITEM_BALL_LAST  7255
#define COOP_VAR_BALL_COUNT         0x8009 // VAR_SPECIAL_x8009

_Static_assert(__builtin_offsetof(LocalMapObject, evFlagId) == 0x1C, "pret LocalMapObject.eventFlag");

// 0-1 badges: the first routes.
static const u16 sTier0[] = {
    ITEM_POTION, ITEM_SUPER_POTION, ITEM_ANTIDOTE, ITEM_BURN_HEAL, ITEM_ICE_HEAL, ITEM_AWAKENING,
    ITEM_PARALYZE_HEAL, ITEM_POKE_BALL, ITEM_GREAT_BALL, ITEM_NET_BALL, ITEM_NEST_BALL, ITEM_PREMIER_BALL,
    ITEM_REPEL, ITEM_ESCAPE_ROPE, ITEM_FRESH_WATER, ITEM_ENERGY_POWDER, ITEM_HEAL_POWDER, ITEM_BERRY_JUICE,
    ITEM_ORAN_BERRY, ITEM_CHERI_BERRY, ITEM_CHESTO_BERRY, ITEM_PECHA_BERRY, ITEM_RAWST_BERRY,
    ITEM_ASPEAR_BERRY, ITEM_PERSIM_BERRY, ITEM_LEPPA_BERRY, ITEM_X_ATTACK, ITEM_X_DEFENSE, ITEM_X_SPEED,
    ITEM_X_SP_ATK, ITEM_X_SP_DEF, ITEM_X_ACCURACY, ITEM_GUARD_SPEC, ITEM_DIRE_HIT, ITEM_TINY_MUSHROOM,
    ITEM_PEARL, ITEM_HONEY,
};

// 2-4 badges.
static const u16 sTier1[] = {
    ITEM_SUPER_POTION, ITEM_FULL_HEAL, ITEM_REVIVE, ITEM_ETHER, ITEM_GREAT_BALL, ITEM_DUSK_BALL,
    ITEM_QUICK_BALL, ITEM_TIMER_BALL, ITEM_REPEAT_BALL, ITEM_DIVE_BALL, ITEM_LUXURY_BALL, ITEM_HEAL_BALL,
    ITEM_SUPER_REPEL, ITEM_SODA_POP, ITEM_LEMONADE, ITEM_MOOMOO_MILK, ITEM_ENERGY_ROOT, ITEM_REVIVAL_HERB,
    ITEM_SITRUS_BERRY, ITEM_LUM_BERRY, ITEM_FIRE_STONE, ITEM_WATER_STONE, ITEM_THUNDER_STONE,
    ITEM_LEAF_STONE, ITEM_MOON_STONE, ITEM_EVERSTONE, ITEM_CHARCOAL, ITEM_MYSTIC_WATER, ITEM_MIRACLE_SEED,
    ITEM_MAGNET, ITEM_NEVER_MELT_ICE, ITEM_BLACK_BELT, ITEM_POISON_BARB, ITEM_SOFT_SAND, ITEM_SHARP_BEAK,
    ITEM_TWISTED_SPOON, ITEM_SILVER_POWDER, ITEM_HARD_STONE, ITEM_SPELL_TAG, ITEM_DRAGON_FANG,
    ITEM_BLACK_GLASSES, ITEM_METAL_COAT, ITEM_SILK_SCARF, ITEM_QUICK_CLAW, ITEM_SMOKE_BALL,
    ITEM_BRIGHT_POWDER, ITEM_WHITE_HERB, ITEM_MENTAL_HERB, ITEM_SOOTHE_BELL, ITEM_BIG_MUSHROOM,
    ITEM_STARDUST, ITEM_BIG_PEARL,
};

// 5-7 badges.
static const u16 sTier2[] = {
    ITEM_HYPER_POTION, ITEM_MAX_REPEL, ITEM_MAX_ETHER, ITEM_ELIXIR, ITEM_ULTRA_BALL, ITEM_HP_UP,
    ITEM_PROTEIN, ITEM_IRON, ITEM_CARBOS, ITEM_CALCIUM, ITEM_ZINC, ITEM_PP_UP, ITEM_RARE_CANDY,
    ITEM_SUN_STONE, ITEM_SHINY_STONE, ITEM_DUSK_STONE, ITEM_DAWN_STONE, ITEM_OVAL_STONE, ITEM_KINGS_ROCK,
    ITEM_DRAGON_SCALE, ITEM_UP_GRADE, ITEM_PROTECTOR, ITEM_ELECTIRIZER, ITEM_MAGMARIZER,
    ITEM_DUBIOUS_DISC, ITEM_REAPER_CLOTH, ITEM_RAZOR_CLAW, ITEM_RAZOR_FANG, ITEM_DEEP_SEA_TOOTH,
    ITEM_DEEP_SEA_SCALE, ITEM_SCOPE_LENS, ITEM_WIDE_LENS, ITEM_ZOOM_LENS, ITEM_MUSCLE_BAND,
    ITEM_WISE_GLASSES, ITEM_SHELL_BELL, ITEM_FOCUS_BAND, ITEM_EXP_SHARE, ITEM_AMULET_COIN, ITEM_LIGHT_CLAY,
    ITEM_POWER_HERB, ITEM_BIG_ROOT, ITEM_SHED_SHELL, ITEM_ICY_ROCK, ITEM_SMOOTH_ROCK, ITEM_HEAT_ROCK,
    ITEM_DAMP_ROCK, ITEM_NUGGET, ITEM_STAR_PIECE, ITEM_HEART_SCALE, ITEM_OCCA_BERRY, ITEM_PASSHO_BERRY,
    ITEM_WACAN_BERRY, ITEM_RINDO_BERRY, ITEM_YACHE_BERRY, ITEM_CHOPLE_BERRY, ITEM_SHUCA_BERRY,
    ITEM_LIECHI_BERRY, ITEM_SALAC_BERRY, ITEM_PETAYA_BERRY,
};

// 8 badges and more.
static const u16 sTier3[] = {
    ITEM_FULL_RESTORE, ITEM_MAX_POTION, ITEM_MAX_REVIVE, ITEM_MAX_ELIXIR, ITEM_FULL_HEAL, ITEM_ULTRA_BALL,
    ITEM_PP_MAX, ITEM_RARE_CANDY, ITEM_HP_UP, ITEM_PROTEIN, ITEM_IRON, ITEM_CARBOS, ITEM_CALCIUM,
    ITEM_ZINC, ITEM_LEFTOVERS, ITEM_LIFE_ORB, ITEM_CHOICE_BAND, ITEM_CHOICE_SPECS, ITEM_CHOICE_SCARF,
    ITEM_LUCKY_EGG, ITEM_EXPERT_BELT, ITEM_FOCUS_SASH, ITEM_BLACK_SLUDGE, ITEM_TOXIC_ORB, ITEM_FLAME_ORB,
    ITEM_SACRED_ASH, ITEM_MASTER_BALL, ITEM_NUGGET, ITEM_STAR_PIECE, ITEM_LANSAT_BERRY,
    ITEM_STARF_BERRY,
};

static const struct
{
    const u16 *items;
    u16 count;
} sTiers[] = {
    { sTier0, NELEMS(sTier0) },
    { sTier1, NELEMS(sTier1) },
    { sTier2, NELEMS(sTier2) },
    { sTier3, NELEMS(sTier3) },
};

static u32 Coop_ItemMix(u32 x)
{
    x ^= x >> 16;
    x *= 0x7FEB352Du;
    x ^= x >> 15;
    x *= 0x846CA68Bu;
    x ^= x >> 16;
    return x;
}

static u32 Coop_ItemBadges(FieldSystem *fsys)
{
    struct PlayerProfile *profile = Sav2_PlayerData_GetProfileAddr(fsys->savedata);
    u32 bits, n = 0;

    if (profile == NULL)
        return 0;
    bits = profile->johtoBadges | ((u32)profile->kantoBadges << 8);
    for (; bits != 0; bits &= bits - 1)
        n++;
    return n;
}

u32 Coop_ChallengeBallItem(FieldSystem *fsys, u32 item)
{
    struct CoopChallengeSave *save = Coop_ChallengeSave();
    u16 *script;
    LocalMapObject **talked;
    u32 badges, tier, h, key;

    if (save == NULL || !(save->rules & COOP_RULE_RANDOM_ITEMS) || item == ITEM_NONE || item >= ITEM_TM001)
        return item; // no rule, or a TM, an HM or a key item
    script = FieldSysGetAttrAddr(fsys, SCRIPTENV_SCRIPT);
    if (script == NULL || *script < COOP_SCRIPT_ITEM_BALL_FIRST || *script > COOP_SCRIPT_ITEM_BALL_LAST)
        return item; // not an item ball
    // Several balls share one script: the ball's own flag tells them apart.
    talked = FieldSysGetAttrAddr(fsys, SCRIPTENV_LAST_TALKED);
    key = talked != NULL && *talked != NULL ? (u32)(*talked)->evFlagId : *script;

    badges = Coop_ItemBadges(fsys);
    tier = badges < 2 ? 0 : badges < 5 ? 1 : badges < 8 ? 2 : 3;
    h = Coop_ItemMix(save->seed ^ Coop_ItemMix(key + 0x17E3u));
    if (tier > 0 && (h & 3) == 0)
        tier = (h >> 2) % tier; // one ball in four comes from a lower tier
#if COOP_DEBUG_TOOLS
    gCoopDebugMonBuffer.ball_vanilla = (u16)item;
    gCoopDebugMonBuffer.ball_key = (u16)key;
    gCoopDebugMonBuffer.ball_count++;
#endif
    item = sTiers[tier].items[(h >> 8) % sTiers[tier].count];
#if COOP_DEBUG_TOOLS
    gCoopDebugMonBuffer.ball_result = (u16)item;
#endif
    VarSet(fsys, COOP_VAR_BALL_COUNT, 1); // copied into 0x8005 next: one, never a pile
    return item;
}
