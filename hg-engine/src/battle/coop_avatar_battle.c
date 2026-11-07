// Custom avatars in battle (docs/CUSTOM_SPRITES_PLAN.md Phase 5): the trainer
// pictures (the throw, the link opponent's front) in each player's avatar.
// Battle extension, loaded with overlay 12. ccd names, pret labels.
//
// A player's avatar ID rides in their Trainer.loseMessage (Frontier-only),
// stamped by the co-op code when it builds a link battle (coop_ghosts.c
// Coop_StampBattleAvatar), so the link exchange of Trainer structs gives every
// game every player's avatar; outside link battles the local player's own
// battler uses the mailbox's local_avatar. Pictures: a custom avatar's battler
// gets a class that leads vanilla's file lookup to its pictures: the front
// set's index (GetTrainerPicFileIds multiplies the class by 5), or the
// reserved 0xE0 + k for a back picture (Coop_TrainerClassToBackpicID).

#include "config.h"
#include "types.h"

#include "constants/battle_constants.h"
#include "battle.h"
#include "trainer_data.h"
#include "coop_sync.h"
#include "coop_avatars_gen.h"
#include "coop_boss.h"

struct ManagedSprite;

// ARM9 (pret sub_02070C24, ccd CreateTrainerPicSprite).
struct ManagedSprite *LONG_CALL CreateTrainerPicSprite(void *spriteSystem, void *spriteManager, void *palette, int x, int y,
                                                       int trainerClass, int battlerPosition, BOOL isLink, int resTag,
                                                       u32 heapID);
// ARM9 (pret TrainerClass_GetGenderOrTrainerCount, 0x020735F8).
u32 LONG_CALL TrainerClass_GetGenderOrTrainerCount(int trainerClass);

// pret BattleSystem (battle.h in ccd): hg-engine's names for these fields.
_Static_assert(__builtin_offsetof(struct BattleSystem, palette) == 0x28, "BattleSystem.palette");
_Static_assert(__builtin_offsetof(struct BattleSystem, battleType) == 0x2C, "BattleSystem.battleType");
_Static_assert(__builtin_offsetof(struct BattleSystem, opponentData) == 0x34, "BattleSystem.opponentData");
_Static_assert(__builtin_offsetof(struct BattleSystem, playerProfile) == 0x48, "BattleSystem.playerProfile");
_Static_assert(__builtin_offsetof(struct BattleSystem, unk90) == 0x90, "BattleSystem.spriteRenderer (pret)");
_Static_assert(__builtin_offsetof(struct BattleSystem, unk94) == 0x94, "BattleSystem.gfxHandler (pret)");
_Static_assert(sizeof(Trainer) == 0x34, "Trainer (pret trainer_data.h)");
_Static_assert(__builtin_offsetof(Trainer, loseMessage) == COOP_AVATAR_CARRIER_OFFSET, "Trainer.loseMessage");

#define COOP_OPPONENT_BATTLER_ID   0x194 // pret OpponentData.battlerId
#define COOP_OPPONENT_BATTLER_TYPE 0x195 // pret OpponentData.battlerType
#define COOP_PROFILE_VERSION       0x1C  // pret PlayerProfile.version
#define COOP_HEAP_ID_BATTLE        5
#define COOP_AVATAR_BACK_CLASS     0xE0  // reserved: 0xE0 + k = custom avatar k's back picture

// GetTrainerPicFileIds (pret sub_02070D3C, up to sub_02070D84): calls from
// there want the picture's files; the others (the throw's ball offsets,
// gBallThrowOffsets[backpic]) want Ethan's back picture ID.
#define COOP_GET_TRAINER_PIC_FILE_IDS     0x02070D3C
#define COOP_GET_TRAINER_PIC_FILE_IDS_END 0x02070D84

#if COOP_AVATAR_CUSTOM > 0
static const u16 sCoopAvatarPics[COOP_AVATAR_CUSTOM][3] = COOP_AVATAR_PICS;
#endif

