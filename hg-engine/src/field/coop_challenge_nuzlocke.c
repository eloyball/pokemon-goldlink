#include "config.h"
#include "types.h"

#include "constants/battle_constants.h"
#include "constants/item.h"
#include "constants/species.h"
#include "bag.h"
#include "encounter.h"
#include "map_events_internal.h"
#include "pokemon.h"
#include "save.h"
#include "script.h"
#include "task.h"
#include "coop_sync.h"
#include "coop_dialog.h"
#include "coop_challenge.h"

// hg-engine's PC storage (src/pokemon_storage_system.c) is in Overlay 129. Its
// addresses reach this overlay as plain absolute symbols (build/rom_gen.ld), so
// a direct `bl` would enter its Thumb code in ARM state: call through a register.
#pragma long_calls
#include "pokemon_storage_system.h"
void PCStorage_DeleteBoxMonByIndexPair(PCStorage *storage, u32 boxno, u32 slotno);
void PCStorage_SetBoxName(PCStorage *storage, u32 boxno, const String *src);
int PCStorage_CountEmptySpotsInBox(PCStorage *storage, u32 boxno);
#pragma long_calls_off

// ---------------------------------------------------------------------------
// Milestone 5 Phase 3: Nuzlocke (docs/MILESTONE5_PLAN.md §5.4), Overlay 131.
// The battle side (marking deaths, blocking balls, nicknames) is in
// src/battle/coop_challenge_battle.c.
// ---------------------------------------------------------------------------

// pret: Overlay 2 (field) and the Safari save (docs/MILESTONE5_PLAN.md §11.1).
int LONG_CALL ov02_0224E340(FieldSystem *fsys);                  // the player's slot in the Safari layout, 0-5
void *LONG_CALL SafariZone_GetAreaSet(void *safariZone, s32 setNo); // 3: the current set
u16 LONG_CALL MapHeader_GetMapSec(u32 mapNo);
void *LONG_CALL SaveArray_Party_Get(void *saveData);


#define COOP_SAVE_SAFARI_ZONE  35 // pret SAVE_SAFARI_ZONE
#define COOP_SAVE_PCSTORAGE    41 // pret SAVE_PCSTORAGE
#define COOP_SAFARI_AREA_SIZE  122 // pret SAFARIZONE_AREA: 2 + 30 objects of 4 bytes
#define COOP_SAFARI_SLOTS      6
#define COOP_FAMILY_BYTES      80 // 640 families (the pool has 542)
#define COOP_HEAP_FIELD        11

// pret Daycare: two DaycareMon of 0xEC (BoxPokemon 0x88, DaycareMail 0x60, steps),
// then egg_pid at 0x1D8 and egg_cycles at 0x1DC; 0x1E0 bytes in all. hg-engine's
// Daycare struct (include/pokemon.h) pads DaycareMon to 240 bytes and so puts
// egg_pid at 0x1E0, past the end of the save block: write the byte offset instead.
#define COOP_DAYCARE_EGG_PID 0x1D8
_Static_assert(sizeof(MAP_EVENTS) == 0x920, "the wild encounter table follows MAP_EVENTS (pret MapEvents.wildEncounters)");

// Overlay 131's data is reloaded with the overlay (after every battle and app),
// so these restart then: the family list is rebuilt, the box sweep runs again.
static u8 sFamilies[COOP_FAMILY_BYTES];
static u8 sFamiliesReady;
static u8 sSweepBox; // next PC box to sweep; NUM_PC_BOXES: done since the overlay loaded

static BOOL Coop_Bit(const u8 *bits, u32 i)
{
    return (bits[i >> 3] >> (i & 7)) & 1;
}

static void Coop_SetBit(u8 *bits, u32 i)
{
    bits[i >> 3] |= (u8)(1 << (i & 7));
}

static PCStorage *Coop_PC(FieldSystem *fsys)
{
    return SaveArray_Get(fsys->savedata, COOP_SAVE_PCSTORAGE);
}

