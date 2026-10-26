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

// ---------------------------------------------------------------------------
// Vanilla apps over the field (docs/MILESTONE3_PLAN.md §13), Overlay 129.
//
// The order and calls of vanilla's link trade (pret src/launch_application.c
// Task_WirelessTrade) and NPC trade (src/npc_trade_task.c Task_NPCTrade):
//
//   fade out -> leave the overworld -> the app -> (trade: the evolution
//   scene if the received Pokemon evolves by trade) -> restore the overworld
//   -> fade in
//
// Apps: the TradeSequence (Overlay 71), and the party screen in vanilla's
// NPC-trade context to choose the Pokemon to offer.
//
// Resident because leaving the overworld unloads Overlay 1, and Overlay 131
// with it (src/overlay.c). The trade scenes run on copies; only the evolved
// Pokemon is copied back into the party.
// ---------------------------------------------------------------------------

// pret include/trade_anim.h TRADE_ANIM_WORK: the TradeSequence app's args.
struct CoopTradeAnimArgs
{
    struct BoxPokemon *sentBoxMon;
    struct BoxPokemon *receivedBoxMon;
    struct PlayerProfile *partnerProfile; // name shown as the other trainer
    int timeOfDay;                        // 0 day, 1 evening, 2 night (background)
    int mode;                             // COOP_TRADE_ANIM_FULL
    void *options;
};

// pret OverlayManagerTemplate.
struct CoopAppTemplate
{
    BOOL (*init)(void *man, int *state);
    BOOL (*main)(void *man, int *state);
    BOOL (*exit)(void *man, int *state);
    u32 overlayId;
};

#define COOP_TRADE_ANIM_FULL   1  // TRADE_SEQUENCE_MODE_FULL: send, exchange, receive
#define COOP_OVERLAY_TRADE_SEQ 71 // FS_OVERLAY_ID(OVY_71)
#define COOP_OVERLAY_NONE      0xFFFFFFFF
#define COOP_EVO_FLAG_TRADE    4  // EVO_FLAG_TRADE: no B to stop it
#define COOP_HEAP_ID_APP       3  // HEAP_ID_3, the applications' parent heap
#define COOP_HEAP_ID_EVOLUTION 26 // what vanilla's link trade gives the scene
#define COOP_EVOLUTION_HEAP    0x30000
#define COOP_HEAP_ID_FIELD3    32 // field task heap

// Overlay 71 (pret asm/overlay_71.s labels).
BOOL TradeSequence_Init(void *man, int *state);
BOOL TradeSequence_Main(void *man, int *state);
BOOL TradeSequence_Exit(void *man, int *state);
// The party screen (arm9; Exit is hg-engine's PokeListProc_End).
BOOL PartyMenuApp_Init(void *man, int *state);
BOOL PartyMenuApp_Main(void *man, int *state);
// Field transitions (pret asm/unk_02055244.s labels).
void LONG_CALL PaletteFadeUntilFinished(TaskManager *taskman);
void LONG_CALL CallTask_LeaveOverworld(TaskManager *taskman);
void LONG_CALL CallTask_RestoreOverworld(TaskManager *taskman);
void LONG_CALL CallTask_FadeFromBlack(TaskManager *taskman);
// pret src/task.c (sub_02050624 before the rename).
void LONG_CALL CallApplicationAsTask(TaskManager *taskman, const struct CoopAppTemplate *tmpl, void *args);
// Evolution scene (pret asm/unk_020755E8.s sub_02075A7C / D3C / D4C).
void *LONG_CALL EvolutionScene_New(struct Party *party, struct PartyPokemon *mon, int species, void *options, u32 summaryDefault,
                                   void *pokedex, void *bag, void *gameStats, u32 evolutionCondition, u32 flags, u32 heapId);
BOOL LONG_CALL EvolutionScene_IsDone(void *scene);
void LONG_CALL EvolutionScene_Delete(void *scene);
// pret asm sub_02088288 (ccd PokemonSummary_GetDefaultUnk2C): the scene's
// summary-screen argument, read from the save.
u32 LONG_CALL PokemonSummary_GetDefaultUnk2C(void *saveData);
// Heaps (pret src/heap.c Heap_Create / Heap_Destroy, CreateHeap /
// DestroyHeap in pret's asm history).
BOOL LONG_CALL Heap_Create(u32 parent, u32 child, u32 size);
void LONG_CALL Heap_Destroy(u32 heapId);
void *LONG_CALL Save_PlayerData_GetOptionsAddr(void *saveData);
void *LONG_CALL Save_Mailbox_Get(void *saveData);
struct Party *LONG_CALL SaveArray_Party_Get(void *saveData);
void LONG_CALL PokeCopyPPtoPP(struct PartyPokemon *src, struct PartyPokemon *dst);

_Static_assert(__builtin_offsetof(FieldSystem, menuInputState) == 0x10C, "pret FieldSystem.menuInputState");

enum CoopSceneState
{
    COOP_SCENE_FADE_OUT,
    COOP_SCENE_LEAVE,
    COOP_SCENE_APP,
    COOP_SCENE_AFTER_APP,
    COOP_SCENE_EVOLVING,
    COOP_SCENE_RESTORE,
    COOP_SCENE_FADE_IN,
    COOP_SCENE_DONE,
};

struct CoopTradeScene
{
    struct CoopTradeAnimArgs anim;
    struct PlayerProfile profile;
    struct PartyPokemon sent;
    struct PartyPokemon received;
    struct PartyPokemon *partyMon;
    void *evolution;
};

