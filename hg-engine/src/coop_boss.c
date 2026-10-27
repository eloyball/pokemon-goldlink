#include "config.h"
#include "types.h"

#include "constants/battle_constants.h"

#include "battle.h"
#include "battle_controller_player.h"
#include "map_events_internal.h"
#include "pokemon.h"
#include "script.h"
#include "system.h"
#include "task.h"
#include "coop_sync.h"
#include "coop_boss.h"

// ---------------------------------------------------------------------------
// Team bosses (docs/MILESTONE3_PLAN.md §18), Overlay 129.
//
// Two vanilla functions are replaced (`hooks`):
// - StartMapSceneScript: talking to a team boss the player faces starts the
//   waiting task (Overlay 131) instead of the boss's script.
// - SetupAndStartTrainerBattle: once the waiting task has linked both games
//   and started the boss's script, the boss's TrainerBattle runs as a link
//   multi of the two players against the boss in both enemy seats.
// Otherwise both do what vanilla does (pret src/script_manager.c,
// src/encounter.c; the hooks overwrite the originals' first bytes).
//
// The battle task is resident: after a lost battle it hands back to the
// script (white-out) without restoring the overworld, as vanilla does, and
// Overlay 131 is not loaded then.
// ---------------------------------------------------------------------------

#define COOP_HEAP_ID_FIELD2      11
#define COOP_HEAP_ID_FIELD3      32 // field task heap
#define COOP_GAME_STAT_TRAINER_BATTLES 9  // ccd constants/game_stats.h
#define COOP_VAR_BATTLE_RESULT   0x4013  // pret constants/vars.h
#define COOP_BATTLE_TYPE_11      (1 << 11) // ccd constants/battle.h
#define COOP_BOSS_BATTLE_TYPE    (BATTLE_TYPE_FRONTIER | BATTLE_TYPE_MULTI | BATTLE_TYPE_LINK | BATTLE_TYPE_DOUBLES | BATTLE_TYPE_TRAINER)
#define COOP_SHUTDOWN_FAILSAFE   (10 * 30) // field tasks run 30 times a second
#define COOP_BOSS_SYNC_FAILSAFE  (60 * 30) // the teammate still reading the boss's lines
#define COOP_BOSS_BATTLE_SYNC    0x62      // LinkSync value both games reach before the battle
// A Party as the game stores it (ccd PartyCore_sizeof): max and count, then
// six Pokemon of 0xEC bytes. hg-engine's struct Party pads them to 0xF0.
#define COOP_PARTY_CORE_SIZE     (8 + 6 * 0xEC)

// ccd BattleSetup.followMonSlot[4] (after saveData 0x1C0, isRecordingPaused,
// bugContestMon 0x1C8). hg-engine's struct does not line up there (its
// unk1CC is elsewhere), so it is written by offset.
#define COOP_SETUP_FOLLOW_MON_SLOT 0x1CC

_Static_assert(offsetof(struct BattleSetup, winFlag) == 0x14, "BattleSetup.winFlag (ccd)");
_Static_assert(offsetof(struct BattleSetup, trainerId) == 0x18, "BattleSetup.trainerId (ccd)");
_Static_assert(offsetof(LocalMapObject, scriptId) == 0x20, "LocalMapObject.scriptId (pret)");

// Script manager (pret asm/fieldmap.s labels before the C conversion).
void *LONG_CALL ScriptEnvironment_New(void);
void LONG_CALL SetupScriptEngine(FieldSystem *fsys, void *env, u16 script, LocalMapObject *obj, void *result);
BOOL LONG_CALL Task_RunScripts(TaskManager *taskman);
void LONG_CALL FieldSystem_CreateTask(FieldSystem *fsys, TaskFunc func, void *env);
// Encounters and battle setup (pret src/encounter.c, src/battle/battle_setup.c,
// src/trainer_data.c; addresses from their labels before the renames).
void LONG_CALL EnemyTrainerSet_Init(struct BattleSetup *setup, void *saveData, int heapID);
void LONG_CALL BattleSetup_SetAllySideBattlersToPlayer(struct BattleSetup *setup);
void LONG_CALL BattleSetup_CopyPokedexToSaveData(struct BattleSetup *setup, FieldSystem *fsys);
void LONG_CALL BattleSetup_Delete(struct BattleSetup *setup);
void LONG_CALL CallTask_StartEncounter(TaskManager *taskman, struct BattleSetup *setup, s32 effect, s32 bgm, u32 *winFlag);
void LONG_CALL CallTask_BattleTransition(TaskManager *taskman, s32 effect, s32 bgm);
void LONG_CALL CallTask_StartBattle(TaskManager *taskman, struct BattleSetup *setup);
void LONG_CALL CallTask_LeaveOverworld(TaskManager *taskman);
void LONG_CALL CallTask_RestoreOverworld(TaskManager *taskman);
void LONG_CALL CallTask_FadeFromBlack(TaskManager *taskman);
struct Party *LONG_CALL SaveArray_Party_Get(void *saveData);
int LONG_CALL Comm_GetLocalNetId(void);
BOOL LONG_CALL CommState_Exists(void);
BOOL LONG_CALL CommState_IsLinked(void);
void LONG_CALL CommState_RequestShutdown(void);
void LONG_CALL LinkSync_StartSync(u8 state);
BOOL LONG_CALL LinkSync_IsSynced(u8 state);

