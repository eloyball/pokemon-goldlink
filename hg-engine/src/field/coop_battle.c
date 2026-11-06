#include "config.h"
#include "types.h"

#include "constants/battle_constants.h"
#include "constants/buttons.h"

#include "battle.h"
#include "battle_controller_player.h"
#include "map_events_internal.h"
#include "message.h"
#include "pokemon.h"
#include "system.h"
#include "task.h"
#include "window.h"
#include "coop_sync.h"
#include "coop_wild.h"
#include "coop_dialog.h"
#include "coop_choice.h"
#include "constants/file.h"
#include "coop_battle.h"
#include "coop_boss.h"
#include "coop_link.h"
#include "save.h"
#include "coop_rewards.h"
#include "coop_scene.h"

// ---------------------------------------------------------------------------
// Link battles (docs/MILESTONE3_PLAN.md §17), Overlay 131.
//
// An accepted Battle request turns the session task into this one on both
// games. They open a vanilla local-wireless session over the virtual link
// (src/coop_link.c): the requester as the parent, the other as the child,
// both with a beacon id from the requester's player id so the child only
// lists its partner. Once linked and synced, both run vanilla's link battle
// (Task_LinkBattle: transition, battle, back to the field) with their whole
// party; the battle runs on copies, so nothing in the save changes but the
// trainer card's record and the Pokedex. Then the session shuts down and
// the result is shown.
//
// A team battle (2v2, §19) is the Colosseum's 4-player multi: the requester
// hosts, the other three join with the same beacon id, and the relay gives
// the requester's teammate aid 2 so teammates share a side (aids 0 and 2 vs
// 1 and 3). Each player brings their first three Pokemon.
// ---------------------------------------------------------------------------

#define COOP_COMM_TYPE_SINGLE_BATTLE 1 // ccd constants/comm_type.h
#define COOP_COMM_TYPE_MULTI_BATTLE  4 // a team battle: the Colosseum's 4-player multi
#define COOP_COMM_TYPE_BATTLE_TOWER  16 // a team boss: the Battle Tower's link multi
#define COOP_BOSS_BEACON_BASE        16 // team boss beacon ids: 16 + lower id * 6 + higher id (PvP: 1-6)
#define COOP_HEAP_ID_FIELD2          11
#define COOP_BATTLE_SYNC             0x61 // LinkSync value both games reach before the battle
#define COOP_FPS                     30 // game logic (field tasks) runs every other frame (ccd main.c)
#define COOP_FIND_FAILSAFE           (20 * COOP_FPS) // child: the parent's beacon
#define COOP_LINK_FAILSAFE           (30 * COOP_FPS) // both linked and synced
#define COOP_TEAM_LINK_FAILSAFE      (90 * COOP_FPS) // a team battle: the others may still be choosing their three
#define COOP_SHUTDOWN_FAILSAFE       (10 * COOP_FPS)

void LONG_CALL CommState_StartParentSession(void *saveData, s8 linkMode, s8 beaconId, void *rules, u32 unused);
void LONG_CALL CommState_StartChildSession(void *saveData, s8 linkMode, s8 beaconId, void *rules, u32 unused);
void LONG_CALL CommState_ConnectToSlot(s8 slot);
void LONG_CALL CommState_RequestShutdown(void);
BOOL LONG_CALL CommState_Exists(void);
BOOL LONG_CALL CommState_IsLinked(void);
void LONG_CALL LinkSync_StartSync(u8 state);
BOOL LONG_CALL LinkSync_IsSynced(u8 state);
int LONG_CALL CommWireless_CountParents(void);
int LONG_CALL Comm_CountConnected(void);
// Vanilla's link battle (pret src/encounter.c CallTask_020508B8 /
// Task_020508B8; ccd CallTask_LinkBattle / Task_LinkBattle).
void LONG_CALL BattleSetup_InitForLinkBattle(struct BattleSetup *setup, FieldSystem *fsys, void *partySlots);
void *LONG_CALL Encounter_New(struct BattleSetup *setup, s32 effect, s32 bgm, u32 *winFlag);
BOOL LONG_CALL Task_LinkBattle(TaskManager *taskman);
void LONG_CALL CallTask_FadeFromBlack(TaskManager *taskman);
// pret src/script_manager.c: runs a script as a subtask of this task.
void LONG_CALL QueueScript(TaskManager *taskman, u16 script, LocalMapObject *obj, void *result);
void LONG_CALL PokeParty_Init(struct Party *party, int max);
// Overlay 01 (rom.ld): the field's normal save (blocks until written).
BOOL LONG_CALL Field_SaveGameNormal(FieldSystem *fsys);

#define COOP_MOVE_NAME_BUF  20 // 15 characters, EOS, and 0xFFFF padding up to the split
#define COOP_MOVE_NAME_FILE 750 // a027 message file: move names (hg-engine armips/include/movemacros.s)
#define COOP_ARG_SPLIT      11 // Coop_SetStringFromAscii prints at most 11 characters per argument
#define COOP_MSG_HEAP_ID    32 // HEAP_ID_FIELD3, the field task heap
#define COOP_DEBUG_MOVES_OFFSET 0x60 // probes: the moves after a forget-a-move answer
_Static_assert(sizeof(struct CoopRewards) <= COOP_DEBUG_MOVES_OFFSET, "the probe's moves follow the rewards summary");
_Static_assert(__builtin_offsetof(struct CoopRewards, pendingMove) == 42 && __builtin_offsetof(struct CoopRewards, pendingCount) == 78,
               "link_e2e_probe.lua reads these");

enum CoopBattleTaskState
{
    COOP_BATTLE_OPEN,        // a 2v2: the party screen to choose three first
    COOP_BATTLE_CHOSEN,      // back from the party screen
    COOP_BATTLE_START,
    COOP_BATTLE_PRESAVE,     // save before linking up (a crash or disconnect loses nothing)
    COOP_BATTLE_BOSS_WAIT,   // team boss: until the teammate waits at the same boss
    COOP_BATTLE_BOSS_ALONE,  // B while waiting: "Battle alone instead?" Yes / No
    COOP_BATTLE_BOSS_ALONE_DONE, // the boss's script ran (a solo battle)
    COOP_BATTLE_START_LINK,
    COOP_BATTLE_FIND_PARENT, // child: until the scan lists the parent
    COOP_BATTLE_WAIT_LINKED,
    COOP_BATTLE_WAIT_SYNC,
    COOP_BATTLE_FIGHT,       // Task_LinkBattle runs (the overworld is left meanwhile)
    COOP_BATTLE_SHUTDOWN,
    COOP_BATTLE_RESULT,
    COOP_BATTLE_FINAL_TEXT,
    COOP_BATTLE_FINAL_BUTTON,
    COOP_BATTLE_BOSS_SCRIPT, // team boss: the boss's script ran (and its battle, src/coop_boss.c)
    COOP_BATTLE_BOSS_CLOSE,
    COOP_BATTLE_REWARD_PAGE,    // team boss won: the next reward page (§20)
    COOP_BATTLE_REWARD_WAIT,
    COOP_BATTLE_REWARD_EVOLVE,  // the next level-up evolution, then the next
    COOP_BATTLE_MOVE_NEXT,      // the next move a Pokemon with four could not learn
    COOP_BATTLE_MOVE_PAGE,      // a page of the prompt: wait, then env->afterPage
    COOP_BATTLE_MOVE_ASK,       // "Delete a move to make room?" Yes / No
    COOP_BATTLE_MOVE_FORGOT,    // back from the summary screen
    COOP_BATTLE_MOVE_STOP,      // "Stop trying to teach it?" Yes / No
    COOP_BATTLE_POSTSAVE,       // save the battle's outcome, then end
    COOP_BATTLE_SAVED,          // a frame for the spinner to restore the frame, then end
};