static u32 Coop_TrainerCarrier(Trainer *trainer)
{
    u16 carrier[2];

    __builtin_memcpy(carrier, (const u8 *)trainer + COOP_AVATAR_CARRIER_OFFSET, sizeof(carrier));
    return carrier[0] == COOP_AVATAR_CARRIER_MAGIC ? carrier[1] : 0;
}

// The battler's avatar ID; 0: vanilla.
static u32 Coop_BattlerAvatar(struct BattleSystem *bsys, int battlerId)
{
    Trainer *trainer = BattleSystem_GetTrainer(bsys, battlerId);
    u32 avatar = Coop_TrainerCarrier(trainer);

    if (avatar != 0)
        return avatar;
    // Outside link battles the player's own trainer (not an AI partner's).
    if (!(bsys->battleType & BATTLE_TYPE_LINK) && !(battlerId & 1) && trainer == BattleSystem_GetTrainer(bsys, 0))
        return gCoopCommBlock.local_avatar;
    return 0;
}

static const u8 *Coop_OpponentData(struct BattleSystem *bsys, int battlerId, int battlerType)
{
    u32 i;

    for (i = 0; i < 4; i++)
    {
        const u8 *data = (const u8 *)bsys->opponentData[i];

        if (data == NULL)
            continue;
        if ((battlerId >= 0 && data[COOP_OPPONENT_BATTLER_ID] == battlerId)
            || (battlerType >= 0 && data[COOP_OPPONENT_BATTLER_TYPE] == battlerType))
            return data;
    }
    return NULL;
}

// Overlay-12 hook (hooks: ov12_02261EF0, ccd
// BattleControllerOpponent_GetVersionTrainerClass; replaced): vanilla's class
// swap, after the battler's avatar: Ethan / Lyra's class, or a custom
// avatar's picture class (Ethan's look where the column wasn't drawn).
u8 Coop_GetVersionTrainerClass(struct BattleSystem *bsys, int battlerId, u8 trainerClass)
{
    u32 avatar = Coop_BattlerAvatar(bsys, battlerId);
    u32 version;

    if (avatar == 1 || avatar == 2)
        trainerClass = (u8)(avatar - 1);
#if COOP_AVATAR_CUSTOM > 0
    else if (avatar >= COOP_AVATAR_FIRST_CUSTOM_ID && avatar < COOP_AVATAR_FIRST_CUSTOM_ID + COOP_AVATAR_CUSTOM)
    {
        u32 k = avatar - COOP_AVATAR_FIRST_CUSTOM_ID;
        const u8 *data = Coop_OpponentData(bsys, battlerId, -1);
        BOOL enemy = data != NULL && (data[COOP_OPPONENT_BATTLER_TYPE] & 1);

        if (enemy && sCoopAvatarPics[k][0] != 0xFFFF)
            return (u8)sCoopAvatarPics[k][0];
        if (!enemy && sCoopAvatarPics[k][1] != 0xFFFF)
            return (u8)(COOP_AVATAR_BACK_CLASS + k);
        trainerClass = 0;
    }
#endif
    if (!(bsys->battleType & BATTLE_TYPE_LINK) || (trainerClass != 0 && trainerClass != 1))
        return trainerClass;
    version = ((const u8 *)bsys->playerProfile[battlerId])[COOP_PROFILE_VERSION];
    if (version == 0)
        trainerClass += 125;
    else if (version == 12)
        trainerClass += 127;
    return trainerClass;
}

