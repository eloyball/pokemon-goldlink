#include "config.h"
#include "types.h"

#include "constants/buttons.h"
#include "constants/item.h"
#include "constants/moves.h"
#include "constants/species.h"

#include "map_events_internal.h"
#include "message.h"
#include "pokemon.h"
#include "system.h"
#include "task.h"
#include "window.h"
#include "coop_sync.h"
#include "coop_dialog.h"
#include "coop_choice.h"
#include "coop_session.h"
#include "coop_scene.h"

// ---------------------------------------------------------------------------
// Trading (docs/MILESTONE3_PLAN.md, Phase 2), Overlay 131.
//
// Starts whenever the relay holds a trade for this player
// (gates.trade_pending) and the player is free in the field: right after
// both accepted a trade request, or after a reconnect when a committed trade
// still has to be applied here. The task owns a CoopTradeExchange (on the
// field heap, published in gCoopCommBlock.exchange_ptr) that the bridge
// mirrors to and from the relay.
//
//   pick a Pokemon -> offer (encrypted, as in the party) -> the partner's
//   offer arrives, validated -> Yes -> both confirmed: the relay journals the
//   trade and commits -> swap, Pokedex -> trade scene and evolution scene
//   (src/coop_scene.c) -> save -> DONE
//
// The relay only commits once both sides confirmed each other's current
// offer, and keeps the commit (with both Pokemon) on disk until both games
// reported DONE after saving. A game that crashed before saving gets the
// same commit again after reconnecting: the Pokemon to give is found by its
// personality value, and one already swapped is recognized by the received
// personality, so applying is idempotent.
// ---------------------------------------------------------------------------

struct Party *LONG_CALL SaveArray_Party_Get(void *saveData);
BOOL LONG_CALL Field_SaveGameNormal(FieldSystem *fsys);
void LONG_CALL PokeCopyPPtoPP(struct PartyPokemon *src, struct PartyPokemon *dst);

extern u16 gEvolutionSceneOverride[2][2]; // src/pokemon.c: form of the evolution GetMonEvolution found

#define COOP_NICK_CHARS           12 // 10 + EOS, plus a guard terminator
#define COOP_FPS                  60
#define COOP_SYNC_FAILSAFE        (5 * COOP_FPS)
#define COOP_CANCEL_FAILSAFE      (3 * COOP_FPS)
#define COOP_DONE_FAILSAFE        (5 * COOP_FPS)

enum CoopTradeTaskState
{
    COOP_TRADE_WAIT_STILL,
    COOP_TRADE_OPEN,
    COOP_TRADE_SYNC,          // wait for the relay's view of this trade
    COOP_TRADE_PICK_TEXT,     // "Which Pokémon will you trade?"
    COOP_TRADE_PICK_SCREEN,   // back from the party screen (overworld left meanwhile)
    COOP_TRADE_PICK,          // the in-field list, when the party screen cannot open
    COOP_TRADE_RULE_TEXT,     // why that Pokemon cannot be traded; back to the list
    COOP_TRADE_RULE_BUTTON,
    COOP_TRADE_WAIT_OFFER,    // my offer is out; waiting for the partner's
    COOP_TRADE_SHOW_OFFER,    // "NAME offers MON (Lv. N)."
    COOP_TRADE_CONFIRM_TEXT,  // "Trade your MON for MON?"
    COOP_TRADE_CONFIRM,
    COOP_TRADE_WAIT_COMMIT,   // I said yes; waiting for the partner
    COOP_TRADE_CANCELLING,    // CANCEL sent; waiting for the relay to drop the trade
    COOP_TRADE_APPLY_TEXT,    // after a reconnect: "Finishing your last trade..."
    COOP_TRADE_SCENE,         // the trade and evolution scenes (overworld left)
    COOP_TRADE_SCENE_DONE,    // the overworld is back
    COOP_TRADE_EVOLVE_TEXT,   // without the scenes (no memory): "What? Y is evolving!"
    COOP_TRADE_EVOLVED_TEXT,  // "Congratulations! ..."
    COOP_TRADE_SAVE_TEXT,
    COOP_TRADE_SAVE,
    COOP_TRADE_SAVE_FAILED,
    COOP_TRADE_WAIT_DONE,     // DONE sent; waiting for the relay to close the trade
    COOP_TRADE_FINAL_TEXT,
    COOP_TRADE_FINAL_BUTTON,
};