struct CoopBattleEnv
{
    struct CoopDialog dialog;
    u16 name[9];   // the other player, 0xFFFF-terminated
    u32 winFlag;   // BATTLE_OUTCOME_*, written by Task_LinkBattle
    u16 frames;
    u8 parent;
    u8 beaconId;
    u8 failed;     // the link never came up
    u8 fought;
    u8 lost;       // the link dropped in the battle (Coop_LinkBattleWatchdog)
    u8 team;       // a 2v2 between two teams
    u8 saved;      // the save after the battle was made (or tried)
    // Team boss (docs/MILESTONE3_PLAN.md §18).
    u8 boss;
    u8 bossIndex;  // sGatedBosses index
    u8 peerSlot;   // the teammate's (team boss) or the opponent's (1v1) remote slot
    u16 bossScript;
    LocalMapObject *bossObj;
    int bossObjId;     // to find it again after the party screen (the overworld is rebuilt)
    u8 order[6];       // the three this player brings to a 2v2 (party slots, 1-based)
    // Team boss rewards (§20).
    struct CoopRewards rewards;
    u8 rewardPage;     // enum CoopRewardPage
    u8 rewardSlot;     // the party slot the level pages are at
    u16 nick[12];      // its nickname, for '@'
    char text[72];     // the page being printed (Coop_DialogPrint keeps the pointer)
    // The forget-a-move prompt (2026-11-02). A move name is up to 15
    // characters and a dialog argument up to 11, so a name prints as '&^'
    // (args 2 and 3: its start and the rest).
    struct CoopChoice choice;
    u16 moveName[COOP_MOVE_NAME_BUF];
    u16 forgotName[COOP_MOVE_NAME_BUF];
    u8 forgetSlot;     // the summary screen's answer
    u8 afterPage;      // COOP_BATTLE_MOVE_PAGE goes on here
};

enum CoopRewardPage
{
    COOP_REWARD_PAGE_MONEY,
    COOP_REWARD_PAGE_EXP,
    COOP_REWARD_PAGE_LEVEL,    // per leveled slot: its level and a learned move
    COOP_REWARD_PAGE_NO_ROOM,  // the same slot: a move it could not learn
    COOP_REWARD_PAGE_DONE,
};

// Appends text / a number to an ASCII page.
static char *Coop_AppendText(char *dst, const char *src)
{
    while (*src != '\0')
        *dst++ = *src++;
    *dst = '\0';
    return dst;
}

static char *Coop_AppendNumber(char *dst, u32 n)
{
    char digits[10];
    u32 len = 0;

    do
    {
        digits[len++] = (char)('0' + n % 10);
        n /= 10;
    } while (n != 0 && len < sizeof(digits));
    while (len != 0)
        *dst++ = digits[--len];
    *dst = '\0';
    return dst;
}

// The next reward page into env->text, or FALSE when all are shown.
static BOOL Coop_RewardNextPage(FieldSystem *fsys, struct CoopBattleEnv *env)
{
    struct CoopRewards *r = &env->rewards;
    char *p = env->text;

    for (;;)
    {
        switch (env->rewardPage)
        {
        case COOP_REWARD_PAGE_MONEY:
            env->rewardPage = COOP_REWARD_PAGE_EXP;
            if (r->money == 0)
                continue;
            p = Coop_AppendText(p, "You got ");
            p = Coop_AppendNumber(p, r->money);
            Coop_AppendText(p, " in prize\nmoney for winning!");
            return TRUE;
        case COOP_REWARD_PAGE_EXP:
            env->rewardPage = COOP_REWARD_PAGE_LEVEL;
            env->rewardSlot = 0;
            if (r->gainedExp == 0)
                continue;
            Coop_AppendText(p, "Your Pokémon gained\nEXP. Points!");
            return TRUE;
        case COOP_REWARD_PAGE_LEVEL:
            while (env->rewardSlot < 6 && !(r->leveled & (1u << env->rewardSlot)))
                env->rewardSlot++;
            if (env->rewardSlot >= 6)
            {
                env->rewardPage = COOP_REWARD_PAGE_DONE;
                continue;
            }
            GetMonData(Party_GetMonByIndex(SaveData_GetPlayerPartyPtr(fsys->savedata), env->rewardSlot), MON_DATA_NICKNAME, env->nick);
            env->nick[11] = 0xFFFF;
            p = Coop_AppendText(p, "@ grew to Lv. ");
            p = Coop_AppendNumber(p, r->newLevel[env->rewardSlot]);
            Coop_AppendText(p, (r->learned & (1u << env->rewardSlot)) ? "!\n@ learned a new move!" : "!");
            env->rewardPage = COOP_REWARD_PAGE_NO_ROOM;
            return TRUE;
        case COOP_REWARD_PAGE_NO_ROOM:
            // A move it could not learn is offered after the pages
            // (COOP_BATTLE_MOVE_NEXT).
            env->rewardPage = COOP_REWARD_PAGE_LEVEL;
            env->rewardSlot++;
            continue;
        default:
            return FALSE;
        }
    }
}

static BOOL Coop_BattleEnd(FieldSystem *fsys, struct CoopBattleEnv *env)
{
    Coop_DialogClose(fsys, &env->dialog);
    MapObjectMan_UnpauseAllMovement(fsys->mapObjectMan);
    sys_FreeMemoryEz(env);
    return TRUE;
}

static void Coop_BattleOpenDialog(FieldSystem *fsys, struct CoopBattleEnv *env)
{
    MapObjectMan_PauseAllMovement(fsys->mapObjectMan);
    Coop_DialogOpen(fsys, &env->dialog);
    env->dialog.args[0] = env->name;
}

// The link did not come up (or the player cancelled): close it, then say so.
static void Coop_BattleFail(TaskManager *taskman, struct CoopBattleEnv *env)
{
    env->failed = TRUE;
    if (env->boss)
        gCoopCommBlock.track_boss_wait = 0;
    if (CommState_Exists())
        CommState_RequestShutdown();
    env->frames = 0;
    taskman->state = COOP_BATTLE_SHUTDOWN;
}