// ARM9 hook (hooks: 0207280C, pret TrainerClassToBackpicID = sub_0207280C;
// replaced). Reached only from battle code: GetTrainerPicFileIds' back path
// (whose other callers ask for fronts) and the throw. Vanilla's answer by
// class range; for a custom avatar's back class, its picture's set when
// GetTrainerPicFileIds asks, else Ethan's (the ball offsets follow the
// template's frames, which are Ethan's).
int Coop_TrainerClassToBackpicID(int trainerClass, BOOL isLink)
{
#if COOP_AVATAR_CUSTOM > 0
    if (trainerClass >= COOP_AVATAR_BACK_CLASS && trainerClass < COOP_AVATAR_BACK_CLASS + COOP_AVATAR_CUSTOM)
    {
        u32 from = (u32)__builtin_return_address(0);
        const u16 *pics = sCoopAvatarPics[trainerClass - COOP_AVATAR_BACK_CLASS];

        if (from >= COOP_GET_TRAINER_PIC_FILE_IDS && from < COOP_GET_TRAINER_PIC_FILE_IDS_END)
            return isLink && pics[2] != 0xFFFF ? pics[2] : pics[1];
        trainerClass = 0;
    }
#endif
    if (trainerClass == 0 || trainerClass == 1) // Ethan, Lyra
        return isLink ? trainerClass + 15 : trainerClass;
    if (trainerClass == 23) // rival
        return 2;
    if (trainerClass == 125 || trainerClass == 126) // Lucas / Dawn (DP)
        return trainerClass - 125 + 9;
    if (trainerClass == 123) // Lance
        return 3;
    if (trainerClass >= 90 && trainerClass <= 94) // Cheryl ... Marley
        return trainerClass - 90 + 4;
    if (trainerClass == 127 || trainerClass == 128) // Lucas / Dawn (Pt)
        return trainerClass - 127 + 12;
    return TrainerClass_GetGenderOrTrainerCount(trainerClass) == TRAINER_FEMALE ? 1 : 0;
}

// A co-op team boss battle (src/coop_boss.c COOP_BOSS_BATTLE_TYPE: the Battle Tower's link multi,
// which co-op uses for nothing else).
static BOOL Coop_IsTeamBossBattle(struct BattleSystem *bsys)
{
    return (bsys->battleType & (BATTLE_TYPE_FRONTIER | BATTLE_TYPE_LINK | BATTLE_TYPE_MULTI))
               == (BATTLE_TYPE_FRONTIER | BATTLE_TYPE_LINK | BATTLE_TYPE_MULTI)
        && gCoopBoss.fought;
}

// The enemy side's second seat (ccd BATTLER_TYPE_ENEMY_SIDE_SLOT_2), the resource tag of its
// trainer sprite. pret struct ManagedSprite starts with its Sprite *.
#define COOP_BATTLER_TYPE_ENEMY_SLOT_2 5
void LONG_CALL Sprite_SetDrawFlag(void *sprite, BOOL flag); // ARM9 (pret sprite.c)

// ccd BattleMessage: the tag, the message ID, then the tag's parameters (battlers).
struct CoopBattleMessage
{
    u8 commandType;
    u8 tag;
    u16 id;
    int param[6];
};

