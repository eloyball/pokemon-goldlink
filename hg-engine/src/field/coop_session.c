#include "config.h"
#include "types.h"

#include "constants/buttons.h"

#include "map_events_internal.h"
#include "message.h"
#include "pokemon.h"
#include "sound.h"
#include "system.h"
#include "task.h"
#include "window.h"
#include "coop_sync.h"
#include "coop_dialog.h"
#include "coop_session.h"
#include "coop_choice.h"
#include "coop_battle.h"

// ---------------------------------------------------------------------------
// Player-to-player sessions (docs/MILESTONE3_PLAN.md, Phase 1), Overlay 131.
//
// Talking to an idle ghost opens Trade / Battle / Team up / Cancel (Leave team
// instead of Team up when it is my teammate). A choice becomes a request
// latched in gCoopCommBlock.session_out; the relay applies it once, asks the
// other player, and reports both sides' outcome in the gates (session_state,
// bumped session_seq). The other player gets a Yes / No prompt as soon as
// they are free. Accepted battles are placeholders until Phase 4; teams are
// stored by the relay in the campaign file. Leaving needs no answer: the
// relay drops the team at once and the ex-teammate reads a notice.
//
// Like the rest of Overlay 131, nothing lives in statics (no BSS init): the
// task env holds the flow, and gCoopCommBlock the latched seqs.
// ---------------------------------------------------------------------------

#define COOP_SE_SELECT 1500 // pret SEQ_SE_DP_SELECT

#define COOP_FRAMES_PER_SECOND   60
#define COOP_ANSWER_FAILSAFE     (25 * COOP_FRAMES_PER_SECOND) // the relay times out at 20 s
#define COOP_CANCEL_FAILSAFE     (3 * COOP_FRAMES_PER_SECOND)
#define COOP_RESULT_FAILSAFE     (5 * COOP_FRAMES_PER_SECOND)

enum CoopMenuValue // ghost menu rows (Yes / No: coop_choice.h)
{
    COOP_CHOICE_TRADE = COOP_SESSION_KIND_TRADE,
    COOP_CHOICE_BATTLE = COOP_SESSION_KIND_BATTLE,
    COOP_CHOICE_TEAM_UP = COOP_SESSION_KIND_TEAM_UP,
    COOP_CHOICE_LEAVE_TEAM = COOP_SESSION_KIND_LEAVE_TEAM,
    COOP_CHOICE_CANCEL,
};

enum CoopSessionTaskState
{
    COOP_SESSION_WAIT_STILL,
    COOP_SESSION_OPEN,
    COOP_SESSION_MENU_TEXT,      // "It's NAME! What would you like to do?"
    COOP_SESSION_MENU,
    COOP_SESSION_CONFIRM_TEXT,   // team up / leave team: "Team up with NAME?"
    COOP_SESSION_CONFIRM,
    COOP_SESSION_WAIT_ANSWER,    // my request (or my Yes) is with the relay
    COOP_SESSION_PROMPT_PAGE,    // incoming team-up: first page, then the question
    COOP_SESSION_PROMPT_TEXT,
    COOP_SESSION_PROMPT,
    COOP_SESSION_FINAL_TEXT,
    COOP_SESSION_FINAL_BUTTON,
};

struct CoopSessionTaskEnv
{
    struct CoopDialog dialog;
    struct CoopChoice choice;
    u16 name[9];       // the other player's name (HG-encoded), always 0xFFFF-terminated
    u32 peerTrainer;   // their trainer ID
    u16 frames;        // frames spent in the current wait
    u8 peerSlot;       // their remote slot
    u8 kind;           // enum CoopSessionKind
    u8 seenSeq;        // gates.session_seq when this side last acted
    u8 incoming;       // TRUE: answering someone else's request
    u8 cancelSent;
};

// ---------------------------------------------------------------------------
// Session requests (latched in the mailbox, applied once by the relay)
// ---------------------------------------------------------------------------

static void Coop_SessionSend(u32 op, u32 kind, u32 target)
{
    volatile struct CoopSessionOut *out = &gCoopCommBlock.session_out;

    out->op = (u8)op;
    out->kind = (u8)kind;
    out->target_trainer = target;
    out->seq = (u8)(out->seq + 1); // last: the bridge forwards the block as is
}

static BOOL Coop_IsSelfTeamed(void)
{
    return (gCoopCommBlock.gates.teamed_mask & COOP_TEAMED_SELF_BIT) != 0;
}