static const char *Coop_TeamBattleResultText(const struct CoopBattleEnv *env)
{
    if (env->failed)
        return "Couldn't link up with\neveryone. The battle was cancelled.";
    if (env->lost)
        return "The link with the others\nwas lost. The battle has ended.";
    switch (env->winFlag & 0xF)
    {
    case BATTLE_OUTCOME_WIN:    return "Your team won the battle!";
    case BATTLE_OUTCOME_LOSE:   return "Your team lost the battle.";
    case BATTLE_OUTCOME_DRAW:   return "The team battle ended\nin a draw.";
    default:                    return "The team battle is over.";
    }
}

static const char *Coop_BattleResultText(const struct CoopBattleEnv *env)
{
    if (env->team)
        return Coop_TeamBattleResultText(env);
    if (env->failed)
        return "Couldn't link up with @.\nThe battle was cancelled.";
    if (env->lost)
        return "The link with @ was lost.\nThe battle has ended.";
    switch (env->winFlag & 0xF)
    {
    case BATTLE_OUTCOME_WIN:    return "You won the battle\nagainst @!";
    case BATTLE_OUTCOME_LOSE:   return "You lost the battle\nagainst @.";
    case BATTLE_OUTCOME_DRAW:   return "The battle against @\nended in a draw.";
    default:                    return "The battle against @\nis over.";
    }
}

// A 2v2 (team battle, team boss): this player's three. With more than three
// that can battle (eggs cannot), the party screen chooses them (vanilla's
// Union Room selection); otherwise they all come. TRUE: the party screen was
// called (the task continues here once it is closed).
static BOOL Coop_BattleChooseEntries(TaskManager *taskman, struct CoopBattleEnv *env)
{
    struct Party *party = SaveData_GetPlayerPartyPtr(taskman->fieldSystem->savedata);
    s32 i, n = 0;

    __builtin_memset(env->order, 0, sizeof(env->order));
#if COOP_DEBUG_TOOLS
    if (gCoopDebugMonBuffer.auto_pick[0] != 0) // probes: no party screen to drive
    {
        for (i = 0; i < 3; i++)
            env->order[i] = gCoopDebugMonBuffer.auto_pick[i];
        return FALSE;
    }
#endif
    for (i = 0; i < party->count; i++)
        if (!GetMonData(Party_GetMonByIndex(party, i), MON_DATA_IS_EGG, NULL))
            n++;
    if (n > 3)
        return Coop_CallPartySelectEntries(taskman, 3, env->order);
    for (i = 0, n = 0; i < party->count && n < 3; i++)
        if (!GetMonData(Party_GetMonByIndex(party, i), MON_DATA_IS_EGG, NULL))
            env->order[n++] = (u8)(i + 1);
    return FALSE;
}

// The teammate waits at the same boss, in this map (their published
// boss_wait). Decides who hosts: the lower player id.
static BOOL Coop_BossTeammateWaits(FieldSystem *fsys, struct CoopBattleEnv *env)
{
    struct CoopPlayerState mate;

    if (gCoopCommBlock.gates.team_partner_slot != env->peerSlot || !Coop_ReadRemotePlayer(env->peerSlot, &mate))
        return FALSE;
    return mate.boss_wait == env->bossIndex + 1 && mate.map_id == (u16)fsys->location->mapId;
}

// ---------------------------------------------------------------------------
// The forget-a-move prompt after a team boss win (2026-11-02): vanilla's
// level-up prompt, with the move tutor's summary screen (Coop_CallForgetMove)
// to choose the move to forget.
// ---------------------------------------------------------------------------

enum CoopMoveAfterPage
{
    COOP_MOVE_AFTER_CANT,    // -> "But it can't learn more than four moves."
    COOP_MOVE_AFTER_ASK,     // -> "Delete a move to make room?"
    COOP_MOVE_AFTER_FORGOT,  // -> "It forgot how to use X."
    COOP_MOVE_AFTER_AND,     // -> "And..."
    COOP_MOVE_AFTER_LEARNED, // -> "It learned Y!"
    COOP_MOVE_AFTER_NEXT,    // -> the next move
};

// A move's name into a COOP_MOVE_NAME_BUF buffer, 0xFFFF-padded so that
// buf + COOP_ARG_SPLIT is always a terminated rest.
static void Coop_LoadMoveName(u32 move, u16 *buf)
{
    MsgData *msgData = NewMsgDataFromNarc(MSGDATA_LOAD_LAZY, ARC_MSG_DATA, COOP_MOVE_NAME_FILE, COOP_MSG_HEAP_ID);
    String *name;
    u32 i;

    for (i = 0; i < COOP_MOVE_NAME_BUF; i++)
        buf[i] = 0xFFFF;
    if (msgData == NULL)
        return;
    name = NewString_ReadMsgData(msgData, (s32)move);
    if (name != NULL)
    {
        CopyStringToU16Array(name, buf, COOP_MOVE_NAME_BUF - 1);
        String_Delete(name);
    }
    DestroyMsgData(msgData);
    buf[COOP_MOVE_NAME_BUF - 1] = 0xFFFF;
}

static void Coop_MoveSetArgs(struct CoopBattleEnv *env, u16 *name)
{
    env->dialog.args[2] = name;
    env->dialog.args[3] = name + COOP_ARG_SPLIT;
}

// Starts the next pending move: its Pokemon's nickname and the move's name.
// FALSE when none is left (or the Pokemon left the slot it learned it in).
static BOOL Coop_MoveNext(FieldSystem *fsys, struct CoopBattleEnv *env)
{
    struct CoopRewards *r = &env->rewards;
    struct Party *party = SaveData_GetPlayerPartyPtr(fsys->savedata);
    struct PartyPokemon *mon;
    u32 i, move, slot;

    while (r->pendingNext < r->pendingCount)
    {
        move = r->pendingMove[r->pendingNext];
        slot = r->pendingSlot[r->pendingNext];
        r->pendingNext++;
        if (slot >= (u32)party->count)
            continue;
        mon = Party_GetMonByIndex(party, slot);
        for (i = 0; i < 4; i++)
            if (GetMonData(mon, MON_DATA_MOVE1 + i, NULL) == move || GetMonData(mon, MON_DATA_MOVE1 + i, NULL) == 0)
                break;
        if (i < 4)
            continue; // already knows it, or has room after all
        GetMonData(mon, MON_DATA_NICKNAME, env->nick);
        env->nick[11] = 0xFFFF;
        Coop_LoadMoveName(move, env->moveName);
        env->dialog.args[0] = env->nick;
        Coop_MoveSetArgs(env, env->moveName);
        return TRUE;
    }
    return FALSE;
}

