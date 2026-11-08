#include "config.h"
#include "types.h"

#include "constants/battle_constants.h"
#include "constants/item.h"
#include "bag.h"
#include "battle.h"
#include "pokemon.h"
#include "coop_sync.h"
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

// No player Pokemon fights in the Safari Zone or the Bug Contest, nor in the
// catching tutorial (Lyra's Marill does, Milestone 6 §3.3).
static BOOL Coop_PlayerFights(struct BattleSystem *bsys)
{
    return !(BattleTypeGet(bsys) & (BATTLE_TYPE_SAFARI | BATTLE_TYPE_BUG_CONTEST | BATTLE_TYPE_TUTORIAL));
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
    // The catching tutorial's ball is Lyra's demonstration, not the player's
    // catch: no rule applies (Milestone 6 §3.3, Monolocke blocked it).
    if (save == NULL || (type & (BATTLE_TYPE_TRAINER | BATTLE_TYPE_LINK | BATTLE_TYPE_TUTORIAL)))
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
    // The ball left the bag (or the ball count) when it was chosen: give it
    // back. Safari and Sport Balls share the battle's count (ccd
    // BattleControllerPlayer_SafariThrowBall / the Sport Ball throw; the setup
    // writes it back to the Safari counter or the contest's, KNOWN_RISKS #67).
    ctx = bsys->sp;
    if (type & (BATTLE_TYPE_SAFARI | BATTLE_TYPE_BUG_CONTEST))
        BattleSystem_SetSafariBallCount(bsys, BattleSystem_GetSafariBallCount(bsys) + 1);
    else if (ctx->item_work != ITEM_NONE)
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

// ---------------------------------------------------------------------------
// Milestone 6 §5.2: the "already met here" icon. In a wild battle the enemy's
// HP box shows a Poke Ball when the species is caught (pret battle_hp_bar.c
// ov12_02265560, HP-bar flag 0x200, which only enemy boxes of non-trainer
// battles get). That function is replaced through `hooks`: under Nuzlocke,
// when this place's first encounter is spent (the balls are blocked, or the
// shiny clause lets them through), the same tile shows a grey ball instead.
// Otherwise it does what vanilla does.
// ---------------------------------------------------------------------------

// pret BattleHpBar (include/battle/battle.h): only the fields used.
struct CoopHpBar
{
    u8 script;
    u8 pad1[3];
    struct CoopManagedSprite *boxObj;   // 0x04
    void *arrowObj;
    struct BattleSystem *battleSystem;  // 0x0C
    u8 pad10[0x25 - 0x10];
    u8 type;                            // 0x25 HPBarType
    u8 pad26[0x4B - 0x26];
    u8 caught;                          // 0x4B unk4B: 1 when the species is caught
};
_Static_assert(__builtin_offsetof(struct CoopHpBar, boxObj) == 0x04, "pret BattleHpBar.boxObj");
_Static_assert(__builtin_offsetof(struct CoopHpBar, battleSystem) == 0x0C, "pret BattleHpBar.battleSystem");
_Static_assert(__builtin_offsetof(struct CoopHpBar, type) == 0x25, "pret BattleHpBar.type");
_Static_assert(__builtin_offsetof(struct CoopHpBar, caught) == 0x4B, "pret BattleHpBar.unk4B");

// pret ManagedSprite.sprite (include/sprite.h).
struct CoopManagedSprite
{
    u8 *sprite;
};

#define COOP_SPRITE_VRAM_MAIN 0xB8       // pret Sprite.imageProxy (0xB4) .vramLocation.baseAddrOfVram[NNS_G2D_VRAM_TYPE_2DMAIN]
#define COOP_OBJ_VRAM         0x06400000 // G2_GetOBJCharPtr()
#define COOP_HP_BAR_TYPES     6          // the table covers the four single / double types; Safari and Pal Park read
                                         // the next table's { 0, 0 } (pret ov12_0226D3D8): nothing is copied

// pret ov12_0226D3C0: where the icon tile sits in each box type.
static const u16 sCoopHpIconOffset[COOP_HP_BAR_TYPES] = { 0x460, 0x420, 0x440, 0x420, 0x440, 0x420 };

// pret gBattleHpBar_RawGraphicComponents 38 (no icon) and 59 (caught), and
// the same ball with a grey top (already met here). 8x8, 4bpp.
static const u8 sCoopHpIconTiles[3][32] = {
    { 0xFF, 0xFF, 0xFF, 0xFF, 0x22, 0x22, 0x22, 0x22, 0xFF, 0xFF, 0xFF, 0xFF, 0x22, 0x22, 0x22, 0x22,
      0x22, 0x22, 0x22, 0x22, 0x22, 0x22, 0x22, 0x22, 0x22, 0x22, 0x22, 0x22, 0x22, 0x22, 0x22, 0x22 },
    { 0x2F, 0xEE, 0x2E, 0xFF, 0xE2, 0xAA, 0xEA, 0x22, 0xAE, 0xE9, 0xA9, 0xFE, 0xEE, 0x4E, 0xEE, 0x2E,
      0x4E, 0xE2, 0x42, 0x2E, 0xE3, 0x44, 0xE4, 0x23, 0x32, 0xEE, 0x3E, 0x22, 0x22, 0x22, 0x22, 0x22 },
    { 0x2F, 0xEE, 0x2E, 0xFF, 0xE2, 0x33, 0xE3, 0x22, 0x3E, 0xE3, 0x33, 0xFE, 0xEE, 0x4E, 0xEE, 0x2E,
      0x4E, 0xE2, 0x42, 0x2E, 0xE3, 0x44, 0xE4, 0x23, 0x32, 0xEE, 0x3E, 0x22, 0x22, 0x22, 0x22, 0x22 },
};

// The place's first encounter is spent (Nuzlocke): the shiny clause doesn't hide it.
BOOL LONG_CALL Coop_ChallengeAlreadyMet(struct BattleSystem *bsys)
{
    struct CoopChallengeSave *save = Coop_NuzlockeSave();

    if (save == NULL || (BattleTypeGet(bsys) & (BATTLE_TYPE_TRAINER | BATTLE_TYPE_LINK | BATTLE_TYPE_TUTORIAL)))
        return FALSE;
    return (save->pending_flags & COOP_PENDING_OPEN) && !(save->pending_flags & (COOP_PENDING_FIRST | COOP_PENDING_EXEMPT));
}

void LONG_CALL Coop_HpBar_DrawCaughtIcon(struct CoopHpBar *hpBar)
{
    const u8 *tile;
    u32 base, i, kind;
    u16 word;

    if (hpBar->type >= COOP_HP_BAR_TYPES || hpBar->boxObj == NULL || hpBar->boxObj->sprite == NULL)
        return;
    kind = Coop_ChallengeAlreadyMet(hpBar->battleSystem) ? 2 : hpBar->caught == 1 ? 1 : 0;
    tile = sCoopHpIconTiles[kind];
    __builtin_memcpy(&base, hpBar->boxObj->sprite + COOP_SPRITE_VRAM_MAIN, sizeof(base));
    base += COOP_OBJ_VRAM + sCoopHpIconOffset[hpBar->type];
#if COOP_DEBUG_TOOLS
    gCoopDebugMonBuffer.hp_icon_kind = (u16)kind;
    gCoopDebugMonBuffer.hp_icon_count++;
    gCoopDebugMonBuffer.hp_icon_addr = base;
#endif
    for (i = 0; i < 32; i += 2) // VRAM takes 16-bit writes (vanilla: MI_CpuCopy16)
    {
        word = (u16)(tile[i] | (tile[i + 1] << 8));
        *(vu16 *)(base + i) = word;
    }
}