struct CoopTradeEnv
{
    struct CoopTradeExchange ex; // mirrored by the bridge while published
    struct PartyPokemon incoming; // validated private copy (0xF0 >= COOP_PARTY_MON_SIZE)
    struct PartyPokemon outgoing; // the Pokemon given, for the trade scene
    struct CoopDialog dialog;
    struct CoopChoice choice;
    u16 peerName[9];
    u16 myNick[COOP_NICK_CHARS];
    u16 theirNick[COOP_NICK_CHARS];
    u16 levelText[4];
    u16 evoName[COOP_NICK_CHARS];
    u32 tradeId;
    u16 frames;
    u8 slot;           // my offered party slot
    u8 picked;         // the party screen's result (COOP_PARTY_SELECT_CANCEL: B)
    u8 cursor;         // where the party screen's cursor starts
    u8 listOnly;       // the party screen could not open: use the in-field list
    u8 partnerOfferId; // the partner offer shown and validated
    u8 evolved;
    u8 recovery;       // applying a commit this task never confirmed (after a reconnect)
    u8 swapped;        // the received Pokemon entered the party in this task
};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

static void Coop_NumberToGlyphs(u32 value, u16 *out)
{
    u16 digits[4];
    u32 n = 0, i;

    do
    {
        digits[n++] = (u16)(0x0121 + value % 10); // charmap '0'..'9'
        value /= 10;
    } while (value != 0 && n < 3);
    for (i = 0; i < n; i++)
        out[i] = digits[n - 1 - i];
    out[n] = 0xFFFF;
}

static void Coop_GetNickname(struct PartyPokemon *mon, u16 *out)
{
    GetMonData(mon, MON_DATA_NICKNAME, out);
    out[COOP_NICK_CHARS - 1] = 0xFFFF;
}

static BOOL Coop_IsFieldFree(FieldSystem *fsys)
{
    return fsys->taskman == NULL && !fsys->textbox_open && gCoopCommBlock.disconnect_state == COOP_DISCONNECT_NONE
        && !Coop_IsMapNameBannerShowing(fsys) && !MapObject_IsSingleMovementActive(fsys->playerAvatar->mapObject);
}

// Everything a received Pokemon must pass before it can enter the party: it
// decrypts (checksum), is a real species at a real level, is not an egg, and
// carries no mail. The partner's game checks the same before offering; this
// is the defense against a corrupted or tampered transfer.
static BOOL Coop_TradeMonIsValid(struct PartyPokemon *mon)
{
    u32 species, level, i;

    if (GetMonData(mon, MON_DATA_CHECKSUM_FAILED, NULL) || GetMonData(mon, MON_DATA_IS_EGG, NULL)
        || GetMonData(mon, MON_DATA_SANITY_IS_EGG, NULL))
        return FALSE;
    species = GetMonData(mon, MON_DATA_SPECIES, NULL);
    level = GetMonData(mon, MON_DATA_LEVEL, NULL);
    if (species == SPECIES_NONE || species > MAX_SPECIES_INCLUDING_FORMS || level == 0 || level > 100)
        return FALSE;
    if (IS_ITEM_MAIL(GetMonData(mon, MON_DATA_HELD_ITEM, NULL)))
        return FALSE;
    for (i = 0; i < 4; i++)
    {
        if (GetMonData(mon, MON_DATA_MOVE1 + i, NULL) >= NUM_OF_MOVES)
            return FALSE;
    }
    return GetMonData(mon, MON_DATA_HP, NULL) <= GetMonData(mon, MON_DATA_MAXHP, NULL);
}

// Loads the encrypted bytes into the env's aligned copy and validates them.
static BOOL Coop_TradeLoadIncoming(struct CoopTradeEnv *env, const u8 *bytes)
{
    __builtin_memset(&env->incoming, 0, sizeof(env->incoming));
    __builtin_memcpy(&env->incoming, bytes, COOP_PARTY_MON_SIZE);
    if (!Coop_TradeMonIsValid(&env->incoming))
        return FALSE;
    Coop_GetNickname(&env->incoming, env->theirNick);
    Coop_NumberToGlyphs(GetMonData(&env->incoming, MON_DATA_LEVEL, NULL), env->levelText);
    return TRUE;
}

