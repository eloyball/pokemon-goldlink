#include "config.h"
#include "types.h"

#include "message.h"
#include "window.h"
#include "coop_sync.h"
#include "coop_battle.h"
#include "coop_avatars_gen.h"

// ---------------------------------------------------------------------------
// Custom avatars, Phase 6 (docs/CUSTOM_SPRITES_PLAN.md): the PvP battle intro,
// Overlay 131.
//
// A 1v1 link battle plays the Elite Four / Champion intro (ccd
// src/field/encounter_effect_leaders.c, Overlay 115) with both players' VS
// faces: this game's on the left, the opponent's on the right, mirrored to
// face it, with the opponent's name. Vanilla runs it: only its first state,
// which loads the faces, is ours (vanilla loads the player's face by the
// save's gender and the opponent's from its const params), and two touches
// after vanilla's states 1 and 5 (the mirror, the name).
//
// The effect runs through gEncounterEffectFuncs (Overlay 1 data) at the
// Champion's index: Coop_VsIntroArm points that entry at Coop_VsIntroEffect
// just before the battle's Encounter_New, and the intro puts it back when it
// ends. Effect 33 also makes the transition task load Overlay 115. Overlay 1
// is reloaded with every map load, so an entry left armed (a battle that never
// started) is vanilla again on the next one; Lance's own battle never sees it.
// ---------------------------------------------------------------------------

// pret ov01_022068C4 (ccd gEncounterEffectFuncs), entry 33: EncounterEffect_Champion (pret ov115_02260350).
#define COOP_ENCOUNTER_EFFECT_FUNCS    0x022068C4
#define COOP_ENCOUNTER_EFFECT_CHAMPION 33
#define COOP_VANILLA_CHAMPION_EFFECT   (0x02260350 | 1)
#define COOP_HEAP_ID_FIELD1            4

// The VS faces in NARC a/1/0/9: palette, characters, cell, animation (ccd LeadersEffect_RunEliteFourIntro).
#define COOP_VS_FACE_ETHAN   207
#define COOP_VS_FACE_LYRA    211
#define COOP_VS_NAMEPLATE    54  // the Champion's (sEncounterChampionIntroParams, pret ov115_022603A8)
#define COOP_VS_SHAKE_FRAMES 9
#define COOP_AVATAR_ETHAN_ID 1
#define COOP_AVATAR_LYRA_ID  2

// ccd EncounterEffectTask (offsets checked against pret's asm: state 0x00, work 0x0C,
// fieldSystem 0x10, narc 0x20).
struct CoopEncounterEffectTask
{
    u32 state;
    BOOL fadeDone;
    u32 timer;
    u8 *work;
    void *fieldSystem;
    BOOL *done;
    BOOL wipeDone;
    u32 unk_1C;
    void *narc;
};

// ccd EliteFourIntroParams.
struct CoopEliteFourIntroParams
{
    u16 opponentMember;
    u8 nameplateMember;
    u8 shakeFrames;
    u16 unk_04;
    u16 trainerId;
};

// ccd EliteFourIntroWork, the fields used here (pret ov115_0225F978: allocates 0x314 bytes,
// the sprite manager at 0x5C, resources[0] at 0x198 (0x34 each), the window at 0x2F4,
// playerGender / playerMember at 0x304 / 0x308).
#define COOP_E4_WORK_SIZE            0x314
#define COOP_E4_WORK_SPRITE_MANAGER  0x5C
#define COOP_E4_WORK_RESOURCES       0x198
#define COOP_E4_RESOURCES_SIZE       0x34
#define COOP_E4_WORK_SPRITES         0x268
#define COOP_E4_WORK_WINDOW          0x2F4
#define COOP_E4_WORK_PLAYER_GENDER   0x304
#define COOP_E4_WORK_PLAYER_MEMBER   0x308
#define COOP_E4_STATE_CREATE_SPRITES 1
#define COOP_E4_STATE_START_SLIDE_IN 5
#define COOP_SPRITE_FLIP             0x27 // pret struct Sprite.flip (Sprite_SetFlipMode)
#define COOP_SPRITE_FLIP_H           1    // NNS_G2D_RENDERERFLIP_H