static void Coop_MovePage(TaskManager *taskman, struct CoopBattleEnv *env, const char *text, u32 after)
{
    Coop_DialogPrint(&env->dialog, text);
    env->afterPage = (u8)after;
    taskman->state = COOP_BATTLE_MOVE_PAGE;
}

static void Coop_MoveAskDelete(TaskManager *taskman, struct CoopBattleEnv *env)
{
    Coop_MoveSetArgs(env, env->moveName);
    Coop_DialogPrint(&env->dialog, "Delete a move to make\nroom for &^?");
    taskman->state = COOP_BATTLE_MOVE_ASK;
}

static void Coop_MoveAskStop(TaskManager *taskman, struct CoopBattleEnv *env)
{
    Coop_MoveSetArgs(env, env->moveName);
    Coop_DialogPrint(&env->dialog, "Stop trying to teach\n&^?");
    taskman->state = COOP_BATTLE_MOVE_STOP;
}

// pret MonSetMoveInSlot_ResetPpUp: the new move in the chosen slot, its PP
// Ups cleared and its PP full. Keeps the forgotten move's name for the text.
static void Coop_MoveReplace(FieldSystem *fsys, struct CoopBattleEnv *env)
{
    struct CoopRewards *r = &env->rewards;
    struct PartyPokemon *mon = Party_GetMonByIndex(SaveData_GetPlayerPartyPtr(fsys->savedata), r->pendingSlot[r->pendingNext - 1]);
    u32 slot = env->forgetSlot;
    u32 move = r->pendingMove[r->pendingNext - 1];
    u32 value;

    Coop_LoadMoveName(GetMonData(mon, MON_DATA_MOVE1 + slot, NULL), env->forgotName);
    SetMonData(mon, MON_DATA_MOVE1 + slot, &move);
    value = 0;
    SetMonData(mon, MON_DATA_MOVE1PPUP + slot, &value);
    value = GetMonData(mon, MON_DATA_MOVE1MAXPP + slot, NULL);
    SetMonData(mon, MON_DATA_MOVE1PP + slot, &value);
}

static void Coop_MoveAfterPage(TaskManager *taskman, struct CoopBattleEnv *env)
{
    switch (env->afterPage)
    {
    case COOP_MOVE_AFTER_CANT:
        Coop_MovePage(taskman, env, "But @ can't learn\nmore than four moves.", COOP_MOVE_AFTER_ASK);
        break;
    case COOP_MOVE_AFTER_ASK:
        Coop_MoveAskDelete(taskman, env);
        break;
    case COOP_MOVE_AFTER_FORGOT:
        Coop_MoveSetArgs(env, env->forgotName);
        Coop_MovePage(taskman, env, "@ forgot how to\nuse &^.", COOP_MOVE_AFTER_AND);
        break;
    case COOP_MOVE_AFTER_AND:
        Coop_MovePage(taskman, env, "And...", COOP_MOVE_AFTER_LEARNED);
        break;
    case COOP_MOVE_AFTER_LEARNED:
        Coop_MoveSetArgs(env, env->moveName);
        Coop_MovePage(taskman, env, "@ learned &^!", COOP_MOVE_AFTER_NEXT);
        break;
    default:
        taskman->state = COOP_BATTLE_MOVE_NEXT;
        break;
    }
}

// After a battle that was fought: "Saving..." with the spinner, then the
// save (COOP_BATTLE_POSTSAVE). The dialog must be open.
static void Coop_BattleStartPostSave(TaskManager *taskman, struct CoopBattleEnv *env)
{
    env->dialog.args[0] = env->name;
    Coop_DialogPrint(&env->dialog, "Saving. Don't turn off\nthe power.");
    env->frames = 0;
    taskman->state = COOP_BATTLE_POSTSAVE;
}