// Why a party Pokemon cannot be offered, or NULL. The lead may be traded
// even while riding or surfing: the trade scene leaves and restores the
// overworld, which rebuilds the follower from the party.
static const char *Coop_TradePickRule(struct Party *party, u32 slot)
{
    struct PartyPokemon *mon = Party_GetMonByIndex(party, slot);
    u32 i, healthy = 0;

    if (GetMonData(mon, MON_DATA_IS_EGG, NULL))
        return "Eggs can't be traded.";
    if (IS_ITEM_MAIL(GetMonData(mon, MON_DATA_HELD_ITEM, NULL)))
        return "It's holding Mail. Take\nthe Mail first.";
    for (i = 0; i < (u32)party->count; i++)
    {
        struct PartyPokemon *other = Party_GetMonByIndex(party, i);
        if (i != slot && !GetMonData(other, MON_DATA_IS_EGG, NULL) && GetMonData(other, MON_DATA_HP, NULL) != 0)
            healthy++;
    }
    if (healthy == 0)
        return "You'd have no Pokémon left\nthat can battle!";
    return NULL;
}

// ---------------------------------------------------------------------------
// Party list
// ---------------------------------------------------------------------------

#define COOP_TRADE_LIST_CANCEL 0x80
#define COOP_TRADE_LIST_WIDTH  15 // tiles: nickname + "Lv. 100"
#define COOP_TRADE_LEVEL_X     80 // pixels

static void Coop_TradeOpenPartyList(FieldSystem *fsys, struct CoopTradeEnv *env, struct Party *party)
{
    u8 values[COOP_CHOICE_MAX];
    u16 nick[COOP_NICK_CHARS];
    u16 level[4];
    const u16 *args[2];
    u32 count = (u32)party->count, i;

    if (count > COOP_CHOICE_MAX - 1)
        count = COOP_CHOICE_MAX - 1;
    for (i = 0; i < count; i++)
        values[i] = (u8)(i + 1); // 0 means "still choosing"
    values[count] = COOP_TRADE_LIST_CANCEL;
    Coop_ChoiceOpenWindow(fsys, &env->choice, values, count + 1, COOP_TRADE_LIST_WIDTH);
    args[0] = nick;
    args[1] = level;
    for (i = 0; i < count; i++)
    {
        struct PartyPokemon *mon = Party_GetMonByIndex(party, i);
        Coop_GetNickname(mon, nick);
        Coop_ChoicePrint(&env->choice, i, COOP_CHOICE_TEXT_X, "@", args);
        if (!GetMonData(mon, MON_DATA_IS_EGG, NULL))
        {
            Coop_NumberToGlyphs(GetMonData(mon, MON_DATA_LEVEL, NULL), level);
            Coop_ChoicePrint(&env->choice, i, COOP_TRADE_LEVEL_X, "Lv. $", args);
        }
    }
    Coop_ChoicePrint(&env->choice, count, COOP_CHOICE_TEXT_X, "CANCEL", NULL);
    Coop_ChoiceShow(&env->choice);
}

// ---------------------------------------------------------------------------
// Task
// ---------------------------------------------------------------------------

static BOOL Coop_TradeEnd(FieldSystem *fsys, struct CoopTradeEnv *env)
{
    gCoopCommBlock.exchange_ptr = 0; // before the free: the bridge stops mirroring
    Coop_ChoiceClose(&env->choice);
    Coop_DialogClose(fsys, &env->dialog);
    MapObjectMan_UnpauseAllMovement(fsys->mapObjectMan);
    sys_FreeMemoryEz(env);
    return TRUE;
}

static void Coop_TradeFinal(TaskManager *taskman, struct CoopTradeEnv *env, const char *msg)
{
    Coop_ChoiceClose(&env->choice);
    Coop_DialogPrint(&env->dialog, msg);
    taskman->state = COOP_TRADE_FINAL_TEXT;
}

static void Coop_TradeCancel(TaskManager *taskman, struct CoopTradeEnv *env)
{
    Coop_ChoiceClose(&env->choice);
    env->ex.out.state = COOP_TRADE_OUT_CANCEL;
    env->frames = 0;
    Coop_DialogPrint(&env->dialog, "Cancelling the trade...");
    taskman->state = COOP_TRADE_CANCELLING;
}

static const char *Coop_TradeAbortText(u32 reason)
{
    switch (reason)
    {
    case COOP_TRADE_ABORT_CANCELLED: return "@ cancelled the trade.";
    case COOP_TRADE_ABORT_LEFT:      return "@ isn't around anymore.\nThe trade was cancelled.";
    case COOP_TRADE_ABORT_TIMEOUT:   return "The trade took too long\nand was cancelled.";
    default:                         return "The trade was cancelled.";
    }
}