static BOOL Coop_IsSlotTeamed(u32 slot)
{
    return (gCoopCommBlock.gates.teamed_mask & (1u << slot)) != 0;
}

// Ends the task: closes everything, lets the world move again.
static BOOL Coop_SessionEnd(FieldSystem *fsys, struct CoopSessionTaskEnv *env)
{
    Coop_ChoiceClose(&env->choice);
    Coop_DialogClose(fsys, &env->dialog);
    MapObjectMan_UnpauseAllMovement(fsys->mapObjectMan);
    sys_FreeMemoryEz(env);
    return TRUE;
}

static void Coop_SessionFinal(TaskManager *taskman, struct CoopSessionTaskEnv *env, const char *msg)
{
    Coop_ChoiceClose(&env->choice);
    Coop_DialogPrint(&env->dialog, msg);
    taskman->state = COOP_SESSION_FINAL_TEXT;
}

static const char *Coop_AcceptedText(u32 kind)
{
    if (kind == COOP_SESSION_KIND_TEAM_UP)
        return "You and @ are now a team!";
    if (kind == COOP_SESSION_KIND_LEAVE_TEAM)
        return "You left your team with @.";
    return "Let's trade with @!";
}

// What either side reads once the request is over (state != OUTGOING/INCOMING).
static const char *Coop_ResultText(const struct CoopSessionTaskEnv *env, u32 state, u32 reason)
{
    switch (state)
    {
    case COOP_SESSION_ACCEPTED:
        return Coop_AcceptedText(env->kind);
    case COOP_SESSION_DECLINED:
        return env->incoming ? NULL : "@ said no.";
    case COOP_SESSION_TIMEOUT:
        return env->incoming ? "The request timed out." : "@ didn't answer.";
    case COOP_SESSION_CANCELLED:
        if (env->incoming)
            return "@ cancelled the request.";
        return env->cancelSent ? "You cancelled the request." : "@ isn't around anymore.";
    case COOP_SESSION_REFUSED:
        switch (reason)
        {
        case COOP_SESSION_REASON_NOT_HERE:       return "@ isn't around anymore.";
        case COOP_SESSION_REASON_BUSY:           return "@ is busy right now.\nTry again in a moment.";
        case COOP_SESSION_REASON_TEAM_RULE:      return "Teams can only battle\nother teams for now.";
        case COOP_SESSION_REASON_ALREADY_TEAMED: return "One of you is already\non a team.";
        case COOP_SESSION_REASON_INVALID:
            if (env->kind == COOP_SESSION_KIND_LEAVE_TEAM)
                return "You aren't on a team\nanymore.";
            return "That isn't possible\nright now.";
        default:                                 return "That isn't possible\nright now.";
        }
    default:
        return "That isn't possible\nright now.";
    }
}

// The request is over once the relay reports a final state for it.
static BOOL Coop_SessionIsOver(const struct CoopSessionTaskEnv *env, u32 *state, u32 *reason)
{
    const volatile struct CoopProgressionGates *gates = &gCoopCommBlock.gates;

    if (gates->session_seq == env->seenSeq)
        return FALSE;
    *state = gates->session_state;
    *reason = gates->session_reason;
    return *state != COOP_SESSION_OUTGOING && *state != COOP_SESSION_INCOMING && *state != COOP_SESSION_NOTICE
        && *state <= COOP_SESSION_STATE_MAX;
}

// Local team rules, checked before asking the relay (which checks again).
static const char *Coop_BattleRuleText(u32 slot)
{
    if (gCoopCommBlock.gates.team_partner_slot == slot)
        return "You can't battle your\nown teammate!";
    if (Coop_IsSelfTeamed() != Coop_IsSlotTeamed(slot))
        return "Teams can only battle\nother teams for now.";
    return NULL;
}

static void Coop_SessionRequest(TaskManager *taskman, struct CoopSessionTaskEnv *env, u32 kind)
{
    env->kind = (u8)kind;
    env->seenSeq = gCoopCommBlock.gates.session_seq;
    env->frames = 0;
    Coop_SessionSend(COOP_SESSION_OP_REQUEST, kind, env->peerTrainer);
    if (kind != COOP_SESSION_KIND_LEAVE_TEAM) // answered by the relay at once: nothing to cancel
        Coop_DialogPrint(&env->dialog, "Waiting for @...\nPress B to cancel.");
    taskman->state = COOP_SESSION_WAIT_ANSWER;
}