struct CoopSceneEnv
{
    const struct CoopAppTemplate *app;
    u8 *slotOut; // party screen: where the chosen slot goes
    union
    {
        struct CoopTradeScene trade;
        PartyMenuArgs party;
    } u;
};

static const struct CoopAppTemplate sCoopTradeSequenceApp = {
    TradeSequence_Init,
    TradeSequence_Main,
    TradeSequence_Exit,
    COOP_OVERLAY_TRADE_SEQ,
};

static const struct CoopAppTemplate sCoopPartyMenuApp = {
    PartyMenuApp_Init,
    PartyMenuApp_Main,
    (BOOL (*)(void *, int *))PokeListProc_End,
    COOP_OVERLAY_NONE,
};

// Starts the evolution scene when the received Pokemon evolves by trade, as
// vanilla's link trade does: the held item is passed for the trade-item
// evolutions, and the scene removes it.
static BOOL Coop_TradeSceneStartEvolution(FieldSystem *fsys, struct CoopTradeScene *trade)
{
    int method = 0;
    u16 heldItem = (u16)GetMonData(&trade->received, MON_DATA_HELD_ITEM, NULL);
    u16 species = GetMonEvolution(NULL, &trade->received, EVOCTX_TRADE, heldItem, &method);

    if (species == SPECIES_NONE || species > MAX_SPECIES_INCLUDING_FORMS)
        return FALSE;
    if (!Heap_Create(COOP_HEAP_ID_APP, COOP_HEAP_ID_EVOLUTION, COOP_EVOLUTION_HEAP))
        return FALSE;
    trade->evolution = EvolutionScene_New(NULL, &trade->received, species, Save_PlayerData_GetOptionsAddr(fsys->savedata),
                                          PokemonSummary_GetDefaultUnk2C(fsys->savedata), SaveData_GetDexPtr(fsys->savedata),
                                          Sav2_Bag_get(fsys->savedata), Save_GameStats_Get(fsys->savedata), (u32)method,
                                          COOP_EVO_FLAG_TRADE, COOP_HEAP_ID_EVOLUTION);
    if (trade->evolution == NULL)
    {
        Heap_Destroy(COOP_HEAP_ID_EVOLUTION);
        return FALSE;
    }
    return TRUE;
}

static BOOL Coop_Task_Scene(TaskManager *taskman)
{
    FieldSystem *fsys = taskman->fieldSystem;
    struct CoopSceneEnv *env = taskman->env;

    switch (taskman->state)
    {
    case COOP_SCENE_FADE_OUT:
        PaletteFadeUntilFinished(taskman);
        taskman->state = COOP_SCENE_LEAVE;
        break;
    case COOP_SCENE_LEAVE:
        CallTask_LeaveOverworld(taskman);
        taskman->state = COOP_SCENE_APP;
        break;
    case COOP_SCENE_APP:
        CallApplicationAsTask(taskman, env->app, &env->u);
        taskman->state = COOP_SCENE_AFTER_APP;
        break;
    case COOP_SCENE_AFTER_APP:
        if (env->slotOut != NULL)
        {
            *env->slotOut = env->u.party.partySlot;
            taskman->state = COOP_SCENE_RESTORE;
        }
        else
            taskman->state = Coop_TradeSceneStartEvolution(fsys, &env->u.trade) ? COOP_SCENE_EVOLVING : COOP_SCENE_RESTORE;
        break;
    case COOP_SCENE_EVOLVING:
        if (!EvolutionScene_IsDone(env->u.trade.evolution))
            break;
        PokeCopyPPtoPP(&env->u.trade.received, env->u.trade.partyMon);
        EvolutionScene_Delete(env->u.trade.evolution);
        Heap_Destroy(COOP_HEAP_ID_EVOLUTION);
        env->u.trade.evolution = NULL;
        taskman->state = COOP_SCENE_RESTORE;
        break;
    case COOP_SCENE_RESTORE:
        CallTask_RestoreOverworld(taskman);
        taskman->state = COOP_SCENE_FADE_IN;
        break;
    case COOP_SCENE_FADE_IN:
        CallTask_FadeFromBlack(taskman);
        taskman->state = COOP_SCENE_DONE;
        break;
    case COOP_SCENE_DONE:
        sys_FreeMemoryEz(env);
        return TRUE;
    }
    return FALSE;
}

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
    struct CoopSceneEnv *env = Coop_NewScene(&sCoopTradeSequenceApp);
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
    TaskManager_Call(taskman, Coop_Task_Scene, env);
    return TRUE;
}

// pret PartyMenu_CreateArgs + PartyMenu_LaunchApp_InGameTrade.
BOOL Coop_CallTradePartySelect(TaskManager *taskman, u8 cursor, u8 *slotOut)
{
    FieldSystem *fsys = taskman->fieldSystem;
    struct CoopSceneEnv *env = Coop_NewScene(&sCoopPartyMenuApp);
    PartyMenuArgs *args;

    if (env == NULL)
        return FALSE;
    args = &env->u.party;
    args->party = SaveArray_Party_Get(fsys->savedata);
    args->bag = Sav2_Bag_get(fsys->savedata);
    args->mailbox = Save_Mailbox_Get(fsys->savedata);
    args->options = Save_PlayerData_GetOptionsAddr(fsys->savedata);
    args->context = PARTY_MENU_CONTEXT_INGAME_TRADE;
    args->fieldSystem = fsys;
    args->menuInputStatePtr = &fsys->menuInputState;
    args->partySlot = cursor;
    env->slotOut = slotOut;
    *slotOut = COOP_PARTY_SELECT_CANCEL;
    TaskManager_Call(taskman, Coop_Task_Scene, env);
    return TRUE;
}