static BOOL Coop_Task_Battle(TaskManager *taskman)
{
    FieldSystem *fsys = taskman->fieldSystem;
    struct CoopBattleEnv *env = taskman->env;
    struct BattleSetup *setup;
    void *encounter;
    s32 effect;

    switch (taskman->state)
    {
    case COOP_BATTLE_OPEN:
        if ((env->team || env->boss) && Coop_BattleChooseEntries(taskman, env))
        {
            taskman->state = COOP_BATTLE_CHOSEN;
            break;
        }
        taskman->state = COOP_BATTLE_START;
        break;

    case COOP_BATTLE_CHOSEN:
        // The party screen rebuilt the overworld and its objects.
        if (env->boss)
            env->bossObj = GetMapObjectByID(fsys->mapObjectMan, env->bossObjId);
        if (env->order[0] == 0 || (env->boss && env->bossObj == NULL))
        {
            // B: no three, no battle.
            Coop_BattleOpenDialog(fsys, env);
            Coop_DialogPrint(&env->dialog, env->team ? "You called off the battle\nagainst @'s team." : "You called off the battle.");
            taskman->state = COOP_BATTLE_FINAL_TEXT;
            break;
        }
        taskman->state = COOP_BATTLE_START;
        break;

    case COOP_BATTLE_START:
        Coop_BattleOpenDialog(fsys, env);
        Coop_DialogPrint(&env->dialog, "Saving before the battle.\nDon't turn off the power.");
        env->frames = 0;
        taskman->state = COOP_BATTLE_PRESAVE;
        break;

    case COOP_BATTLE_PRESAVE:
        // A failed save only means the last save stays: battle anyway.
        if (Coop_DialogIsPrinting(&env->dialog))
            break;
        Coop_DialogShowSpinner(&env->dialog);
        if (++env->frames < 2)
            break; // let the message reach the screen first
        (void)Field_SaveGameNormal(fsys);
        Coop_DialogHideSpinner(&env->dialog);
        env->frames = 0;
        if (env->boss)
        {
            gCoopBoss.order[0] = env->order[0];
            gCoopBoss.order[1] = env->order[1];
            gCoopBoss.order[2] = env->order[2];
            Coop_DialogPrint(&env->dialog, "Waiting for @...\nPress B to cancel.");
            gCoopCommBlock.track_boss_wait = (u8)(env->bossIndex + 1);
            taskman->state = COOP_BATTLE_BOSS_WAIT;
            break;
        }
        if (env->team)
            Coop_DialogPrint(&env->dialog, env->parent ? "Waiting for everyone...\nPress B to cancel." : "Linking up with @...\nPress B to cancel.");
        else
            Coop_DialogPrint(&env->dialog, env->parent ? "Waiting for @...\nPress B to cancel." : "Linking up with @...\nPress B to cancel.");
        taskman->state = COOP_BATTLE_START_LINK;
        break;

    case COOP_BATTLE_BOSS_WAIT:
        if (gSystem.newKeys & PAD_BUTTON_B)
        {
            // Stop waiting (the teammate can no longer link with us), and
            // offer the battle alone: a teammate who is away, or whose save
            // already beat this boss, can never soft-lock the team.
            gCoopCommBlock.track_boss_wait = 0;
            Coop_DialogPrint(&env->dialog, "Battle alone instead?");
            taskman->state = COOP_BATTLE_BOSS_ALONE;
            break;
        }
        if (Coop_BossTeammateWaits(fsys, env))
            taskman->state = COOP_BATTLE_START_LINK; // boss_wait stays up until the script starts
        break;

    case COOP_BATTLE_BOSS_ALONE:
    {
        u32 value;

        if (Coop_DialogIsPrinting(&env->dialog))
            break;
        if (!env->choice.open)
            Coop_ChoiceOpenYesNo(fsys, &env->choice);
        value = Coop_ChoiceInput(&env->choice);
        if (value == COOP_CHOICE_NONE)
            break;
        Coop_ChoiceClose(&env->choice);
        if (value == COOP_CHOICE_NO)
            return Coop_BattleEnd(fsys, env);
        // The boss's own script, as the A press would have run it: not
        // armed, so its battle is vanilla's solo battle with the solo roster.
        Coop_DialogClose(fsys, &env->dialog);
        MapObjectMan_UnpauseAllMovement(fsys->mapObjectMan);
        QueueScript(taskman, env->bossScript, env->bossObj, NULL);
        taskman->state = COOP_BATTLE_BOSS_ALONE_DONE;
        break;
    }

    case COOP_BATTLE_BOSS_ALONE_DONE:
        sys_FreeMemoryEz(env);
        return TRUE;

    case COOP_BATTLE_START_LINK:
    {
        s8 linkMode = env->boss ? COOP_COMM_TYPE_BATTLE_TOWER
                    : env->team ? COOP_COMM_TYPE_MULTI_BATTLE : COOP_COMM_TYPE_SINGLE_BATTLE;
        if (env->parent)
            CommState_StartParentSession(fsys->savedata, linkMode, (s8)env->beaconId, NULL, 0);
        else
            CommState_StartChildSession(fsys->savedata, linkMode, (s8)env->beaconId, NULL, 0);
        env->frames = 0;
        taskman->state = env->parent ? COOP_BATTLE_WAIT_LINKED : COOP_BATTLE_FIND_PARENT;
        break;
    }

    case COOP_BATTLE_FIND_PARENT:
        if ((gSystem.newKeys & PAD_BUTTON_B) || ++env->frames >= (env->team ? COOP_TEAM_LINK_FAILSAFE : COOP_FIND_FAILSAFE))
        {
            Coop_BattleFail(taskman, env);
            break;
        }
        if (CommWireless_CountParents() > 0)
        {
            CommState_ConnectToSlot(0); // only the partner's parent has this beacon id
            env->frames = 0;
            taskman->state = COOP_BATTLE_WAIT_LINKED;
        }
        break;

    case COOP_BATTLE_WAIT_LINKED:
        if ((gSystem.newKeys & PAD_BUTTON_B) || ++env->frames >= (env->team ? COOP_TEAM_LINK_FAILSAFE : COOP_LINK_FAILSAFE))
        {
            Coop_BattleFail(taskman, env);
            break;
        }
        if (CommState_IsLinked() && Comm_CountConnected() >= (env->team ? 4 : 2))
        {
            LinkSync_StartSync(COOP_BATTLE_SYNC);
            taskman->state = COOP_BATTLE_WAIT_SYNC;
        }
        break;

    case COOP_BATTLE_WAIT_SYNC:
        if (++env->frames >= COOP_LINK_FAILSAFE)
        {
            Coop_BattleFail(taskman, env);
            break;
        }
        if (!LinkSync_IsSynced(COOP_BATTLE_SYNC) || Coop_DialogIsPrinting(&env->dialog))
            break;
        if (env->boss)
        {
            // Both linked: each game runs the boss's own script, as the A
            // press would have; its TrainerBattle becomes the link multi
            // (src/coop_boss.c), its rewards land on each save.
            Coop_DialogClose(fsys, &env->dialog);
            MapObjectMan_UnpauseAllMovement(fsys->mapObjectMan);
            gCoopCommBlock.track_boss_wait = 0;
            gCoopBoss.bossIndex = env->bossIndex;
            gCoopBoss.fought = 0;
            gCoopBoss.won = 0;
            gCoopBoss.foeCount = 0;
            gCoopBoss.prizeLevel = 0;
            Coop_SnapshotPartyForRewards(fsys);
            gCoopBoss.armed = 1;
            QueueScript(taskman, env->bossScript, env->bossObj, NULL);
            taskman->state = COOP_BATTLE_BOSS_SCRIPT;
            break;
        }
        // Both games are linked and ready: vanilla's link battle. The battle
        // leaves the overworld (and this overlay); the task continues here.
        Coop_DialogClose(fsys, &env->dialog);
        if (env->team)
        {
            // The Colosseum's multi (ccd CommClubLink_StartBattle) with this
            // player's three (1-based party slots, Coop_BattleChooseEntries).
            setup = BattleSetup_New(COOP_HEAP_ID_FIELD2, BATTLE_TYPE_LINK | BATTLE_TYPE_MULTI | BATTLE_TYPE_DOUBLES | BATTLE_TYPE_TRAINER);
            BattleSetup_InitForLinkBattle(setup, fsys, env->order);
            Coop_StampBattleAvatar(setup); // custom avatars: the pictures on every game
#if COOP_DEBUG_TOOLS
            gCoopDebugMonBuffer.pad[1] = (u8)setup->party[0]->count; // probes: the Pokemon this player brings
#endif
        }
        else
        {
            setup = BattleSetup_New(COOP_HEAP_ID_FIELD2, BATTLE_TYPE_LINK | BATTLE_TYPE_TRAINER);
            BattleSetup_InitForLinkBattle(setup, fsys, NULL);
            Coop_StampBattleAvatar(setup);
        }
        // A 1v1 opens with both players' VS faces (custom avatars Phase 6); 2v2s keep vanilla's.
        effect = !env->team && Coop_VsIntroArm(env->peerSlot, env->name) ? COOP_VS_INTRO_EFFECT
                                                                        : BattleSetup_GetWildTransitionEffect(setup);
        encounter = Encounter_New(setup, effect, BattleSetup_GetWildBattleMusic(setup), &env->winFlag);
        env->fought = TRUE;
        gCoopLinkBattleChildren = env->team ? 3 : 1;
        gCoopCommBlock.track_link_lost_frames = 0;
        gCoopCommBlock.track_link_battle = COOP_LINK_BATTLE_RUNNING; // arms the watchdog (src/coop_link.c)
        TaskManager_Call(taskman, Task_LinkBattle, encounter);
        taskman->state = COOP_BATTLE_FIGHT;
        break;

    case COOP_BATTLE_FIGHT:
        env->lost = gCoopCommBlock.track_link_battle == COOP_LINK_BATTLE_LOST;
        gCoopCommBlock.track_link_battle = COOP_LINK_BATTLE_NONE;
        gCoopLinkBattleChildren = 1;
#if COOP_DEBUG_TOOLS
        gCoopDebugMonBuffer.pad[0] = (u8)env->winFlag; // link probe
#endif
        CommState_RequestShutdown();
        env->frames = 0;
        // Task_LinkBattle restores the overworld but leaves it black (vanilla's
        // Colosseum script fades in itself).
        CallTask_FadeFromBlack(taskman);
        taskman->state = COOP_BATTLE_SHUTDOWN;
        break;

    case COOP_BATTLE_SHUTDOWN:
        if (CommState_Exists() && ++env->frames < COOP_SHUTDOWN_FAILSAFE)
            break;
        taskman->state = COOP_BATTLE_RESULT;
        // fallthrough
    case COOP_BATTLE_RESULT:
        if (env->fought)
            Coop_BattleOpenDialog(fsys, env);
        else if (Coop_DialogIsPrinting(&env->dialog))
            break;
        Coop_DialogPrint(&env->dialog, Coop_BattleResultText(env));
        taskman->state = COOP_BATTLE_FINAL_TEXT;
        break;

    case COOP_BATTLE_FINAL_TEXT:
        if (!Coop_DialogIsPrinting(&env->dialog))
            taskman->state = COOP_BATTLE_FINAL_BUTTON;
        break;

    case COOP_BATTLE_FINAL_BUTTON:
        if (!(gSystem.newKeys & (PAD_BUTTON_A | PAD_BUTTON_B)))
            break;
        if (!env->fought || env->saved)
            return Coop_BattleEnd(fsys, env);
        Coop_BattleStartPostSave(taskman, env);
        break;

    case COOP_BATTLE_BOSS_SCRIPT:
        // The script is over (the battle task shut the link down after a
        // battle; a script that ended without one leaves it to us).
        gCoopBoss.armed = 0;
        if (CommState_Exists())
            CommState_RequestShutdown();
        env->frames = 0;
        taskman->state = COOP_BATTLE_BOSS_CLOSE;
        break;

    case COOP_BATTLE_BOSS_CLOSE:
        if (CommState_Exists() && ++env->frames < COOP_SHUTDOWN_FAILSAFE)
            break;
        // A win: the experience and prize money the link battle could not
        // give (src/field/coop_rewards.c), then the pages and evolutions.
        if (!Coop_PayBossRewards(fsys, &env->rewards))
        {
            sys_FreeMemoryEz(env);
            return TRUE;
        }
#if COOP_DEBUG_TOOLS
        __builtin_memcpy(gCoopDebugMonBuffer.mon, &env->rewards, sizeof(env->rewards)); // link_e2e_probe.lua
#endif
        Coop_BattleOpenDialog(fsys, env);
        env->dialog.args[0] = env->nick;
        taskman->state = COOP_BATTLE_REWARD_PAGE;
        // fallthrough
    case COOP_BATTLE_REWARD_PAGE:
        if (Coop_RewardNextPage(fsys, env))
        {
            Coop_DialogPrint(&env->dialog, env->text);
            taskman->state = COOP_BATTLE_REWARD_WAIT;
            break;
        }
        taskman->state = COOP_BATTLE_MOVE_NEXT;
        // fallthrough
    case COOP_BATTLE_MOVE_NEXT:
        if (!Coop_MoveNext(fsys, env))
        {
            Coop_DialogClose(fsys, &env->dialog); // the scenes need the windows gone
            taskman->state = COOP_BATTLE_REWARD_EVOLVE;
            break;
        }
        Coop_MovePage(taskman, env, "@ wants to learn\nthe move &^.", COOP_MOVE_AFTER_CANT);
        break;

    case COOP_BATTLE_MOVE_PAGE:
        if (Coop_DialogIsPrinting(&env->dialog) || !(gSystem.newKeys & (PAD_BUTTON_A | PAD_BUTTON_B)))
            break;
        Coop_MoveAfterPage(taskman, env);
        break;

    case COOP_BATTLE_MOVE_ASK:
    case COOP_BATTLE_MOVE_STOP:
    {
        u32 value;

        if (Coop_DialogIsPrinting(&env->dialog))
            break;
        if (!env->choice.open)
            Coop_ChoiceOpenYesNo(fsys, &env->choice);
        value = Coop_ChoiceInput(&env->choice);
        if (value == COOP_CHOICE_NONE)
            break;
        Coop_ChoiceClose(&env->choice);
        if (taskman->state == COOP_BATTLE_MOVE_STOP)
        {
            if (value == COOP_CHOICE_YES)
                Coop_MovePage(taskman, env, "@ did not learn\nthe move &^.", COOP_MOVE_AFTER_NEXT);
            else
                Coop_MoveAskDelete(taskman, env);
            break;
        }
        if (value == COOP_CHOICE_NO)
        {
            Coop_MoveAskStop(taskman, env);
            break;
        }
        // The move tutor's screen: the overworld is left meanwhile.
        Coop_DialogClose(fsys, &env->dialog);
        if (!Coop_CallForgetMove(taskman, env->rewards.pendingSlot[env->rewards.pendingNext - 1],
                                 env->rewards.pendingMove[env->rewards.pendingNext - 1], &env->forgetSlot))
            env->forgetSlot = COOP_FORGET_MOVE_GIVE_UP; // out of memory
        taskman->state = COOP_BATTLE_MOVE_FORGOT;
        break;
    }

    case COOP_BATTLE_MOVE_FORGOT:
        Coop_BattleOpenDialog(fsys, env);
        env->dialog.args[0] = env->nick;
        if (env->forgetSlot >= COOP_FORGET_MOVE_GIVE_UP)
        {
            Coop_MoveAskStop(taskman, env);
            break;
        }
        Coop_MoveReplace(fsys, env);
#if COOP_DEBUG_TOOLS
        {
            // link_e2e_probe.lua (E2E_BOSS_LEAD=10): the Pokemon's moves now,
            // after the rewards summary (80 bytes) at the buffer's start.
            struct PartyPokemon *mon = Party_GetMonByIndex(SaveData_GetPlayerPartyPtr(fsys->savedata),
                                                           env->rewards.pendingSlot[env->rewards.pendingNext - 1]);
            u32 i;
            for (i = 0; i < 4; i++)
            {
                u16 m = (u16)GetMonData(mon, MON_DATA_MOVE1 + i, NULL);
                __builtin_memcpy((u8 *)gCoopDebugMonBuffer.mon + COOP_DEBUG_MOVES_OFFSET + i * 2, &m, 2);
            }
        }
#endif
        Coop_MovePage(taskman, env, "1, 2, and... ... Poof!", COOP_MOVE_AFTER_FORGOT);
        break;

    case COOP_BATTLE_REWARD_WAIT:
        if (!Coop_DialogIsPrinting(&env->dialog) && (gSystem.newKeys & (PAD_BUTTON_A | PAD_BUTTON_B)))
            taskman->state = COOP_BATTLE_REWARD_PAGE;
        break;

    case COOP_BATTLE_REWARD_EVOLVE:
    {
        u16 species;
        int method, slot = Coop_NextRewardEvolution(fsys, &env->rewards, &species, &method);

        if (slot >= 0
            && Coop_CallEvolutionScene(taskman, Party_GetMonByIndex(SaveData_GetPlayerPartyPtr(fsys->savedata), slot), species, method))
            break; // back here once the overworld is back
        if (slot >= 0)
            break; // out of memory: try the next one
        // The badge, the rewards and the evolutions go on the save now, so
        // both games of the team keep the win (one that never saves after a
        // win would have to fight the boss again, alone).
        Coop_BattleOpenDialog(fsys, env);
        Coop_BattleStartPostSave(taskman, env);
        break;
    }

    case COOP_BATTLE_POSTSAVE:
        if (Coop_DialogIsPrinting(&env->dialog))
            break;
        Coop_DialogShowSpinner(&env->dialog);
        if (++env->frames < 2)
            break; // let the message reach the screen first
        env->saved = TRUE;
        if (Field_SaveGameNormal(fsys))
        {
            Coop_DialogHideSpinner(&env->dialog);
            taskman->state = COOP_BATTLE_SAVED;
            break;
        }
        Coop_DialogHideSpinner(&env->dialog);
        Coop_DialogPrint(&env->dialog, "The save failed.\nSave from the menu soon.");
        taskman->state = COOP_BATTLE_FINAL_TEXT;
        break;

    case COOP_BATTLE_SAVED:
        return Coop_BattleEnd(fsys, env);
    }
    return FALSE;
}