static BOOL Coop_IsGraveyard(const struct CoopChallengeSave *save, u32 box)
{
    return box == COOP_GRAVEYARD_BOX || (box == COOP_GRAVEYARD_BOX2 && (save->state & COOP_CHALLENGE_GRAVE2));
}

// ---------------------------------------------------------------------------
// Locations and the dupes clause
// ---------------------------------------------------------------------------

u32 Coop_ChallengeLocation(FieldSystem *fsys, u32 fightType, u32 map)
{
    u32 mapsec;

    if (fightType & BATTLE_TYPE_BUG_CONTEST)
        return COOP_LOCATION_CONTEST;
    if (fightType & BATTLE_TYPE_SAFARI)
    {
        const u8 *set = SafariZone_GetAreaSet(SaveArray_Get(fsys->savedata, COOP_SAVE_SAFARI_ZONE), 3);
        int slot = ov02_0224E340(fsys);
        u32 biome;

        if (set == NULL || slot < 0 || slot >= COOP_SAFARI_SLOTS)
            return 0xFF;
        biome = set[slot * COOP_SAFARI_AREA_SIZE];
        return biome < COOP_SAFARI_AREAS ? COOP_LOCATION_SAFARI + biome : 0xFF;
    }
    mapsec = MapHeader_GetMapSec(map);
    if (mapsec >= COOP_MAPSEC_FIRST && mapsec < COOP_MAPSEC_FIRST + COOP_LOCATION_SAFARI)
        return mapsec - COOP_MAPSEC_FIRST;
    return 0xFF;
}

static u32 Coop_BoxMonValue(struct BoxPokemon *mon)
{
    return GetBoxMonData(mon, MON_DATA_SPECIES, NULL) | (GetBoxMonData(mon, MON_DATA_FORM, NULL) << 11);
}

static void Coop_AddFamily(u32 value)
{
    const struct CoopPoolEntry *e = Coop_PoolFind(value);

    if (e != NULL && e->family < COOP_FAMILY_BYTES * 8)
        Coop_SetBit(sFamilies, e->family);
}

static void Coop_AddFamilyOf(struct BoxPokemon *mon)
{
    BOOL fast = BoxMonSetFastModeOn(mon);

    if (GetBoxMonData(mon, MON_DATA_SPECIES_EXISTS, NULL) && !GetBoxMonData(mon, MON_DATA_IS_EGG, NULL)
        && (GetBoxMonData(mon, MON_DATA_RESERVED_114, NULL) & COOP_MON_WILD_CATCH))
        Coop_AddFamily(Coop_BoxMonValue(mon));
    BoxMonSetFastModeOff(mon, fast);
}

// The families this player caught in the wild and still owns (party, boxes,
// Graveyard): a released catch is forgotten (E44). Built once per load.
static void Coop_BuildFamilies(FieldSystem *fsys)
{
    struct Party *party = SaveArray_Party_Get(fsys->savedata);
    PCStorage *pc = Coop_PC(fsys);
    u32 i, b, s;

    for (i = 0; i < COOP_FAMILY_BYTES; i++)
        sFamilies[i] = 0;
    for (i = 0; i < (u32)party->count; i++)
        Coop_AddFamilyOf(&Party_GetMonByIndex(party, i)->box);
    for (b = 0; b < NUM_PC_BOXES; b++)
        for (s = 0; s < MONS_PER_BOX; s++)
            Coop_AddFamilyOf(&pc->boxes[b].mons[s]);
    sFamiliesReady = 1;
}

const u8 *Coop_ChallengeFamilyMask(void)
{
    if (!sFamiliesReady && gFieldSysPtr != NULL)
        Coop_BuildFamilies(gFieldSysPtr);
    return sFamilies;
}

BOOL Coop_ChallengeFamilyCaught(u32 value)
{
    const struct CoopPoolEntry *e = Coop_PoolFind(value);

    return e != NULL && e->family < COOP_FAMILY_BYTES * 8 && Coop_Bit(Coop_ChallengeFamilyMask(), e->family);
}