// Swaps the received Pokemon in and sets the state the task continues with:
// the trade scene, after a page when finishing a trade from before a
// reconnect.
static void Coop_TradeApply(FieldSystem *fsys, TaskManager *taskman, struct CoopTradeEnv *env)
{
    struct Party *party = SaveArray_Party_Get(fsys->savedata);
    const struct CoopTradeIn *in = &env->ex.in; // written by the bridge between frames only
    u32 givePid = in->give_pid;
    u32 receivePid, i, slot = 0xFF, alreadyHere = 0xFF;

    env->tradeId = in->trade_id;
    Coop_ChoiceClose(&env->choice);
    if (!Coop_TradeLoadIncoming(env, in->mon))
    {
        // Cannot happen for an offer this game validated; a damaged journal
        // after a crash is reported as done so it stops coming back.
        env->ex.out.trade_id = env->tradeId;
        env->ex.out.state = COOP_TRADE_OUT_DONE;
        env->frames = 0;
        Coop_DialogPrint(&env->dialog, "The last trade couldn't\nbe finished.");
        taskman->state = COOP_TRADE_WAIT_DONE;
        return;
    }
    receivePid = GetMonData(&env->incoming, MON_DATA_PERSONALITY, NULL);
    for (i = 0; i < (u32)party->count; i++)
    {
        u32 pid = GetMonData(Party_GetMonByIndex(party, i), MON_DATA_PERSONALITY, NULL);
        if (pid == givePid && slot == 0xFF)
            slot = i;
        if (pid == receivePid)
            alreadyHere = i;
    }
    if (slot == 0xFF)
    {
        // Already swapped and saved before a crash or a lost packet (or the
        // party no longer matches): nothing to change, just close it.
        env->ex.out.trade_id = env->tradeId;
        env->ex.out.state = COOP_TRADE_OUT_DONE;
        env->frames = 0;
        Coop_DialogPrint(&env->dialog, alreadyHere != 0xFF ? "Your last trade is complete." : "The last trade couldn't\nbe finished.");
        taskman->state = COOP_TRADE_WAIT_DONE;
        return;
    }

    env->slot = (u8)slot;
    Coop_GetNickname(Party_GetMonByIndex(party, slot), env->myNick);
    PokeCopyPPtoPP(Party_GetMonByIndex(party, slot), &env->outgoing);
    PokeCopyPPtoPP(&env->incoming, Party_GetMonByIndex(party, slot));
    env->swapped = TRUE;
    UpdatePokedexWithReceivedSpecies(fsys->savedata, Party_GetMonByIndex(party, slot));
    if (env->recovery)
    {
        Coop_DialogPrint(&env->dialog, "Finishing your last trade\nwith @...");
        taskman->state = COOP_TRADE_APPLY_TEXT;
    }
    else
        taskman->state = COOP_TRADE_SCENE;
}

// Trade evolution without the evolution scene, when the scenes could not
// start (no memory): species and form, stats, the consumed item, the name,
// the Pokedex. No moves are learned.
static BOOL Coop_TradeEvolve(FieldSystem *fsys, struct CoopTradeEnv *env)
{
    struct Party *party = SaveArray_Party_Get(fsys->savedata);
    struct PartyPokemon *mon = Party_GetMonByIndex(party, env->slot);
    int method = 0;
    u16 target, none = 0;

    target = GetMonEvolution(party, mon, EVOCTX_TRADE, 0, &method);
    if (target == SPECIES_NONE || target > MAX_SPECIES_INCLUDING_FORMS)
        return FALSE;
    if (method == EVO_TRADE_ITEM)
        SetMonData(mon, MON_DATA_HELD_ITEM, &none);
    if (gEvolutionSceneOverride[1][0] == target)
    {
        u32 form = gEvolutionSceneOverride[1][1];
        SetMonData(mon, MON_DATA_SPECIES, &target);
        SetMonData(mon, MON_DATA_FORM, &form);
    }
    else
        SetMonData(mon, MON_DATA_SPECIES, &target);
    if (!GetMonData(mon, MON_DATA_HAS_NICKNAME, NULL))
        SetMonData(mon, MON_DATA_SPECIES_NAME, NULL); // the nickname follows the species
    RecalcPartyPokemonStats(mon);
    UpdatePokedexWithReceivedSpecies(fsys->savedata, mon);
    GetMonData(mon, MON_DATA_SPECIES_NAME, env->evoName);
    env->evoName[COOP_NICK_CHARS - 1] = 0xFFFF;
    return TRUE;
}