// Overlay-12 hook (hooks: ov12_02261544, ccd BattleControllerOpponent_GetEncounterMessage;
// replaced): vanilla's choice, except that a team boss (in both enemy seats) is greeted as
// one trainer, "X would like to battle!", not "X and X..." (KNOWN_RISKS #35).
void Coop_GetEncounterMessage(struct BattleSystem *bsys, void *opponentData, struct CoopBattleMessage *msg)
{
    u32 type = BattleTypeGet(bsys);
    int enemy1, enemy2;

    (void)opponentData;
    if (type & BATTLE_TYPE_DOUBLES)
    {
        enemy1 = BattleWorkClientNoGet(bsys, 3); // BATTLER_TYPE_ENEMY_SIDE_SLOT_1
        enemy2 = BattleWorkClientNoGet(bsys, 5); // BATTLER_TYPE_ENEMY_SIDE_SLOT_2
    }
    else
        enemy1 = enemy2 = BattleWorkClientNoGet(bsys, 1); // BATTLER_TYPE_SOLO_ENEMY
    if ((type & BATTLE_TYPE_LINK) && !Coop_IsTeamBossBattle(bsys))
    {
        if (type & BATTLE_TYPE_FRONTIER)
        {
            msg->id = 992; // msg_0197_00992: two trainers (class and name each)
            msg->tag = 59;
            msg->param[0] = enemy1;
            msg->param[1] = enemy1;
            msg->param[2] = enemy2;
            msg->param[3] = enemy2;
        }
        else if (type & BATTLE_TYPE_MULTI)
        {
            msg->id = 971; // two link players
            msg->tag = 26;
            msg->param[0] = enemy1;
            msg->param[1] = enemy2;
        }
        else
        {
            msg->id = 970; // a link player
            msg->tag = 8;  // TAG_TRNAME
            msg->param[0] = enemy1;
        }
    }
    else if (!Coop_IsTeamBossBattle(bsys) && (type & (BATTLE_TYPE_TAG | BATTLE_TYPE_MULTI)))
    {
        msg->id = 992;
        msg->tag = 59;
        msg->param[0] = enemy1;
        msg->param[1] = enemy1;
        msg->param[2] = enemy2;
        msg->param[3] = enemy2;
    }
    else
    {
        msg->id = 969; // msg_0197_00969: one trainer
        msg->tag = 30;
        msg->param[0] = enemy1;
        msg->param[1] = enemy1;
    }
}

// Overlay-12 hook (hooks: ov12_02261B2C, ccd
// BattleControllerOpponent_CreateTrainerSprite; replaced). The encounter
// passes the class from GetVersionTrainerClass, the slide-in the raw one: an
// avatar's battler gets its picture class either way.
struct ManagedSprite *Coop_CreateTrainerSprite(struct BattleSystem *bsys, int position, int trainerClass, int resTag,
                                               BOOL isLink, s16 x, s16 y)
{
    const u8 *data = Coop_OpponentData(bsys, -1, resTag);
    struct ManagedSprite *managed;
    void *sprite;

    if (data != NULL && Coop_BattlerAvatar(bsys, data[COOP_OPPONENT_BATTLER_ID]) != 0)
        trainerClass = Coop_GetVersionTrainerClass(bsys, data[COOP_OPPONENT_BATTLER_ID], (u8)trainerClass);
#if COOP_DEBUG_TOOLS
    if (resTag >= 0 && resTag < 4)
    {
        // avatar probes: per battler type, the class drawn (232 + type), the battler ID
        // (224 + type) and the carrier found on its trainer (228 + type)
        gCoopDebugMonBuffer.mon[232 + resTag] = (u8)trainerClass;
        gCoopDebugMonBuffer.mon[224 + resTag] = data != NULL ? data[COOP_OPPONENT_BATTLER_ID] : 0xEE;
        gCoopDebugMonBuffer.mon[228 + resTag] =
            data != NULL ? (u8)Coop_TrainerCarrier(BattleSystem_GetTrainer(bsys, data[COOP_OPPONENT_BATTLER_ID])) : 0xEE;
    }
#endif
    managed = CreateTrainerPicSprite(bsys->unk90, bsys->unk94, bsys->palette, x, y, trainerClass, (position & 1) ? 2 : 0,
                                     isLink, resTag, COOP_HEAP_ID_BATTLE);
    // A team boss sits in both enemy seats (KNOWN_RISKS #35): its second seat's sprite is
    // created as vanilla expects (moved, animated, deleted with the first) but never drawn.
    if (managed != NULL && resTag == COOP_BATTLER_TYPE_ENEMY_SLOT_2 && Coop_IsTeamBossBattle(bsys))
    {
        __builtin_memcpy(&sprite, managed, sizeof(sprite));
        if (sprite != NULL)
            Sprite_SetDrawFlag(sprite, FALSE);
    }
    return managed;
}