// The table section (land by time of day, surf, a rod, Rock Smash) of the map
// that holds `original`, as a list of u16 values with a stride.
static BOOL Coop_FindSection(const EncounterData *enc, u32 original, const u16 **first, u32 *count, u32 *stride)
{
    static const u8 sCounts[] = { NUM_ENCOUNTERS_SURF, NUM_ENCOUNTERS_ROCKSMASH, NUM_ENCOUNTERS_FISH, NUM_ENCOUNTERS_FISH, NUM_ENCOUNTERS_FISH };
    const EncounterDataSlot *slots[] = { enc->surfSlots, enc->rockSmashSlots, enc->oldRodSlots, enc->goodRodSlots, enc->superRodSlots };
    const u16 *land[] = { enc->landSlots.speciesMorning, enc->landSlots.speciesDay, enc->landSlots.speciesNight };
    u32 i, j;

    for (i = 0; i < 3; i++)
        for (j = 0; j < NUM_ENCOUNTERS_LAND; j++)
            if (land[i][j] == original)
            {
                *first = land[i];
                *count = NUM_ENCOUNTERS_LAND;
                *stride = 1;
                return TRUE;
            }
    for (i = 0; i < NELEMS(slots); i++)
        for (j = 0; j < sCounts[i]; j++)
            if (slots[i][j].species == original)
            {
                *first = &slots[i][0].species;
                *count = sCounts[i];
                *stride = sizeof(EncounterDataSlot) / sizeof(u16);
                return TRUE;
            }
    return FALSE;
}

u32 Coop_ChallengeTableReroll(FieldSystem *fsys, u32 original)
{
    const EncounterData *enc;
    const u16 *first;
    u32 count, stride, i;

    if (fsys == NULL || fsys->map_events == NULL)
        return 0;
    enc = (const EncounterData *)(fsys->map_events + 1);
    if (!Coop_FindSection(enc, original, &first, &count, &stride))
        return 0;
    for (i = 0; i < 10; i++) // the draft: at most 10 tries
    {
        u32 candidate = first[(gf_rand() % count) * stride];
        if (candidate != 0 && candidate != original && !Coop_ChallengeFamilyCaught(candidate))
            return candidate;
    }
    return 0;
}

// ---------------------------------------------------------------------------
// After a wild battle: was the first encounter caught?
// ---------------------------------------------------------------------------

// Where a Pokemon with this personality value is now (party first, then the
// PC: a catch goes to the PC when the party is full), or NULL.
static struct BoxPokemon *Coop_FindByPid(FieldSystem *fsys, u32 pid)
{
    struct Party *party = SaveArray_Party_Get(fsys->savedata);
    PCStorage *pc = Coop_PC(fsys);
    u32 i, b, s, p;

    for (i = 0; i < (u32)party->count; i++)
    {
        struct BoxPokemon *mon = &Party_GetMonByIndex(party, i)->box;
        if (GetBoxMonData(mon, MON_DATA_SPECIES_EXISTS, NULL) && GetBoxMonData(mon, MON_DATA_PERSONALITY, NULL) == pid)
            return mon;
    }
    for (b = 0; b < NUM_PC_BOXES; b++)
        for (s = 0; s < MONS_PER_BOX; s++)
        {
            struct BoxPokemon *mon = &pc->boxes[b].mons[s];
            __builtin_memcpy(&p, mon, sizeof(p)); // the personality value is stored unencrypted at offset 0
            if (p == pid && GetBoxMonData(mon, MON_DATA_SPECIES_EXISTS, NULL))
                return mon;
        }
    return NULL;
}