// Before the commit: the link, an abort, or the relay forgetting the trade
// end it; a commit (even after I pressed Cancel) must be applied.
static BOOL Coop_TradeCheckInterrupt(FieldSystem *fsys, TaskManager *taskman, struct CoopTradeEnv *env)
{
    u32 state = env->ex.in.state;

    if (state == COOP_TRADE_IN_COMMIT)
    {
        Coop_TradeApply(fsys, taskman, env);
        return TRUE;
    }
    if (!Coop_IsConnectedToRoom())
    {
        env->ex.out.state = COOP_TRADE_OUT_CANCEL;
        Coop_TradeFinal(taskman, env, "The link to the others\nwas lost. Trade cancelled.");
        return TRUE;
    }
    if (state == COOP_TRADE_IN_ABORTED)
    {
        env->ex.out.state = COOP_TRADE_OUT_CANCEL; // acknowledges the notice
        Coop_TradeFinal(taskman, env, Coop_TradeAbortText(env->ex.in.reason));
        return TRUE;
    }
    if (state == COOP_TRADE_IN_NONE && !gCoopCommBlock.gates.trade_pending)
    {
        Coop_TradeFinal(taskman, env, "The trade was cancelled.");
        return TRUE;
    }
    return FALSE;
}

static void Coop_TradeLoadPeerName(struct CoopTradeEnv *env)
{
    struct CoopPlayerState remote;
    u32 i;

    if (Coop_ReadRemotePlayer(env->ex.in.peer_slot, &remote))
    {
        for (i = 0; i < 8; i++)
            env->peerName[i] = remote.name[i];
    }
    else
    {
        static const u16 someone[] = { 0x013E, 0x0149, 0x0145, 0x0151, 0x0151, 0x0145, 0x0158, 0x0149, 0xFFFF }; // "Teammate"
        for (i = 0; i < 9; i++)
            env->peerName[i] = someone[i];
    }
    env->peerName[8] = 0xFFFF;
}

static void Coop_TradeOpenDialog(FieldSystem *fsys, struct CoopTradeEnv *env)
{
    MapObjectMan_PauseAllMovement(fsys->mapObjectMan);
    Coop_DialogOpen(fsys, &env->dialog);
    env->dialog.args[0] = env->peerName;
    env->dialog.args[1] = env->theirNick;
    env->dialog.args[2] = env->levelText;
    env->dialog.args[3] = env->myNick;
}

// A Pokemon was picked: why it cannot be traded, or offer it (the encrypted
// bytes as the party holds them) and wait for the partner's.
static void Coop_TradePicked(TaskManager *taskman, struct CoopTradeEnv *env, struct Party *party, u32 slot)
{
    const char *msg;

    env->slot = (u8)slot;
    env->cursor = (u8)slot;
    if (party->count < 2)
        msg = "You need at least two\nPokémon to trade.";
    else
        msg = Coop_TradePickRule(party, slot);
    if (msg != NULL)
    {
        Coop_DialogPrint(&env->dialog, msg);
        taskman->state = COOP_TRADE_RULE_TEXT;
        return;
    }
    __builtin_memcpy(env->ex.out.mon, Party_GetMonByIndex(party, slot), COOP_PARTY_MON_SIZE);
    Coop_GetNickname(Party_GetMonByIndex(party, slot), env->myNick);
    env->ex.out.offer_id = (u8)(env->ex.out.offer_id + 1);
    env->ex.out.state = COOP_TRADE_OUT_OFFERED;
    Coop_DialogPrint(&env->dialog, "Waiting for @...\nPress B to cancel.");
    taskman->state = COOP_TRADE_WAIT_OFFER;
}