static struct CoopBattleEnv *Coop_BattleNewEnv(const u16 *name, BOOL parent, u32 beaconId)
{
    struct CoopBattleEnv *env = sys_AllocMemory(32 /* HEAP_ID_FIELD3 */, sizeof(struct CoopBattleEnv));
    u32 i;

    if (env == NULL)
        return NULL;
    __builtin_memset(env, 0, sizeof(*env));
    for (i = 0; i < 8; i++)
        env->name[i] = name != NULL ? name[i] : 0xFFFF;
    env->name[8] = 0xFFFF;
    env->parent = parent ? TRUE : FALSE;
    env->beaconId = (u8)beaconId;
    return env;
}

BOOL Coop_BattleTakeOver(TaskManager *taskman, const u16 *name, u32 peerSlot, BOOL parent, BOOL team)
{
    u32 me = gCoopCommBlock.assigned_player_id;
    u32 requester = parent ? me : (peerSlot < me ? peerSlot : peerSlot + 1);
    struct CoopBattleEnv *env = Coop_BattleNewEnv(name, parent, requester + 1);

    if (env == NULL)
        return FALSE;
    env->team = team ? TRUE : FALSE;
    env->peerSlot = (u8)peerSlot; // the other player's remote slot (the PvP intro shows them)
    taskman->func = Coop_Task_Battle;
    taskman->state = COOP_BATTLE_OPEN;
    taskman->env = env;
    return TRUE;
}