void Coop_ChallengeResolvePending(FieldSystem *fsys, struct CoopChallengeSave *save)
{
    struct BoxPokemon *caught = Coop_FindByPid(fsys, save->pending_pid);
    u32 loc = save->pending_loc;
    u16 bits;

    if (caught != NULL)
    {
        // A catch: its family counts for the dupes clause; a first encounter's
        // catch is also this location's linked Pokemon (§5.4.8).
        bits = GetBoxMonData(caught, MON_DATA_RESERVED_114, NULL) | COOP_MON_WILD_CATCH;
        if (save->pending_flags & COOP_PENDING_FIRST)
        {
            bits |= COOP_MON_LINKED;
            if (loc >= COOP_LOCATION_SAFARI && loc < COOP_LOCATION_SAFARI + COOP_SAFARI_AREAS)
                save->safari_pid[loc - COOP_LOCATION_SAFARI] = save->pending_pid;
            else if (loc == COOP_LOCATION_CONTEST)
                save->contest_pid = save->pending_pid;
        }
        SetBoxMonData(caught, MON_DATA_RESERVED_114, &bits);
        if (sFamiliesReady)
            Coop_AddFamily(Coop_BoxMonValue(caught));
    }
    else if ((save->pending_flags & COOP_PENDING_FIRST) && loc < COOP_LOCATION_BYTES * 8)
    {
        Coop_SetBit(save->failed, loc); // fainted, fled, blown away, or the player lost (Soul-Link, Phase 6)
    }
    save->pending_flags = 0;
    save->pending_loc = 0xFF;
}

// ---------------------------------------------------------------------------
// The Graveyard
// ---------------------------------------------------------------------------

struct CoopGraveTaskEnv
{
    struct CoopDialog dialog;
    u16 name[12];
    u8 state;
    u8 kind; // 0 laid to rest, 1 withdrawn from the PC
};

static BOOL Coop_Task_GraveMessage(TaskManager *taskman)
{
    FieldSystem *fsys = taskman->fieldSystem;
    struct CoopGraveTaskEnv *env = taskman->env;

    switch (env->state)
    {
    case 0:
        MapObjectMan_PauseAllMovement(fsys->mapObjectMan);
        Coop_DialogOpen(fsys, &env->dialog);
        env->dialog.args[0] = env->name;
        Coop_DialogPrint(&env->dialog, env->kind == 0 ? "@ has fallen...\n@ was laid to rest in the Graveyard."
                                                      : "@ came out of the PC\nto join your party.");
        env->state = 1;
        break;
    case 1:
        if (Coop_DialogIsPrinting(&env->dialog) || !Coop_DialogButtonPressed())
            break;
        Coop_DialogClose(fsys, &env->dialog);
        MapObjectMan_UnpauseAllMovement(fsys->mapObjectMan);
        sys_FreeMemoryEz(env);
        return TRUE;
    }
    return FALSE;
}

// Starts the message task; FALSE when it can't (the caller retries next frame).
static BOOL Coop_StartGraveMessage(FieldSystem *fsys, const u16 *name, u8 kind)
{
    struct CoopGraveTaskEnv *env;
    u32 i;

    if (!Coop_StartFieldTask(fsys, Coop_Task_GraveMessage, sizeof(struct CoopGraveTaskEnv)))
        return FALSE;
    env = ((TaskManager *)fsys->taskman)->env;
    for (i = 0; i < NELEMS(env->name); i++)
        env->name[i] = name[i];
    env->name[NELEMS(env->name) - 1] = 0xFFFF;
    env->kind = kind;
    return TRUE;
}

static void Coop_NameGraveyard(FieldSystem *fsys, struct CoopChallengeSave *save, u32 box, const char *name)
{
    String *text = String_New(24, COOP_HEAP_FIELD);

    if (text == NULL)
        return;
    Coop_SetStringFromAscii(text, name, NULL);
    PCStorage_SetBoxName(Coop_PC(fsys), box, text);
    String_Delete(text);
    if (box == COOP_GRAVEYARD_BOX)
        save->state |= COOP_CHALLENGE_GRAVE_NAMED;
}

// The Graveyard box with room, opening the second one when the first is full (Q11).
static int Coop_GraveyardWithRoom(FieldSystem *fsys, struct CoopChallengeSave *save)
{
    PCStorage *pc = Coop_PC(fsys);

    if (PCStorage_CountEmptySpotsInBox(pc, COOP_GRAVEYARD_BOX) > 0)
        return COOP_GRAVEYARD_BOX;
    if (!(save->state & COOP_CHALLENGE_GRAVE2))
    {
        save->state |= COOP_CHALLENGE_GRAVE2;
        Coop_NameGraveyard(fsys, save, COOP_GRAVEYARD_BOX2, "GRAVEYARD 2");
    }
    return PCStorage_CountEmptySpotsInBox(pc, COOP_GRAVEYARD_BOX2) > 0 ? COOP_GRAVEYARD_BOX2 : -1;
}