static void Coop_SessionOpenMainMenu(FieldSystem *fsys, struct CoopSessionTaskEnv *env)
{
    const char *labels[4];
    u8 values[4];
    u32 n = 0;

    labels[n] = "TRADE";
    values[n++] = COOP_CHOICE_TRADE;
    labels[n] = "BATTLE";
    values[n++] = COOP_CHOICE_BATTLE;
    // Team up while neither player is in a team; leave it from my teammate.
    if (!Coop_IsSelfTeamed() && !Coop_IsSlotTeamed(env->peerSlot))
    {
        labels[n] = "TEAM UP";
        values[n++] = COOP_CHOICE_TEAM_UP;
    }
    else if (gCoopCommBlock.gates.team_partner_slot == env->peerSlot)
    {
        labels[n] = "LEAVE TEAM";
        values[n++] = COOP_CHOICE_LEAVE_TEAM;
    }
    labels[n] = "CANCEL";
    values[n++] = COOP_CHOICE_CANCEL;
    Coop_ChoiceOpen(fsys, &env->choice, labels, values, n, 10);
}

// A pending incoming request was withdrawn, timed out, or the link dropped.
static BOOL Coop_PromptInterrupted(TaskManager *taskman, struct CoopSessionTaskEnv *env)
{
    const char *msg;

    if (!Coop_IsConnectedToRoom())
        msg = "The link to the others\nwas lost.";
    else if (gCoopCommBlock.gates.session_seq == env->seenSeq)
        return FALSE;
    else
        msg = Coop_ResultText(env, gCoopCommBlock.gates.session_state, gCoopCommBlock.gates.session_reason);
    Coop_SessionFinal(taskman, env, msg != NULL ? msg : "The request was cancelled.");
    return TRUE;
}