#if COOP_DEBUG_TOOLS
BOOL Coop_DebugStartBattle(FieldSystem *fsys, BOOL parent)
{
    static const u16 sProbe[] = { 0x013A, 0x0149, 0x0149, 0x0156, 0xFFFF }; // "Peer"
    struct CoopBattleEnv *env = Coop_BattleNewEnv(sProbe, parent, 1);

    if (env == NULL)
        return FALSE;
    if (Coop_FieldSystem_CreateTask(fsys, Coop_Task_Battle, env) == NULL)
    {
        sys_FreeMemoryEz(env);
        return FALSE;
    }
    return TRUE;
}
#endif

// ---------------------------------------------------------------------------
// Team bosses (docs/MILESTONE3_PLAN.md §18)
// ---------------------------------------------------------------------------

// The teammate already has this leader's badge (their save kept a win this
// one lost, e.g. after a crash before saving): their leader would not battle,
// so this player fights alone, as vanilla. sGatedBosses (coop_field_tasks.c)
// lists the sixteen leaders first, in badge order, so a leader's index is
// its badge; the Elite Four, Red and Lance follow and have none.
#define COOP_BOSS_BADGE_COUNT 16

static BOOL Coop_MateHasBossBadge(const struct CoopPlayerState *mate, u32 index)
{
    u32 badges = (u32)mate->badge_mask[0] | ((u32)mate->badge_mask[1] << 8);

    return index < COOP_BOSS_BADGE_COUNT && (badges & (1u << index)) != 0;
}

// The resident talk hook (src/coop_boss.c): the player talked to a team boss
// while teamed. Wait for the teammate instead of the boss's script; with no
// teammate to wait for, talk as vanilla does.
void LONG_CALL Coop_BossTalkStart(FieldSystem *fsys, LocalMapObject *boss, u16 script)
{
    int index = Coop_FindGatedBoss(fsys, boss);
    u32 slot = gCoopCommBlock.gates.team_partner_slot;
    u32 me = gCoopCommBlock.assigned_player_id;
    u32 mateId, low, high;
    struct CoopPlayerState mate;
    struct CoopBattleEnv *env;

    if (index < 0 || slot == COOP_SLOT_NONE || !Coop_ReadRemotePlayer(slot, &mate)
        || Coop_MateHasBossBadge(&mate, (u32)index))
    {
        Coop_StartVanillaTalk(fsys, script, boss);
        return;
    }
    mateId = slot < me ? slot : slot + 1;
    low = me < mateId ? me : mateId;
    high = me < mateId ? mateId : me;
    env = Coop_BattleNewEnv(mate.name, me < mateId, COOP_BOSS_BEACON_BASE + low * 6 + high);
    if (env == NULL)
    {
        Coop_StartVanillaTalk(fsys, script, boss);
        return;
    }
    env->boss = TRUE;
    env->bossIndex = (u8)index;
    env->peerSlot = (u8)slot;
    env->bossScript = script;
    env->bossObj = boss;
    env->bossObjId = boss->id;
    if (Coop_FieldSystem_CreateTask(fsys, Coop_Task_Battle, env) == NULL)
    {
        sys_FreeMemoryEz(env);
        Coop_StartVanillaTalk(fsys, script, boss);
    }
}

// ---------------------------------------------------------------------------
// Team rosters and badge-scaled bosses (docs/GAME_REBALANCE_UPDATE.md §3.3,
// §4.2). The tables are generated (src/field/coop_team_rosters.c).
// ---------------------------------------------------------------------------