static BOOL Coop_Task_Trade(TaskManager *taskman)
{
    FieldSystem *fsys = taskman->fieldSystem;
    struct CoopTradeEnv *env = taskman->env;
    struct Party *party = SaveArray_Party_Get(fsys->savedata);
    u32 value;

    switch (taskman->state)
    {
    case COOP_TRADE_WAIT_STILL:
        if (MapObject_IsSingleMovementActive(fsys->playerAvatar->mapObject))
            break;
        taskman->state = COOP_TRADE_OPEN;
        // fallthrough
    case COOP_TRADE_OPEN:
        Coop_TradeOpenDialog(fsys, env);
        Coop_DialogPrint(&env->dialog, "Please wait...");
        env->frames = 0;
        taskman->state = COOP_TRADE_SYNC;
        break;

    case COOP_TRADE_SYNC:
        // The bridge fills `in` within a frame or two of the publication.
        env->frames++;
        Coop_TradeLoadPeerName(env);
        if (env->ex.in.state == COOP_TRADE_IN_COMMIT)
        {
            env->recovery = TRUE;
            Coop_TradeApply(fsys, taskman, env);
            break;
        }
        if (env->ex.in.state == COOP_TRADE_IN_WAITING || env->ex.in.state == COOP_TRADE_IN_OFFERED)
        {
            env->ex.out.state = COOP_TRADE_OUT_PICKING;
            Coop_DialogPrint(&env->dialog, "Which Pokémon will you\ntrade with @?");
            taskman->state = COOP_TRADE_PICK_TEXT;
            break;
        }
        if (env->ex.in.state == COOP_TRADE_IN_ABORTED || !Coop_IsConnectedToRoom())
        {
            (void)Coop_TradeCheckInterrupt(fsys, taskman, env);
            break;
        }
        if ((env->frames >= COOP_SYNC_FAILSAFE || (env->frames > COOP_FPS && !gCoopCommBlock.gates.trade_pending))
            && !Coop_DialogIsPrinting(&env->dialog))
            return Coop_TradeEnd(fsys, env); // nothing to do after all
        break;

    // --- Choosing -----------------------------------------------------------

    case COOP_TRADE_PICK_TEXT:
        if (Coop_TradeCheckInterrupt(fsys, taskman, env))
            break;
        if (Coop_DialogIsPrinting(&env->dialog))
            break;
        if (!env->listOnly)
        {
            // The party screen, as for vanilla's NPC trades. Its window and
            // ours belong to the field map, which it unloads.
            Coop_ChoiceClose(&env->choice);
            Coop_DialogClose(fsys, &env->dialog);
            if (Coop_CallTradePartySelect(taskman, env->cursor, &env->picked))
            {
                taskman->state = COOP_TRADE_PICK_SCREEN;
                break;
            }
            env->listOnly = TRUE;
            Coop_TradeOpenDialog(fsys, env);
            Coop_DialogPrint(&env->dialog, "Which Pokémon will you\ntrade with @?");
            break;
        }
        Coop_TradeOpenPartyList(fsys, env, party);
        taskman->state = COOP_TRADE_PICK;
        break;

    case COOP_TRADE_PICK_SCREEN:
        // The field is back. A cancel, abort or lost link while the screen was
        // up is handled first.
        Coop_TradeOpenDialog(fsys, env);
        if (Coop_TradeCheckInterrupt(fsys, taskman, env))
            break;
        if (env->picked >= (u32)party->count)
            Coop_TradeCancel(taskman, env);
        else
            Coop_TradePicked(taskman, env, party, env->picked);
        break;

    case COOP_TRADE_PICK:
        if (Coop_TradeCheckInterrupt(fsys, taskman, env))
            break;
        value = Coop_ChoiceInput(&env->choice);
        if (value == COOP_CHOICE_NONE)
            break;
        if (value == COOP_TRADE_LIST_CANCEL)
        {
            Coop_TradeCancel(taskman, env);
            break;
        }
        Coop_ChoiceClose(&env->choice);
        Coop_TradePicked(taskman, env, party, value - 1);
        break;

    case COOP_TRADE_RULE_TEXT:
        if (Coop_TradeCheckInterrupt(fsys, taskman, env))
            break;
        if (!Coop_DialogIsPrinting(&env->dialog))
            taskman->state = COOP_TRADE_RULE_BUTTON;
        break;

    case COOP_TRADE_RULE_BUTTON:
        if (Coop_TradeCheckInterrupt(fsys, taskman, env))
            break;
        if (gSystem.newKeys & (PAD_BUTTON_A | PAD_BUTTON_B))
        {
            Coop_DialogPrint(&env->dialog, "Which Pokémon will you\ntrade with @?");
            taskman->state = COOP_TRADE_PICK_TEXT;
        }
        break;

    // --- The partner's offer ------------------------------------------------

    case COOP_TRADE_WAIT_OFFER:
        if (Coop_TradeCheckInterrupt(fsys, taskman, env))
            break;
        if (gSystem.newKeys & PAD_BUTTON_B)
        {
            Coop_TradeCancel(taskman, env);
            break;
        }
        if (env->ex.in.state != COOP_TRADE_IN_OFFERED || env->ex.in.partner_offer_id == 0)
            break;
        env->partnerOfferId = env->ex.in.partner_offer_id;
        if (!Coop_TradeLoadIncoming(env, env->ex.in.mon))
        {
            env->ex.out.state = COOP_TRADE_OUT_CANCEL;
            Coop_TradeFinal(taskman, env, "Something is wrong with that\nPokémon. Trade cancelled.");
            break;
        }
        Coop_DialogPrint(&env->dialog, "@ offers\n$ (Lv. &).");
        taskman->state = COOP_TRADE_SHOW_OFFER;
        break;

    case COOP_TRADE_SHOW_OFFER:
        if (Coop_TradeCheckInterrupt(fsys, taskman, env))
            break;
        if (!Coop_DialogIsPrinting(&env->dialog) && (gSystem.newKeys & (PAD_BUTTON_A | PAD_BUTTON_B)))
        {
            Coop_DialogPrint(&env->dialog, "Trade your ^\nfor $?");
            taskman->state = COOP_TRADE_CONFIRM_TEXT;
        }
        break;

    case COOP_TRADE_CONFIRM_TEXT:
        if (Coop_TradeCheckInterrupt(fsys, taskman, env))
            break;
        if (!Coop_DialogIsPrinting(&env->dialog))
        {
            Coop_ChoiceOpenYesNo(fsys, &env->choice);
            taskman->state = COOP_TRADE_CONFIRM;
        }
        break;

    case COOP_TRADE_CONFIRM:
        if (Coop_TradeCheckInterrupt(fsys, taskman, env))
            break;
        value = Coop_ChoiceInput(&env->choice);
        if (value == COOP_CHOICE_NONE)
            break;
        if (value == COOP_CHOICE_NO)
        {
            Coop_TradeCancel(taskman, env);
            break;
        }
        Coop_ChoiceClose(&env->choice);
        env->ex.out.confirm_id = env->partnerOfferId;
        env->ex.out.state = COOP_TRADE_OUT_CONFIRMED;
        Coop_DialogPrint(&env->dialog, "Waiting for @...\nPress B to cancel.");
        taskman->state = COOP_TRADE_WAIT_COMMIT;
        break;

    case COOP_TRADE_WAIT_COMMIT:
        if (Coop_TradeCheckInterrupt(fsys, taskman, env))
            break;
        if (gSystem.newKeys & PAD_BUTTON_B)
            Coop_TradeCancel(taskman, env);
        break;

    case COOP_TRADE_CANCELLING:
        // A commit may still win the race: then the trade happens.
        if (env->ex.in.state == COOP_TRADE_IN_COMMIT)
        {
            Coop_TradeApply(fsys, taskman, env);
            break;
        }
        if (++env->frames >= COOP_CANCEL_FAILSAFE || env->ex.in.state == COOP_TRADE_IN_NONE
            || env->ex.in.state == COOP_TRADE_IN_ABORTED || !Coop_IsConnectedToRoom())
            Coop_TradeFinal(taskman, env, "The trade was cancelled.");
        break;

    // --- Committed: no way back ---------------------------------------------

    case COOP_TRADE_APPLY_TEXT:
        if (Coop_DialogIsPrinting(&env->dialog) || !(gSystem.newKeys & (PAD_BUTTON_A | PAD_BUTTON_B)))
            break;
        taskman->state = COOP_TRADE_SCENE;
        // fallthrough
    case COOP_TRADE_SCENE:
        // The windows belong to the field map, which the scene unloads.
        if (Coop_DialogIsPrinting(&env->dialog))
            break;
        Coop_ChoiceClose(&env->choice);
        Coop_DialogClose(fsys, &env->dialog);
        if (Coop_CallTradeScene(taskman, &env->outgoing, Party_GetMonByIndex(party, env->slot), env->peerName))
        {
            taskman->state = COOP_TRADE_SCENE_DONE;
            break;
        }
        Coop_TradeOpenDialog(fsys, env);
        if (Coop_TradeEvolve(fsys, env))
        {
            Coop_DialogPrint(&env->dialog, "What? $ is evolving!");
            taskman->state = COOP_TRADE_EVOLVE_TEXT;
        }
        else
        {
            Coop_DialogPrint(&env->dialog, "Saving. Don't turn off\nthe power.");
            taskman->state = COOP_TRADE_SAVE_TEXT;
        }
        break;

    case COOP_TRADE_SCENE_DONE:
        Coop_TradeOpenDialog(fsys, env);
        Coop_DialogPrint(&env->dialog, "Saving. Don't turn off\nthe power.");
        taskman->state = COOP_TRADE_SAVE_TEXT;
        break;

    case COOP_TRADE_EVOLVE_TEXT:
        if (Coop_DialogIsPrinting(&env->dialog) || !(gSystem.newKeys & (PAD_BUTTON_A | PAD_BUTTON_B)))
            break;
        env->evolved = TRUE;
        env->dialog.args[2] = env->evoName;
        Coop_DialogPrint(&env->dialog, "Congratulations! Your $\nevolved into &!");
        taskman->state = COOP_TRADE_EVOLVED_TEXT;
        break;

    case COOP_TRADE_EVOLVED_TEXT:
        if (Coop_DialogIsPrinting(&env->dialog) || !(gSystem.newKeys & (PAD_BUTTON_A | PAD_BUTTON_B)))
            break;
        Coop_DialogPrint(&env->dialog, "Saving. Don't turn off\nthe power.");
        taskman->state = COOP_TRADE_SAVE_TEXT;
        break;

    case COOP_TRADE_SAVE_TEXT:
        if (Coop_DialogIsPrinting(&env->dialog))
            break;
        env->frames = 0;
        taskman->state = COOP_TRADE_SAVE;
        break;

    case COOP_TRADE_SAVE:
        if (++env->frames < 2)
            break; // let the message reach the screen first
        if (!Field_SaveGameNormal(fsys))
        {
            Coop_DialogPrint(&env->dialog, "The save failed. Press A\nto try again.");
            taskman->state = COOP_TRADE_SAVE_FAILED;
            break;
        }
        // Saved: only now may the relay forget this side of the trade.
        env->ex.out.trade_id = env->tradeId;
        env->ex.out.state = COOP_TRADE_OUT_DONE;
        env->frames = 0;
        taskman->state = COOP_TRADE_WAIT_DONE;
        break;

    case COOP_TRADE_SAVE_FAILED:
        if (!Coop_DialogIsPrinting(&env->dialog) && (gSystem.newKeys & PAD_BUTTON_A))
        {
            Coop_DialogPrint(&env->dialog, "Saving. Don't turn off\nthe power.");
            taskman->state = COOP_TRADE_SAVE_TEXT;
        }
        break;

    case COOP_TRADE_WAIT_DONE:
        // Without the relay's acknowledgment the commit comes back after the
        // next reconnect, and is recognized as already applied.
        if (Coop_DialogIsPrinting(&env->dialog))
            break;
        if (env->ex.in.state == COOP_TRADE_IN_COMMIT && ++env->frames < COOP_DONE_FAILSAFE && Coop_IsConnectedToRoom())
            break;
        if (!env->swapped)
        {
            // Nothing was swapped here (already done, or not possible): the
            // message on screen says so.
            taskman->state = COOP_TRADE_FINAL_BUTTON;
            break;
        }
        Coop_TradeFinal(taskman, env, "Your trade with @\nis complete!");
        break;

    // --- Last page ----------------------------------------------------------

    case COOP_TRADE_FINAL_TEXT:
        if (!Coop_DialogIsPrinting(&env->dialog))
            taskman->state = COOP_TRADE_FINAL_BUTTON;
        break;

    case COOP_TRADE_FINAL_BUTTON:
        if (gSystem.newKeys & (PAD_BUTTON_A | PAD_BUTTON_B))
            return Coop_TradeEnd(fsys, env);
        break;
    }
    return FALSE;
}