struct CoopBossState gCoopBoss = { .magic = COOP_BOSS_MAGIC };

// ---------------------------------------------------------------------------
// Failed asserts (vanilla GF_AssertFail, pret src/error_handling.c)
// ---------------------------------------------------------------------------

// Vanilla ignores a failed GF_ASSERT, except while a comm session exists:
// then it shows "A communication error has occurred" and resets. hg-engine
// fails harmless asserts in places (a party slot past its count, then the
// checksum of the empty slot read), and a co-op team boss keeps a session
// open through the boss's script and battle: a reset there loses far more
// than the assert guards (a lost link is the Phase 4 watchdog's, a stuck
// client the host's 60 s timeout). So it is always ignored; the caller of
// the last one is kept for finding it (hg-engine's own replacement, built
// with its debug options only, ignores them too).
void LONG_CALL Coop_GF_AssertFail(void)
{
    gCoopBoss.assertCaller = (u32)__builtin_return_address(0);
    gCoopBoss.assertCount++;
}

// ---------------------------------------------------------------------------
// Talking
// ---------------------------------------------------------------------------

// Vanilla StartMapSceneScript (pret src/script_manager.c).
void LONG_CALL Coop_StartVanillaTalk(FieldSystem *fsys, u16 script, LocalMapObject *obj)
{
    void *env = ScriptEnvironment_New();

    SetupScriptEngine(fsys, env, script, obj, NULL);
    FieldSystem_CreateTask(fsys, Task_RunScripts, env);
}

void LONG_CALL Coop_StartMapSceneScript(FieldSystem *fsys, u16 script, LocalMapObject *obj)
{
    if (obj != NULL && obj == gCoopBoss.talkObj && gCoopBoss.talkStart != NULL)
    {
        gCoopBoss.talkObj = NULL;
        gCoopBoss.talkStart(fsys, obj, script); // starts the vanilla talk itself if it cannot wait
        return;
    }
    Coop_StartVanillaTalk(fsys, script, obj);
}

// ---------------------------------------------------------------------------
// The boss battle
// ---------------------------------------------------------------------------

enum CoopBossBattleState
{
    COOP_BOSS_SYNC,
    COOP_BOSS_TRANSITION,
    COOP_BOSS_LEAVE,
    COOP_BOSS_BATTLE,
    COOP_BOSS_AFTER,
    COOP_BOSS_SHUTDOWN,
    COOP_BOSS_FADE_IN,
    COOP_BOSS_DONE,
    COOP_BOSS_SOLO_DONE,
};

struct CoopBossBattleEnv
{
    struct BattleSetup *setup;
    u32 *winFlag;
    s32 effect;
    s32 bgm;
    u32 trainer;
    int heapID;
    u16 frames;
    u8 seat; // my party's seat: link slot * 2 (frontier multi, ccd battle_link_comm.c)
    u8 won;
};

static void Coop_StartSoloTrainerBattle(TaskManager *taskman, u32 opponentTrainer1, u32 opponentTrainer2, u32 followerTrainerNum,
                                        u32 a4, u32 a5, int heapID, u32 *winFlag);

