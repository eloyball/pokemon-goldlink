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
#include "coop_dialog.h"
#include "coop_battle.h"
#include "coop_boss.h"

// ---------------------------------------------------------------------------
// 1v1 link battles (docs/MILESTONE3_PLAN.md §17), Overlay 131.
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
// ---------------------------------------------------------------------------

#define COOP_COMM_TYPE_SINGLE_BATTLE 1 // ccd constants/comm_type.h
#define COOP_COMM_TYPE_BATTLE_TOWER  16 // a team boss: the Battle Tower's link multi
#define COOP_BOSS_BEACON_BASE        16 // team boss beacon ids: 16 + lower id * 6 + higher id (PvP: 1-6)
#define COOP_HEAP_ID_FIELD2          11
#define COOP_BATTLE_SYNC             0x61 // LinkSync value both games reach before the battle
#define COOP_FPS                     30 // game logic (field tasks) runs every other frame (ccd main.c)
#define COOP_FIND_FAILSAFE           (20 * COOP_FPS) // child: the parent's beacon
#define COOP_LINK_FAILSAFE           (30 * COOP_FPS) // both linked and synced
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

enum CoopBattleTaskState
{
    COOP_BATTLE_OPEN,
    COOP_BATTLE_BOSS_WAIT,   // team boss: until the teammate waits at the same boss
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
    // Team boss (docs/MILESTONE3_PLAN.md §18).
    u8 boss;
    u8 bossIndex;  // sGatedBosses index
    u8 peerSlot;   // the teammate's remote slot
    u16 bossScript;
    LocalMapObject *bossObj;
};

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

static const char *Coop_BattleResultText(const struct CoopBattleEnv *env)
{
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

// The teammate waits at the same boss, in this map (their published
// boss_wait). Decides who hosts: the lower player id.
static BOOL Coop_BossTeammateWaits(FieldSystem *fsys, struct CoopBattleEnv *env)
{
    struct CoopPlayerState mate;

    if (gCoopCommBlock.gates.team_partner_slot != env->peerSlot || !Coop_ReadRemotePlayer(env->peerSlot, &mate))
        return FALSE;
    return mate.boss_wait == env->bossIndex + 1 && mate.map_id == (u16)fsys->location->mapId;
}

static BOOL Coop_Task_Battle(TaskManager *taskman)
{
    FieldSystem *fsys = taskman->fieldSystem;
    struct CoopBattleEnv *env = taskman->env;
    struct BattleSetup *setup;
    void *encounter;

    switch (taskman->state)
    {
    case COOP_BATTLE_OPEN:
        Coop_BattleOpenDialog(fsys, env);
        if (env->boss)
        {
            Coop_DialogPrint(&env->dialog, "Waiting for @...\nPress B to cancel.");
            gCoopCommBlock.track_boss_wait = (u8)(env->bossIndex + 1);
            taskman->state = COOP_BATTLE_BOSS_WAIT;
            break;
        }
        Coop_DialogPrint(&env->dialog, env->parent ? "Waiting for @...\nPress B to cancel." : "Linking up with @...\nPress B to cancel.");
        taskman->state = COOP_BATTLE_START_LINK;
        break;

    case COOP_BATTLE_BOSS_WAIT:
        if (gSystem.newKeys & PAD_BUTTON_B)
        {
            gCoopCommBlock.track_boss_wait = 0;
            return Coop_BattleEnd(fsys, env);
        }
        if (Coop_BossTeammateWaits(fsys, env))
            taskman->state = COOP_BATTLE_START_LINK; // boss_wait stays up until the script starts
        break;

    case COOP_BATTLE_START_LINK:
    {
        s8 linkMode = env->boss ? COOP_COMM_TYPE_BATTLE_TOWER : COOP_COMM_TYPE_SINGLE_BATTLE;
        if (env->parent)
            CommState_StartParentSession(fsys->savedata, linkMode, (s8)env->beaconId, NULL, 0);
        else
            CommState_StartChildSession(fsys->savedata, linkMode, (s8)env->beaconId, NULL, 0);
        env->frames = 0;
        taskman->state = env->parent ? COOP_BATTLE_WAIT_LINKED : COOP_BATTLE_FIND_PARENT;
        break;
    }

    case COOP_BATTLE_FIND_PARENT:
        if ((gSystem.newKeys & PAD_BUTTON_B) || ++env->frames >= COOP_FIND_FAILSAFE)
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
        if ((gSystem.newKeys & PAD_BUTTON_B) || ++env->frames >= COOP_LINK_FAILSAFE)
        {
            Coop_BattleFail(taskman, env);
            break;
        }
        if (CommState_IsLinked() && Comm_CountConnected() >= 2)
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
            gCoopBoss.armed = 1;
            QueueScript(taskman, env->bossScript, env->bossObj, NULL);
            taskman->state = COOP_BATTLE_BOSS_SCRIPT;
            break;
        }
        // Both games are linked and ready: vanilla's link battle. The battle
        // leaves the overworld (and this overlay); the task continues here.
        Coop_DialogClose(fsys, &env->dialog);
        setup = BattleSetup_New(COOP_HEAP_ID_FIELD2, BATTLE_TYPE_LINK | BATTLE_TYPE_TRAINER);
        BattleSetup_InitForLinkBattle(setup, fsys, NULL);
        encounter = Encounter_New(setup, BattleSetup_GetWildTransitionEffect(setup), BattleSetup_GetWildBattleMusic(setup), &env->winFlag);
        env->fought = TRUE;
        gCoopCommBlock.track_link_lost_frames = 0;
        gCoopCommBlock.track_link_battle = COOP_LINK_BATTLE_RUNNING; // arms the watchdog (src/coop_link.c)
        TaskManager_Call(taskman, Task_LinkBattle, encounter);
        taskman->state = COOP_BATTLE_FIGHT;
        break;

    case COOP_BATTLE_FIGHT:
        env->lost = gCoopCommBlock.track_link_battle == COOP_LINK_BATTLE_LOST;
        gCoopCommBlock.track_link_battle = COOP_LINK_BATTLE_NONE;
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
        if (gSystem.newKeys & (PAD_BUTTON_A | PAD_BUTTON_B))
            return Coop_BattleEnd(fsys, env);
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
        sys_FreeMemoryEz(env);
        return TRUE;
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

BOOL Coop_BattleTakeOver(TaskManager *taskman, const u16 *name, u32 peerSlot, BOOL parent)
{
    u32 me = gCoopCommBlock.assigned_player_id;
    u32 requester = parent ? me : (peerSlot < me ? peerSlot : peerSlot + 1);
    struct CoopBattleEnv *env = Coop_BattleNewEnv(name, parent, requester + 1);

    if (env == NULL)
        return FALSE;
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

    if (index < 0 || slot == COOP_SLOT_NONE || !Coop_ReadRemotePlayer(slot, &mate))
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
    if (Coop_FieldSystem_CreateTask(fsys, Coop_Task_Battle, env) == NULL)
    {
        sys_FreeMemoryEz(env);
        Coop_StartVanillaTalk(fsys, script, boss);
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

