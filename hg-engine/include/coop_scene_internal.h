#ifndef COOP_SCENE_INTERNAL_H
#define COOP_SCENE_INTERNAL_H

#include "types.h"
#include "task.h"
#include "party_menu.h"
#include "pokemon.h"
#include "save.h"

// The scenes' shared state (docs/MILESTONE3_PLAN.md §13). The task that runs
// them is resident (src/coop_scene.c, Overlay 129: the overworld and Overlay
// 131 are gone while it runs); what starts one is field code (Overlay 131,
// src/field/coop_scene_setup.c), which fills a CoopSceneEnv and hands it over.

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
#define COOP_EVO_FLAG_CANCEL   1  // level up: B stops it (pret Task_StartMenu_Evolution)
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

struct CoopTradeScene
{
    struct CoopTradeAnimArgs anim;
    struct PlayerProfile profile;
    struct PartyPokemon sent;
    struct PartyPokemon received;
    struct PartyPokemon *partyMon;
    void *evolution;
};

// A level-up evolution alone (no app): the party Pokemon itself evolves.
struct CoopLevelEvolution
{
    struct PartyPokemon *partyMon;
    void *evolution;
    u16 species;
    int method;
};

struct CoopSceneEnv
{
    const struct CoopAppTemplate *app;
    u8 *slotOut; // party screen: where the chosen slot goes
    u8 *orderOut; // party screen, choosing entries: where the 6-byte order goes
    union
    {
        struct CoopTradeScene trade;
        PartyMenuArgs party;
        struct CoopLevelEvolution level;
    } u;
};

// The apps (resident: the task reads them after Overlay 131 is gone).
extern const struct CoopAppTemplate gCoopTradeSequenceApp;
extern const struct CoopAppTemplate gCoopPartyMenuApp;

// Overlay 129: runs a filled env as a sub-task of taskman (always TRUE).
BOOL LONG_CALL Coop_RunScene(TaskManager *taskman, struct CoopSceneEnv *env);

#endif