// Vanilla Task_StartEncounter (pret src/encounter.c) with a link multi: the
// battle runs on copies, then only this player's own seat goes back into
// the save (vanilla's BattleSetup_CopyResultsToSaveData copies seat 0 and
// its profile: on the child that is the parent's).
static BOOL Coop_Task_BossBattle(TaskManager *taskman)
{
    FieldSystem *fsys = taskman->fieldSystem;
    struct CoopBossBattleEnv *env = taskman->env;

    switch (taskman->state)
    {
    case COOP_BOSS_SYNC:
        // The two games reach the boss's TrainerBattle at different times
        // (each turns its pages). The battle's link commands must not reach
        // a game still in the script: the comm core takes an unknown
        // command as a link error (the error screen). Vanilla syncs too.
        if (env->frames++ == 0)
            LinkSync_StartSync(COOP_BOSS_BATTLE_SYNC);
        if (LinkSync_IsSynced(COOP_BOSS_BATTLE_SYNC))
        {
            taskman->state = COOP_BOSS_TRANSITION;
            break;
        }
        if (env->frames < COOP_BOSS_SYNC_FAILSAFE && CommState_IsLinked())
            break;
        // The link is gone: the boss alone, as vanilla.
        if (CommState_Exists())
            CommState_RequestShutdown();
        BattleSetup_Delete(env->setup);
        env->setup = NULL;
        Coop_StartSoloTrainerBattle(taskman, env->trainer, 0, 0, 0, 0, env->heapID, env->winFlag);
        taskman->state = COOP_BOSS_SOLO_DONE;
        break;
    case COOP_BOSS_SOLO_DONE:
        sys_FreeMemoryEz(env);
        return TRUE;
    case COOP_BOSS_TRANSITION:
        MapObjectMan_PauseAllMovement(fsys->mapObjectMan);
        CallTask_BattleTransition(taskman, env->effect, env->bgm);
        taskman->state = COOP_BOSS_LEAVE;
        break;
    case COOP_BOSS_LEAVE:
        CallTask_LeaveOverworld(taskman);
        taskman->state = COOP_BOSS_BATTLE;
        break;
    case COOP_BOSS_BATTLE:
        gCoopCommBlock.track_link_lost_frames = 0;
        gCoopCommBlock.track_link_battle = COOP_LINK_BATTLE_RUNNING; // the Phase 4 watchdog
        CallTask_StartBattle(taskman, env->setup);
        taskman->state = COOP_BOSS_AFTER;
        break;
    case COOP_BOSS_AFTER:
        gCoopCommBlock.track_link_battle = COOP_LINK_BATTLE_NONE;
        __builtin_memcpy(SaveArray_Party_Get(fsys->savedata), env->setup->party[env->seat], COOP_PARTY_CORE_SIZE);
        BattleSetup_CopyPokedexToSaveData(env->setup, fsys);
        env->won = env->setup->winFlag != BATTLE_OUTCOME_LOSE && env->setup->winFlag != BATTLE_OUTCOME_DRAW; // IsBattleResultWin
        if (env->winFlag != NULL)
            *env->winFlag = env->setup->winFlag;
        VarSet(fsys, COOP_VAR_BATTLE_RESULT, (u16)env->setup->winFlag);
#if COOP_DEBUG_TOOLS
        gCoopDebugMonBuffer.pad[0] = (u8)env->setup->winFlag; // probes
#endif
        if (CommState_Exists())
            CommState_RequestShutdown();
        env->frames = 0;
        taskman->state = COOP_BOSS_SHUTDOWN;
        break;
    case COOP_BOSS_SHUTDOWN:
        if (CommState_Exists() && ++env->frames < COOP_SHUTDOWN_FAILSAFE)
            break;
        if (!env->won)
        {
            // As vanilla: the script whites out from here.
            BattleSetup_Delete(env->setup);
            sys_FreeMemoryEz(env);
            return TRUE;
        }
        CallTask_RestoreOverworld(taskman);
        taskman->state = COOP_BOSS_FADE_IN;
        break;
    case COOP_BOSS_FADE_IN:
        MapObjectMan_UnpauseAllMovement(fsys->mapObjectMan);
        CallTask_FadeFromBlack(taskman);
        taskman->state = COOP_BOSS_DONE;
        break;
    case COOP_BOSS_DONE:
        BattleSetup_Delete(env->setup);
        sys_FreeMemoryEz(env);
        return TRUE;
    }
    return FALSE;
}