static BOOL Coop_Task_Session(TaskManager *taskman)
{
    FieldSystem *fsys = taskman->fieldSystem;
    struct CoopSessionTaskEnv *env = taskman->env;
    u32 value, state, reason;
    const char *msg;

    switch (taskman->state)
    {
    case COOP_SESSION_WAIT_STILL:
        if (MapObject_IsSingleMovementActive(fsys->playerAvatar->mapObject))
            break;
        taskman->state = COOP_SESSION_OPEN;
        // fallthrough
    case COOP_SESSION_OPEN:
        MapObjectMan_PauseAllMovement(fsys->mapObjectMan);
        Coop_DialogOpen(fsys, &env->dialog);
        env->dialog.args[0] = env->name;
        if (!env->incoming)
        {
            Coop_DialogPrint(&env->dialog, "It's @!\nWhat would you like to do?");
            taskman->state = COOP_SESSION_MENU_TEXT;
        }
        else if (env->kind == COOP_SESSION_KIND_LEAVE_TEAM)
        {
            Coop_DialogPrint(&env->dialog, "@ left your team.\nYou can team up again later.");
            taskman->state = COOP_SESSION_FINAL_TEXT;
        }
        else if (env->kind == COOP_SESSION_KIND_TEAM_UP)
        {
            Coop_DialogPrint(&env->dialog, "@ wants to team up!\nYou'd battle as a team.");
            taskman->state = COOP_SESSION_PROMPT_PAGE;
        }
        else
        {
            Coop_DialogPrint(&env->dialog, env->kind == COOP_SESSION_KIND_BATTLE ? "@ wants to battle!\nWill you accept?"
                                                                                : "@ wants to trade!\nWill you trade?");
            taskman->state = COOP_SESSION_PROMPT_TEXT;
        }
        break;

    // --- Asking -------------------------------------------------------------

    case COOP_SESSION_MENU_TEXT:
        if (!Coop_DialogIsPrinting(&env->dialog))
        {
            Coop_SessionOpenMainMenu(fsys, env);
            taskman->state = COOP_SESSION_MENU;
        }
        break;

    case COOP_SESSION_MENU:
        value = Coop_ChoiceInput(&env->choice);
        if (value == COOP_CHOICE_NONE)
            break;
        Coop_ChoiceClose(&env->choice);
        if (value == COOP_CHOICE_CANCEL)
            return Coop_SessionEnd(fsys, env);
        if (value == COOP_CHOICE_BATTLE && (msg = Coop_BattleRuleText(env->peerSlot)) != NULL)
        {
            Coop_SessionFinal(taskman, env, msg);
            break;
        }
        if (value == COOP_CHOICE_TEAM_UP)
        {
            env->kind = COOP_SESSION_KIND_TEAM_UP;
            Coop_DialogPrint(&env->dialog, "Team up with @?\nYou'd battle as a team.");
            taskman->state = COOP_SESSION_CONFIRM_TEXT;
            break;
        }
        if (value == COOP_CHOICE_LEAVE_TEAM)
        {
            env->kind = COOP_SESSION_KIND_LEAVE_TEAM;
            Coop_DialogPrint(&env->dialog, "Leave your team with @?");
            taskman->state = COOP_SESSION_CONFIRM_TEXT;
            break;
        }
        Coop_SessionRequest(taskman, env, value);
        break;

    case COOP_SESSION_CONFIRM_TEXT:
        if (!Coop_DialogIsPrinting(&env->dialog))
        {
            Coop_ChoiceOpenYesNo(fsys, &env->choice);
            taskman->state = COOP_SESSION_CONFIRM;
        }
        break;

    case COOP_SESSION_CONFIRM:
        value = Coop_ChoiceInput(&env->choice);
        if (value == COOP_CHOICE_NONE)
            break;
        Coop_ChoiceClose(&env->choice);
        if (value == COOP_CHOICE_NO)
            return Coop_SessionEnd(fsys, env);
        Coop_SessionRequest(taskman, env, env->kind);
        break;

    case COOP_SESSION_WAIT_ANSWER:
        env->frames++;
        if (Coop_SessionIsOver(env, &state, &reason))
        {
            // An accepted battle continues in this task slot (coop_battle.c).
            if (state == COOP_SESSION_ACCEPTED && env->kind == COOP_SESSION_KIND_BATTLE)
            {
                if (Coop_DialogIsPrinting(&env->dialog))
                    break;
                Coop_ChoiceClose(&env->choice);
                Coop_DialogClose(fsys, &env->dialog);
                if (Coop_BattleTakeOver(taskman, env->name, env->peerSlot, !env->incoming))
                {
                    sys_FreeMemoryEz(env);
                    break;
                }
                Coop_DialogOpen(fsys, &env->dialog);
                env->dialog.args[0] = env->name;
                Coop_SessionFinal(taskman, env, "The battle couldn't start.\nPlease try again.");
                break;
            }
            // An accepted trade continues in its own task (coop_trade.c),
            // which starts as soon as this one is gone.
            msg = Coop_ResultText(env, state, reason);
            if (msg == NULL || (state == COOP_SESSION_ACCEPTED && env->kind == COOP_SESSION_KIND_TRADE))
            {
                // Nothing more to say ("no" was mine, or a trade opens next);
                // the window can only go once its text is done.
                if (!Coop_DialogIsPrinting(&env->dialog))
                    return Coop_SessionEnd(fsys, env);
                break;
            }
            Coop_SessionFinal(taskman, env, msg);
            break;
        }
        if (!Coop_IsConnectedToRoom())
        {
            Coop_SessionFinal(taskman, env, "The link to the others\nwas lost.");
            break;
        }
        if (env->incoming || env->kind == COOP_SESSION_KIND_LEAVE_TEAM)
        {
            // My Yes (or my leaving) is with the relay; it answers within a few frames.
            if (env->frames >= COOP_RESULT_FAILSAFE)
                Coop_SessionFinal(taskman, env, "No answer from the link.\nPlease try again.");
            break;
        }
        if (!env->cancelSent && (gSystem.newKeys & PAD_BUTTON_B))
        {
            PlaySE(COOP_SE_SELECT);
            Coop_SessionSend(COOP_SESSION_OP_CANCEL, COOP_SESSION_KIND_NONE, 0);
            env->cancelSent = TRUE;
            env->frames = 0;
        }
        else if (env->cancelSent ? env->frames >= COOP_CANCEL_FAILSAFE : env->frames >= COOP_ANSWER_FAILSAFE)
        {
            // The relay never answered (packets lost, relay restarted).
            if (!env->cancelSent)
                Coop_SessionSend(COOP_SESSION_OP_CANCEL, COOP_SESSION_KIND_NONE, 0);
            Coop_SessionFinal(taskman, env, env->cancelSent ? "You cancelled the request." : "@ didn't answer.");
        }
        break;

    // --- Being asked --------------------------------------------------------

    case COOP_SESSION_PROMPT_PAGE:
        if (Coop_PromptInterrupted(taskman, env))
            break;
        if (!Coop_DialogIsPrinting(&env->dialog) && (gSystem.newKeys & (PAD_BUTTON_A | PAD_BUTTON_B)))
        {
            Coop_DialogPrint(&env->dialog, "Team up with @?");
            taskman->state = COOP_SESSION_PROMPT_TEXT;
        }
        break;

    case COOP_SESSION_PROMPT_TEXT:
        if (Coop_PromptInterrupted(taskman, env))
            break;
        if (!Coop_DialogIsPrinting(&env->dialog))
        {
            Coop_ChoiceOpenYesNo(fsys, &env->choice);
            taskman->state = COOP_SESSION_PROMPT;
        }
        break;

    case COOP_SESSION_PROMPT:
        if (Coop_PromptInterrupted(taskman, env))
            break;
        value = Coop_ChoiceInput(&env->choice);
        if (value == COOP_CHOICE_NONE)
            break;
        Coop_ChoiceClose(&env->choice);
        Coop_SessionSend(value == COOP_CHOICE_YES ? COOP_SESSION_OP_ACCEPT : COOP_SESSION_OP_DECLINE,
                         COOP_SESSION_KIND_NONE, 0);
        if (value == COOP_CHOICE_NO)
            return Coop_SessionEnd(fsys, env);
        env->frames = 0;
        taskman->state = COOP_SESSION_WAIT_ANSWER;
        break;

    // --- Last page ----------------------------------------------------------

    case COOP_SESSION_FINAL_TEXT:
        if (!Coop_DialogIsPrinting(&env->dialog))
            taskman->state = COOP_SESSION_FINAL_BUTTON;
        break;

    case COOP_SESSION_FINAL_BUTTON:
        if (gSystem.newKeys & (PAD_BUTTON_A | PAD_BUTTON_B))
            return Coop_SessionEnd(fsys, env);
        break;
    }
    return FALSE;
}