// The first box (not the Graveyard) with room, or -1.
static int Coop_BoxWithRoom(FieldSystem *fsys, struct CoopChallengeSave *save)
{
    PCStorage *pc = Coop_PC(fsys);
    u32 b;

    for (b = 0; b < NUM_PC_BOXES; b++)
        if (!Coop_IsGraveyard(save, b) && PCStorage_CountEmptySpotsInBox(pc, b) > 0)
            return (int)b;
    return -1;
}

// A dead party Pokemon goes to the Graveyard; its held item to the bag, unless
// it is mail or the bag has no room (E35). TRUE when one was buried.
static BOOL Coop_BuryFromParty(FieldSystem *fsys, struct CoopChallengeSave *save)
{
    struct Party *party = SaveArray_Party_Get(fsys->savedata);
    u32 i;

    for (i = 0; i < (u32)party->count; i++)
    {
        struct PartyPokemon *mon = Party_GetMonByIndex(party, i);
        u16 name[12];
        u32 item;
        int grave;

        if (!(GetMonData(mon, MON_DATA_RESERVED_114, NULL) & COOP_MON_DEAD))
            continue;
        grave = Coop_GraveyardWithRoom(fsys, save);
        if (grave < 0)
            return FALSE; // both Graveyard boxes full: it stays (logged, KNOWN_RISKS)
        item = GetMonData(mon, MON_DATA_HELD_ITEM, NULL);
        if (item != ITEM_NONE && !IS_ITEM_MAIL(item) && Bag_AddItem(Sav2_Bag_get(fsys->savedata), item, 1, COOP_HEAP_FIELD))
        {
            u32 none = ITEM_NONE;
            SetMonData(mon, MON_DATA_HELD_ITEM, &none);
        }
        GetMonData(mon, MON_DATA_NICKNAME, name);
        name[11] = 0xFFFF;
        if (!PCStorage_PlaceMonInBoxFirstEmptySlot(Coop_PC(fsys), grave, &mon->box))
            return FALSE;
        PokeParty_Delete(party, i);
        if (save->deaths < 0xFFFF)
            save->deaths++;
        Coop_StartGraveMessage(fsys, name, 0);
        return TRUE;
    }
    return FALSE;
}

// One PC box per frame: a dead Pokemon outside the Graveyard goes back in
// (a withdrawal from it is undone, §5.4.5), a living one inside it moves out.
static void Coop_SweepBox(FieldSystem *fsys, struct CoopChallengeSave *save, u32 box)
{
    PCStorage *pc = Coop_PC(fsys);
    BOOL graveyard = Coop_IsGraveyard(save, box);
    u32 s;

    for (s = 0; s < MONS_PER_BOX; s++)
    {
        struct BoxPokemon *mon = &pc->boxes[box].mons[s];
        BOOL fast = BoxMonSetFastModeOn(mon);
        BOOL exists = GetBoxMonData(mon, MON_DATA_SPECIES_EXISTS, NULL);
        BOOL dead = (GetBoxMonData(mon, MON_DATA_RESERVED_114, NULL) & COOP_MON_DEAD) != 0;
        int to;

        BoxMonSetFastModeOff(mon, fast);
        if (!exists || dead == graveyard)
            continue;
        to = dead ? Coop_GraveyardWithRoom(fsys, save) : Coop_BoxWithRoom(fsys, save);
        if (to < 0 || (u32)to == box)
            continue;
        if (PCStorage_PlaceMonInBoxFirstEmptySlot(pc, to, mon))
            PCStorage_DeleteBoxMonByIndexPair(pc, box, s);
    }
}