static BOOL Coop_StartBossBattle(TaskManager *taskman, u32 trainer, int heapID, u32 *winFlag)
{
    FieldSystem *fsys = taskman->fieldSystem;
    struct CoopBossBattleEnv *env = sys_AllocMemory(COOP_HEAP_ID_FIELD3, sizeof(struct CoopBossBattleEnv));
    struct BattleSetup *setup;

    if (env == NULL)
        return FALSE;
    __builtin_memset(env, 0, sizeof(*env));
    setup = BattleSetup_New(COOP_HEAP_ID_FIELD2, COOP_BOSS_BATTLE_TYPE);
    BattleSetup_InitFromFieldSystem(setup, fsys);
    BattleSetup_SetAllySideBattlersToPlayer(setup);
    // The boss in both enemy seats; hg-engine's party builder splits its
    // team between them (Coop_BossSplitParty, while armed).
    setup->trainerId[BATTLER_ENEMY] = (int)trainer;
    setup->trainerId[BATTLER_ENEMY2] = (int)trainer;
    EnemyTrainerSet_Init(setup, fsys->savedata, heapID);
    setup->battleType = COOP_BOSS_BATTLE_TYPE;
    GameStats_Inc(Save_GameStats_Get(fsys->savedata), COOP_GAME_STAT_TRAINER_BATTLES);

    env->setup = setup;
    env->winFlag = winFlag;
    env->trainer = trainer;
    env->heapID = heapID;
    env->effect = BattleSetup_GetWildTransitionEffect(setup);
    env->bgm = BattleSetup_GetWildBattleMusic(setup);
    env->seat = (u8)(Comm_GetLocalNetId() * 2);
    TaskManager_Call(taskman, Coop_Task_BossBattle, env);
    return TRUE;
}

void LONG_CALL Coop_SetupAndStartTrainerBattle(TaskManager *taskman, u32 opponentTrainer1, u32 opponentTrainer2, u32 followerTrainerNum,
                                               u32 a4, u32 a5, int heapID, u32 *winFlag)
{
    if (gCoopBoss.armed)
    {
        // Still armed while the boss's parties are built (the split).
        BOOL started = CommState_Exists() && Coop_StartBossBattle(taskman, opponentTrainer1, heapID, winFlag);

        gCoopBoss.armed = 0;
        if (started)
        {
            gCoopBoss.fought = 1;
            return;
        }
        // No link (it dropped while the boss talked) or no memory: a solo fight.
    }
    Coop_StartSoloTrainerBattle(taskman, opponentTrainer1, opponentTrainer2, followerTrainerNum, a4, a5, heapID, winFlag);
}

// Vanilla (pret src/encounter.c SetupAndStartTrainerBattle).
static void Coop_StartSoloTrainerBattle(TaskManager *taskman, u32 opponentTrainer1, u32 opponentTrainer2, u32 followerTrainerNum,
                                        u32 a4, u32 a5, int heapID, u32 *winFlag)
{
    FieldSystem *fsys = taskman->fieldSystem;
    struct BattleSetup *setup;
    u32 battleType;

    if (opponentTrainer2 != 0 && opponentTrainer1 != opponentTrainer2)
        battleType = followerTrainerNum == 0 ? (BATTLE_TYPE_TRAINER | BATTLE_TYPE_DOUBLES | BATTLE_TYPE_TAG)
                                             : (BATTLE_TYPE_TRAINER | BATTLE_TYPE_DOUBLES | BATTLE_TYPE_MULTI | BATTLE_TYPE_AI);
    else if (opponentTrainer1 == opponentTrainer2)
        battleType = BATTLE_TYPE_TRAINER | BATTLE_TYPE_DOUBLES;
    else
        battleType = BATTLE_TYPE_TRAINER | (a4 ? COOP_BATTLE_TYPE_11 : 0);

    setup = BattleSetup_New(COOP_HEAP_ID_FIELD2, battleType);
    BattleSetup_InitFromFieldSystem(setup, fsys);
    setup->trainerId[BATTLER_ENEMY] = (int)opponentTrainer1;
    setup->trainerId[BATTLER_ENEMY2] = (int)opponentTrainer2;
    setup->trainerId[BATTLER_PLAYER2] = (int)followerTrainerNum;
    EnemyTrainerSet_Init(setup, fsys->savedata, heapID);
    GameStats_Inc(Save_GameStats_Get(fsys->savedata), COOP_GAME_STAT_TRAINER_BATTLES);
    if (a5)
    {
        u8 *followMonSlot = (u8 *)setup + COOP_SETUP_FOLLOW_MON_SLOT;
        if (battleType & BATTLE_TYPE_MULTI)
            followMonSlot[2] = 0;
        else if (!(battleType & BATTLE_TYPE_DOUBLES))
            followMonSlot[1] = 0;
    }
    CallTask_StartEncounter(taskman, setup, BattleSetup_GetWildTransitionEffect(setup), BattleSetup_GetWildBattleMusic(setup), winFlag);
}