static BOOL Coop_StartTradeTask(FieldSystem *fsys)
{
    struct CoopTradeEnv *env;

    if (!Coop_StartFieldTask(fsys, Coop_Task_Trade, sizeof(struct CoopTradeEnv)))
        return FALSE;
    env = ((TaskManager *)fsys->taskman)->env;
    env->ex.magic = COOP_EXCHANGE_MAGIC;
    env->ex.version = COOP_EXCHANGE_VERSION;
    env->ex.size = sizeof(struct CoopTradeExchange);
    env->ex.out.state = COOP_TRADE_OUT_NONE;
    env->ex.in.peer_slot = COOP_SLOT_NONE;
    env->slot = 0xFF;
    env->peerName[0] = 0xFFFF;
    env->myNick[0] = 0xFFFF;
    env->theirNick[0] = 0xFFFF;
    env->levelText[0] = 0xFFFF;
    env->evoName[0] = 0xFFFF;
    gCoopCommBlock.exchange_ptr = (u32)&env->ex;
    return TRUE;
}

void Coop_UpdateTrade(FieldSystem *fsys)
{
    if (!Coop_IsFieldFree(fsys))
        return;
    if (gCoopCommBlock.gates.trade_pending == 1 && Coop_IsConnectedToRoom())
        (void)Coop_StartTradeTask(fsys);
}