static BOOL Coop_StartSessionTask(FieldSystem *fsys, u32 remoteSlot, const struct CoopPlayerState *remote,
                                  BOOL incoming, u32 kind)
{
    struct CoopSessionTaskEnv *env;
    u32 i;

    if (!Coop_StartFieldTask(fsys, Coop_Task_Session, sizeof(struct CoopSessionTaskEnv)))
        return FALSE;
    env = ((TaskManager *)fsys->taskman)->env;
    for (i = 0; i < 8; i++)
        env->name[i] = remote->name[i];
    env->name[8] = 0xFFFF;
    env->peerTrainer = remote->trainer_id;
    env->peerSlot = (u8)remoteSlot;
    env->incoming = incoming ? TRUE : FALSE;
    env->kind = (u8)kind;
    env->seenSeq = gCoopCommBlock.gates.session_seq;
    return TRUE;
}

BOOL Coop_StartSessionMenu(FieldSystem *fsys, u32 remoteSlot, const struct CoopPlayerState *remote)
{
    return Coop_StartSessionTask(fsys, remoteSlot, remote, FALSE, COOP_SESSION_KIND_NONE);
}

void Coop_UpdateSessionPrompt(FieldSystem *fsys)
{
    const volatile struct CoopProgressionGates *gates = &gCoopCommBlock.gates;
    struct CoopPlayerState remote;
    u8 seq = gates->session_seq;
    u32 kind = gates->session_kind;
    u32 slot = gates->session_peer_slot;

    if (gates->session_state != COOP_SESSION_INCOMING && gates->session_state != COOP_SESSION_NOTICE)
    {
        // Forget the last prompt once it is over: after a reconnect the relay
        // counts session_seq from 1 again, and an old value must not hide a
        // new request. Changes never carry seq 0.
        gCoopCommBlock.track_session_answered = 0;
        return;
    }
    if (seq == gCoopCommBlock.track_session_answered)
        return;
    if (fsys->taskman != NULL || fsys->textbox_open || gCoopCommBlock.disconnect_state != COOP_DISCONNECT_NONE
        || !Coop_IsConnectedToRoom() || Coop_IsMapNameBannerShowing(fsys)
        || MapObject_IsSingleMovementActive(fsys->playerAvatar->mapObject))
        return; // asked again next frame; the relay expires the request after 20 s
    if (gates->session_state == COOP_SESSION_NOTICE ? kind != COOP_SESSION_KIND_LEAVE_TEAM
                                                    : (kind < COOP_SESSION_KIND_TRADE || kind > COOP_SESSION_KIND_TEAM_UP))
        kind = COOP_SESSION_KIND_NONE;
    if (kind == COOP_SESSION_KIND_NONE || !Coop_ReadRemotePlayer(slot, &remote))
    {
        gCoopCommBlock.track_session_answered = seq; // nobody to answer: ignore it
        return;
    }
    if (Coop_StartSessionTask(fsys, slot, &remote, TRUE, kind))
        gCoopCommBlock.track_session_answered = seq;
}
