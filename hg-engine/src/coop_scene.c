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
// Vanilla apps over the field (docs/MILESTONE3_PLAN.md §13), Overlay 129.
//
// The order and calls of vanilla's link trade (pret src/launch_application.c
// Task_WirelessTrade) and NPC trade (src/npc_trade_task.c Task_NPCTrade):
//
//   fade out -> leave the overworld -> the app -> (trade: the evolution
//   scene if the received Pokemon evolves by trade) -> restore the overworld
//   -> fade in
//
// Apps: the TradeSequence (Overlay 71), the party screen in vanilla's
// NPC-trade context to choose the Pokemon to offer, and the summary screen
// that chooses a move to forget.
//
// Resident because leaving the overworld unloads Overlay 1, and Overlay 131
// with it (src/overlay.c). The trade scenes run on copies; only the evolved
// Pokemon is copied back into the party.
//
// Only the task is resident; what starts a scene (the env for it) is field
// code: src/field/coop_scene_setup.c (Overlay 131).
// ---------------------------------------------------------------------------

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

const struct CoopAppTemplate gCoopTradeSequenceApp = {
    TradeSequence_Init,
    TradeSequence_Main,
    TradeSequence_Exit,
    COOP_OVERLAY_TRADE_SEQ,
};

const struct CoopAppTemplate gCoopPartyMenuApp = {
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
        if (env->app == NULL)
        {
            // Level-up evolution: the scene on the party Pokemon, B allowed.
            struct CoopLevelEvolution *level = &env->u.level;
            taskman->state = COOP_SCENE_RESTORE;
            if (!Heap_Create(COOP_HEAP_ID_APP, COOP_HEAP_ID_EVOLUTION, COOP_EVOLUTION_HEAP))
                break;
            level->evolution = EvolutionScene_New(SaveArray_Party_Get(fsys->savedata), level->partyMon, level->species,
                                                  Save_PlayerData_GetOptionsAddr(fsys->savedata), PokemonSummary_GetDefaultUnk2C(fsys->savedata),
                                                  SaveData_GetDexPtr(fsys->savedata), Sav2_Bag_get(fsys->savedata),
                                                  Save_GameStats_Get(fsys->savedata), (u32)level->method,
                                                  COOP_EVO_FLAG_CANCEL, COOP_HEAP_ID_EVOLUTION);
            if (level->evolution == NULL)
                Heap_Destroy(COOP_HEAP_ID_EVOLUTION);
            else
                taskman->state = COOP_SCENE_EVOLVING;
            break;
        }
        CallApplicationAsTask(taskman, env->app, &env->u);
        taskman->state = COOP_SCENE_AFTER_APP;
        break;
    case COOP_SCENE_AFTER_APP:
        if (env->orderOut != NULL)
        {
            __builtin_memcpy(env->orderOut, env->u.party.selectedOrder, 6);
            taskman->state = COOP_SCENE_RESTORE;
        }
        else if (env->slotOut != NULL)
        {
            *env->slotOut = ((const u8 *)&env->u)[env->slotOffset];
            taskman->state = COOP_SCENE_RESTORE;
        }
        else
            taskman->state = Coop_TradeSceneStartEvolution(fsys, &env->u.trade) ? COOP_SCENE_EVOLVING : COOP_SCENE_RESTORE;
        break;
    case COOP_SCENE_EVOLVING:
    {
        void *evolution = env->app == NULL ? env->u.level.evolution : env->u.trade.evolution;
        if (!EvolutionScene_IsDone(evolution))
            break;
        if (env->app != NULL)
            PokeCopyPPtoPP(&env->u.trade.received, env->u.trade.partyMon);
        EvolutionScene_Delete(evolution);
        Heap_Destroy(COOP_HEAP_ID_EVOLUTION);
        taskman->state = COOP_SCENE_RESTORE;
        break;
    }
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

BOOL Coop_RunScene(TaskManager *taskman, struct CoopSceneEnv *env)
{
    TaskManager_Call(taskman, Coop_Task_Scene, env);
    return TRUE;
}
