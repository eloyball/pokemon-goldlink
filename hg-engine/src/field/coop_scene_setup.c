#include "config.h"
#include "types.h"

#include "bag.h"
#include "constants/game_stats.h"
#include "party_menu.h"
#include "pokemon.h"
#include "rtc.h"
#include "save.h"
#include "system.h"
#include "task.h"
#include "coop_sync.h"
#include "coop_scene.h"
#include "coop_scene_internal.h"

// ---------------------------------------------------------------------------
// Starting the scenes (docs/MILESTONE3_PLAN.md §13), Overlay 131: each fills
// the resident task's env (src/coop_scene.c) and hands it over. Kept out of
// Overlay 129, which has little room (KNOWN_RISKS.md #27).
// ---------------------------------------------------------------------------

_Static_assert(__builtin_offsetof(FieldSystem, menuInputState) == 0x10C, "pret FieldSystem.menuInputState");

static struct CoopSceneEnv *Coop_NewScene(const struct CoopAppTemplate *app)
{
    struct CoopSceneEnv *env = sys_AllocMemory(COOP_HEAP_ID_FIELD3, sizeof(struct CoopSceneEnv));

    if (env != NULL)
    {
        __builtin_memset(env, 0, sizeof(*env));
        env->app = app;
    }
    return env;
}

BOOL Coop_CallTradeScene(TaskManager *taskman, const struct PartyPokemon *sent, struct PartyPokemon *partyMon, const u16 *partnerName)
{
    FieldSystem *fsys = taskman->fieldSystem;
    struct CoopSceneEnv *env = Coop_NewScene(&gCoopTradeSequenceApp);
    struct CoopTradeScene *trade;
    u32 i, timeOfDay;

    if (env == NULL)
        return FALSE;
    trade = &env->u.trade;
    // The game's 0xEC bytes: hg-engine's struct is 0xF0 (KNOWN_RISKS #16).
    __builtin_memcpy(&trade->sent, sent, COOP_PARTY_MON_SIZE);
    __builtin_memcpy(&trade->received, partyMon, COOP_PARTY_MON_SIZE);
    trade->partyMon = partyMon;

    // The partner as a trainer profile: mine, renamed.
    __builtin_memcpy(&trade->profile, Sav2_PlayerData_GetProfileAddr(fsys->savedata), sizeof(trade->profile));
    for (i = 0; i < 7 && partnerName[i] != 0xFFFF; i++)
        trade->profile.name[i] = partnerName[i];
    for (; i < 8; i++)
        trade->profile.name[i] = 0xFFFF;

    timeOfDay = GF_RTC_GetTimeOfDay();
    trade->anim.sentBoxMon = &trade->sent.box;
    trade->anim.receivedBoxMon = &trade->received.box;
    trade->anim.partnerProfile = &trade->profile;
    trade->anim.timeOfDay = (timeOfDay <= RTC_TIMEOFDAY_DAY) ? 0 : (timeOfDay == RTC_TIMEOFDAY_EVE) ? 1 : 2;
    trade->anim.mode = COOP_TRADE_ANIM_FULL;
    trade->anim.options = Save_PlayerData_GetOptionsAddr(fsys->savedata);
    return Coop_RunScene(taskman, env);
}

BOOL Coop_CallEvolutionScene(TaskManager *taskman, struct PartyPokemon *partyMon, u16 species, int method)
{
    struct CoopSceneEnv *env = Coop_NewScene(NULL);

    if (env == NULL)
        return FALSE;
    env->u.level.partyMon = partyMon;
    env->u.level.species = species;
    env->u.level.method = method;
    return Coop_RunScene(taskman, env);
}

// The party screen's arguments for this save (pret PartyMenu_CreateArgs).
static PartyMenuArgs *Coop_PartyMenuArgs(FieldSystem *fsys, struct CoopSceneEnv *env, u8 context)
{
    PartyMenuArgs *args = &env->u.party;

    args->party = SaveArray_Party_Get(fsys->savedata);
    args->bag = Sav2_Bag_get(fsys->savedata);
    args->mailbox = Save_Mailbox_Get(fsys->savedata);
    args->options = Save_PlayerData_GetOptionsAddr(fsys->savedata);
    args->context = context;
    args->fieldSystem = fsys;
    args->menuInputStatePtr = &fsys->menuInputState;
    return args;
}

// pret TaskManager_LaunchPartyMenu_UnionRoomBattleSelect, without a ruleset.
BOOL Coop_CallPartySelectEntries(TaskManager *taskman, u8 count, u8 *orderOut)
{
    struct CoopSceneEnv *env = Coop_NewScene(&gCoopPartyMenuApp);
    PartyMenuArgs *args;

    if (env == NULL)
        return FALSE;
    args = Coop_PartyMenuArgs(taskman->fieldSystem, env, PARTY_MENU_CONTEXT_UNION_ROOM_BATTLE_SELECT);
    args->minMonsToSelect = count;
    args->maxMonsToSelect = count;
    args->maxLevel = 100;
    env->orderOut = orderOut;
    __builtin_memset(orderOut, 0, 6);
    return Coop_RunScene(taskman, env);
}

// pret PartyMenu_CreateArgs + PartyMenu_LaunchApp_InGameTrade.
BOOL Coop_CallTradePartySelect(TaskManager *taskman, u8 cursor, u8 *slotOut)
{
    FieldSystem *fsys = taskman->fieldSystem;
    struct CoopSceneEnv *env = Coop_NewScene(&gCoopPartyMenuApp);
    PartyMenuArgs *args;

    if (env == NULL)
        return FALSE;
    args = Coop_PartyMenuArgs(fsys, env, PARTY_MENU_CONTEXT_INGAME_TRADE);
    args->partySlot = cursor;
    env->slotOut = slotOut;
    *slotOut = COOP_PARTY_SELECT_CANCEL;
    return Coop_RunScene(taskman, env);
}