// An empty party with living Pokemon in the PC: the first of them joins it, so
// the game never runs the overworld without a Pokemon outside Ghost Mode (E2).
static BOOL Coop_WithdrawFirstLiving(FieldSystem *fsys, struct CoopChallengeSave *save)
{
    struct Party *party = SaveArray_Party_Get(fsys->savedata);
    PCStorage *pc = Coop_PC(fsys);
    u32 b, s;

    for (b = 0; b < NUM_PC_BOXES; b++)
    {
        if (Coop_IsGraveyard(save, b))
            continue;
        for (s = 0; s < MONS_PER_BOX; s++)
        {
            struct BoxPokemon *box = &pc->boxes[b].mons[s];
            struct PartyPokemon *mon;
            u16 name[12];

            if (!GetBoxMonData(box, MON_DATA_SPECIES_EXISTS, NULL) || GetBoxMonData(box, MON_DATA_IS_EGG, NULL)
                || (GetBoxMonData(box, MON_DATA_RESERVED_114, NULL) & COOP_MON_DEAD))
                continue;
            mon = AllocMonZeroed(COOP_HEAP_FIELD);
            if (mon == NULL)
                return FALSE;
            CopyBoxPokemonToPokemon(box, mon);
            if (PokeParty_Add(party, mon))
            {
                PCStorage_DeleteBoxMonByIndexPair(pc, b, s);
                GetMonData(mon, MON_DATA_NICKNAME, name);
                name[11] = 0xFFFF;
                Coop_StartGraveMessage(fsys, name, 1);
            }
            sys_FreeMemoryEz(mon);
            return TRUE;
        }
    }
    return FALSE; // no living Pokemon: Ghost Mode (Phase 5)
}

void Coop_ChallengeNuzlockeFrame(FieldSystem *fsys, struct CoopChallengeSave *save)
{
    struct Party *party = SaveArray_Party_Get(fsys->savedata);
    u8 *daycare = (u8 *)Save_Daycare_Get(fsys->savedata);
    u32 i;

    Coop_ChallengeFilterMarts();
    if (party->count > 0)
        save->state |= COOP_CHALLENGE_HAD_POKEMON;
    if (daycare != NULL)
    {
        u32 none = 0; // Day Care eggs are banned (Q10): no egg waits for the player
        __builtin_memcpy(daycare + COOP_DAYCARE_EGG_PID, &none, sizeof(none));
    }

    // A party Pokemon at 0 HP outside a battle is dead too (overworld poison
    // is off in this build; a safety net).
    for (i = 0; i < (u32)party->count; i++)
    {
        struct PartyPokemon *mon = Party_GetMonByIndex(party, i);
        if (GetMonData(mon, MON_DATA_SPECIES_EXISTS, NULL) && !GetMonData(mon, MON_DATA_IS_EGG, NULL)
            && GetMonData(mon, MON_DATA_HP, NULL) == 0)
        {
            u16 bits = GetMonData(mon, MON_DATA_RESERVED_114, NULL) | COOP_MON_DEAD;
            SetMonData(mon, MON_DATA_RESERVED_114, &bits);
        }
    }

    if (!Coop_IsSafeOverworldWindow(fsys))
        return;
    if (!(save->state & COOP_CHALLENGE_GRAVE_NAMED))
        Coop_NameGraveyard(fsys, save, COOP_GRAVEYARD_BOX, "GRAVEYARD");
    if (save->pending_flags & COOP_PENDING_OPEN)
        Coop_ChallengeResolvePending(fsys, save);
    if (Coop_BuryFromParty(fsys, save))
        return; // its message runs now
    if (sSweepBox < NUM_PC_BOXES)
    {
        Coop_SweepBox(fsys, save, sSweepBox++);
        return;
    }
    if (party->count == 0 && (save->state & COOP_CHALLENGE_HAD_POKEMON))
        Coop_WithdrawFirstLiving(fsys, save);
}

BOOL Coop_ChallengeHidesItem(u16 item)
{
    const struct CoopChallengeSave *save = Coop_ChallengeSave();

    if (save == NULL || !(save->rules & COOP_RULE_NUZLOCKE))
        return FALSE;
    return item == ITEM_REVIVE || item == ITEM_MAX_REVIVE || item == ITEM_REVIVAL_HERB || item == ITEM_SACRED_ASH;
}