void LONG_CALL ov01_021EFCDC(struct CoopEncounterEffectTask *effect, void *task);   // EncounterEffect_Finish
void LONG_CALL ov01_021F05C4(void *spriteManager, int maxSprites, int resourceCount); // EncounterEffect_InitSprites
void LONG_CALL ov01_021F0614(void *narc, void *spriteManager, void *resources, int plttMember, int plttCount,
                             int charMember, int cellMember, int animMember, int resourceId); // EncounterEffect_LoadSpriteResources
BOOL LONG_CALL ov115_0225F978(struct CoopEncounterEffectTask *effect, u32 heapId,
                              const struct CoopEliteFourIntroParams *params); // LeadersEffect_RunEliteFourIntro

#if COOP_AVATAR_CUSTOM > 0
static const u16 sCoopAvatarVsFace[COOP_AVATAR_CUSTOM] = COOP_AVATAR_VSFACE;
#endif

// Overlay 131 has no zeroed BSS (AGENTS.md): kept in .data.
static struct
{
    struct CoopEliteFourIntroParams params;
    u16 playerMember;
    u16 name[9];
} sCoopVsIntro __attribute__((section(".data"))) = { { 0 }, 0, { 0 } };

// The face (first of its four members) of an avatar ID on a save of this gender.
static u32 Coop_VsFace(u32 avatar, u32 gender)
{
    if (avatar == COOP_AVATAR_ETHAN_ID)
        return COOP_VS_FACE_ETHAN;
    if (avatar == COOP_AVATAR_LYRA_ID)
        return COOP_VS_FACE_LYRA;
#if COOP_AVATAR_CUSTOM > 0
    if (avatar >= COOP_AVATAR_FIRST_CUSTOM_ID && avatar < COOP_AVATAR_FIRST_CUSTOM_ID + COOP_AVATAR_CUSTOM
        && sCoopAvatarVsFace[avatar - COOP_AVATAR_FIRST_CUSTOM_ID] != 0xFFFF)
        return sCoopAvatarVsFace[avatar - COOP_AVATAR_FIRST_CUSTOM_ID];
#endif
    return gender ? COOP_VS_FACE_LYRA : COOP_VS_FACE_ETHAN;
}

static void Coop_VsIntroEffect(void *task, struct CoopEncounterEffectTask *effect);

BOOL Coop_VsIntroArm(u32 opponentSlot, const u16 *name)
{
    volatile u32 *entry = (volatile u32 *)COOP_ENCOUNTER_EFFECT_FUNCS + COOP_ENCOUNTER_EFFECT_CHAMPION;
    struct CoopPlayerState opponent;
    u32 i;

    if (*entry != COOP_VANILLA_CHAMPION_EFFECT || !Coop_ReadRemotePlayer(opponentSlot, &opponent))
        return FALSE; // not the table we know, or nobody to show: vanilla's link transition
    sCoopVsIntro.params.opponentMember = (u16)Coop_VsFace(opponent.avatar_id, opponent.gender);
    sCoopVsIntro.params.nameplateMember = COOP_VS_NAMEPLATE;
    sCoopVsIntro.params.shakeFrames = COOP_VS_SHAKE_FRAMES;
    sCoopVsIntro.params.unk_04 = 0;
    sCoopVsIntro.params.trainerId = 244; // Lance's, as vanilla's: its name is replaced (Coop_VsIntroEffect)
    sCoopVsIntro.playerMember = (u16)Coop_VsFace(gCoopCommBlock.local_avatar, gCoopCommBlock.local_player.gender);
    for (i = 0; i < 8; i++)
        sCoopVsIntro.name[i] = name != NULL ? name[i] : 0xFFFF;
    sCoopVsIntro.name[8] = 0xFFFF;
    *entry = (u32)Coop_VsIntroEffect;
#if COOP_DEBUG_TOOLS
    gCoopDebugMonBuffer.vs_player = sCoopVsIntro.playerMember;
    gCoopDebugMonBuffer.vs_opponent = sCoopVsIntro.params.opponentMember;
#endif
    return TRUE;
}