u32 LONG_CALL Coop_TeamRosterFor(u32 trainer)
{
    u32 i;

    for (i = 0; i < gCoopTeamRosterCount; i++)
        if (gCoopTeamRosters[i].solo == trainer)
            return gCoopTeamRosters[i].team;
    return trainer;
}

static u32 Coop_CountBadges(u32 mask)
{
    u32 n = 0;

    for (; mask != 0; mask &= mask - 1)
        n++;
    return n;
}

// The badges that scale a boss: this player's, or in a team boss battle the
// lower count of the two (both games must build the same party).
static u32 Coop_ScalingBadges(BOOL kanto)
{
    struct CoopPlayerState mate;
    u32 mine = gCoopCommBlock.badge_mask_local, theirs;

    if (kanto)
        mine >>= 8;
    mine = Coop_CountBadges(mine & 0xFF) + (kanto ? 0 : Coop_CountBadges(gCoopCommBlock.badge_mask_local >> 8));
    if (!gCoopBoss.armed || gCoopCommBlock.gates.team_partner_slot == COOP_SLOT_NONE
        || !Coop_ReadRemotePlayer(gCoopCommBlock.gates.team_partner_slot, &mate))
        return mine;
    theirs = (u32)mate.badge_mask[0] | ((u32)mate.badge_mask[1] << 8);
    theirs = kanto ? Coop_CountBadges(theirs >> 8) : Coop_CountBadges(theirs);
    return theirs < mine ? theirs : mine;
}

void LONG_CALL Coop_ScaleBossParty(struct BATTLE_PARAM *bp, int num)
{
    const struct CoopScaledBoss *boss = NULL;
    struct Party *party;
    u32 trainer, i;
    int delta, badges;

    if (num != BATTLER_ENEMY && num != BATTLER_ENEMY2)
        return;
    trainer = (u32)bp->trainer_id[num];
    for (i = 0; i < gCoopTeamRosterCount; i++)
        if (gCoopTeamRosters[i].team == trainer)
            trainer = gCoopTeamRosters[i].solo;
    for (i = 0; i < gCoopScaledBossCount; i++)
        if (gCoopScaledBosses[i].trainer == trainer)
            boss = &gCoopScaledBosses[i];
#if COOP_DEBUG_TOOLS
    // Milestone 4 Phase 3 probe: the next trainer scales like a Kanto leader
    // (command 11, debug_arg 3), so Falkner can show it.
    static const struct CoopScaledBoss sDebugKanto = { 0, 0, 2, 1 };
    if (boss == NULL && gCoopDebugMonBuffer.team_battle_armed == 3 && num == BATTLER_ENEMY)
    {
        gCoopDebugMonBuffer.team_battle_armed = 0;
        gCoopDebugMonBuffer.team_battle_used = 3;
        boss = &sDebugKanto;
    }
#endif
    if (boss == NULL)
    {
        // An ordinary Kanto trainer (rebalance plan §3.3): +2 per Kanto badge,
        // as the wild Pokemon there. Bosses keep their own rules.
        static const struct CoopScaledBoss sKantoTrainer = { 0, 0, 2, 1 };
        for (i = 0; i < gCoopTeamRosterCount; i++)
            if (gCoopTeamRosters[i].solo == trainer)
                return;
        if (gFieldSysPtr == NULL || gFieldSysPtr->location == NULL || !Coop_IsKantoTrainerMap(gFieldSysPtr->location->mapId))
            return;
        boss = &sKantoTrainer;
    }
    badges = (int)Coop_ScalingBadges(boss->kanto);
    if (boss->kanto)
        delta = boss->step * (badges > 7 ? 7 : badges);
    else
    {
        delta = (badges - (int)boss->expected) * boss->step;
        if (delta > 2 * boss->step)
            delta = 2 * boss->step;
        if (delta < -2 * boss->step)
            delta = -2 * boss->step;
    }
    if (delta == 0)
        return;
    party = bp->poke_party[num];
    for (i = 0; i < (u32)party->count; i++)
    {
        struct PartyPokemon *mon = Party_GetMonByIndex(party, (int)i);
        int level = (int)GetMonData(mon, MON_DATA_LEVEL, NULL) + delta;
        u32 exp;

        if (level < 2)
            level = 2;
        if (level > 100)
            level = 100;
        exp = PokeLevelExpGet((int)GetMonData(mon, MON_DATA_SPECIES, NULL), level);
        SetMonData(mon, MON_DATA_EXPERIENCE, &exp);
        RecalcPartyPokemonStats(mon);
    }
}

// The boss in both enemy seats, its team split between them: seat 1 the
// first half (rounded up), seat 3 the rest (§0: first 3 / last 3). Both
// games build the same parties from the same trainer data.
void LONG_CALL Coop_BossSplitParty(struct BATTLE_PARAM *bp, int num)
{
    struct Party *party;
    int count, half, i;

    if (!gCoopBoss.armed || (num != BATTLER_ENEMY && num != BATTLER_ENEMY2))
        return;
    party = bp->poke_party[num];
    count = party->count;
    half = (count + 1) / 2;
    if (num == BATTLER_ENEMY && count > 0)
    {
        // The prize, as vanilla's CalcPrizeMoney reads it: the whole team's
        // last Pokemon (src/field/coop_rewards.c pays it after a win).
        gCoopBoss.prizeClass = bp->trainer_data[num].tr_type;
        gCoopBoss.prizeLevel = (u8)GetMonData(Party_GetMonByIndex(party, count - 1), MON_DATA_LEVEL, NULL);
        gCoopBoss.prizeDouble = (bp->trainer_data[num].fight_type & BATTLE_TYPE_DOUBLES) != 0;
        // The EXP (src/field/coop_rewards.c): a win beats the whole team.
        for (i = 0; i < count && i < COOP_BOSS_MAX_FOES; i++)
        {
            struct PartyPokemon *foe = Party_GetMonByIndex(party, i);
            gCoopBoss.foeSpecies[i] = (u16)GetMonData(foe, MON_DATA_SPECIES, NULL);
            gCoopBoss.foeForm[i] = (u8)GetMonData(foe, MON_DATA_FORM, NULL);
            gCoopBoss.foeLevel[i] = (u8)GetMonData(foe, MON_DATA_LEVEL, NULL);
        }
        gCoopBoss.foeCount = (u8)i;
    }
    if (count < 2)
        return; // nothing to split: both seats keep it
    if (num == BATTLER_ENEMY)
        party->count = half;
    else
    {
        for (i = 0; i < count - half; i++)
            __builtin_memcpy(Party_GetMonByIndex(party, i), Party_GetMonByIndex(party, i + half), COOP_PARTY_MON_SIZE);
        party->count = count - half;
    }
    // The trainer's own count too (bit 7: random order), read by the battle.
    bp->trainer_data[num].poke_count = (u8)((bp->trainer_data[num].poke_count & 0x80) | party->count);
}