// Vanilla's ELITE_FOUR_INTRO_STATE_LOAD_RESOURCES with this game's face.
static void Coop_VsIntroLoad(struct CoopEncounterEffectTask *effect)
{
    u8 *work = sys_AllocMemory(COOP_HEAP_ID_FIELD1, COOP_E4_WORK_SIZE);
    u8 *manager = work + COOP_E4_WORK_SPRITE_MANAGER;
    u8 *resources = work + COOP_E4_WORK_RESOURCES;
    u32 player = sCoopVsIntro.playerMember;
    u32 opponent = sCoopVsIntro.params.opponentMember;
    u32 gender = player == COOP_VS_FACE_LYRA;

    __builtin_memset(work, 0, COOP_E4_WORK_SIZE);
    effect->work = work;
    ov01_021F05C4(manager, 10, 4);
    ov01_021F0614(effect->narc, manager, resources, player, 1, player + 1, player + 2, player + 3, 600000);
    ov01_021F0614(effect->narc, manager, resources + COOP_E4_RESOURCES_SIZE, opponent, 1, opponent + 1,
                  opponent + 2, opponent + 3, 600001);
    ov01_021F0614(effect->narc, manager, resources + 2 * COOP_E4_RESOURCES_SIZE, COOP_VS_NAMEPLATE, 12, 48, 49, 50, 600002);
    ov01_021F0614(effect->narc, manager, resources + 3 * COOP_E4_RESOURCES_SIZE, 59, 1, 60, 61, 62, 600003);
    __builtin_memcpy(work + COOP_E4_WORK_PLAYER_GENDER, &gender, sizeof(gender));
    __builtin_memcpy(work + COOP_E4_WORK_PLAYER_MEMBER, &player, sizeof(player));
    effect->state++;
}

static void Coop_VsIntroEffect(void *task, struct CoopEncounterEffectTask *effect)
{
    u32 state = effect->state;
    u8 *sprite;
    String *string;

    if (state == 0)
    {
        Coop_VsIntroLoad(effect);
        return;
    }
    if (ov115_0225F978(effect, COOP_HEAP_ID_FIELD1, &sCoopVsIntro.params) == TRUE)
    {
        *((volatile u32 *)COOP_ENCOUNTER_EFFECT_FUNCS + COOP_ENCOUNTER_EFFECT_CHAMPION) = COOP_VANILLA_CHAMPION_EFFECT;
#if COOP_DEBUG_TOOLS
        gCoopDebugMonBuffer.vs_count++;
#endif
        ov01_021EFCDC(effect, task);
        return;
    }
    if (state == COOP_E4_STATE_CREATE_SPRITES)
    {
        // The opponent's face looks at this player's: mirrored.
        __builtin_memcpy(&sprite, effect->work + COOP_E4_WORK_SPRITES + sizeof(sprite), sizeof(sprite));
        if (sprite != NULL)
            sprite[COOP_SPRITE_FLIP] = COOP_SPRITE_FLIP_H;
    }
    else if (state == COOP_E4_STATE_START_SLIDE_IN)
    {
        // Vanilla printed the params' trainer's name: the opponent's instead.
        string = String_New(16, COOP_HEAP_ID_FIELD1);
        if (string != NULL)
        {
            CopyU16ArrayToString(string, sCoopVsIntro.name);
            FillWindowPixelBuffer(effect->work + COOP_E4_WORK_WINDOW, 0);
            AddTextPrinterParameterizedWithColor(effect->work + COOP_E4_WORK_WINDOW, 0, string, 0, 0, 0,
                                                 (1 << 16) | (2 << 8) | 0, NULL);
            String_Delete(string);
        }
    }
}
