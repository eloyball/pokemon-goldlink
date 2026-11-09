#include "config.h"
#include "types.h"

#include "pokemon.h"
#include "coop_sync.h"
#include "coop_link.h"
#include "system.h"
#include "battle.h"

// ---------------------------------------------------------------------------
// Link battle watchdog, battle side (docs/MILESTONE3_PLAN.md §17). In the
// battle extension (Overlay 130), which comes with the battle app: resident
// code (src/coop_link.c Coop_LinkBattleWatchdog) calls it through the header
// below every frame of a co-op link battle's app.
//
// A lost link is decided by the relay (the referee, docs/LINK_BATTLE_REWORK.md
// part 3): its BYE, or this game out of the room. Then the host ends the
// battle the way vanilla's BattleLink_RecvEnd does once its own game is done:
// the link tasks stop and the battle finishes; an undecided battle is a draw.
// The other games, which can't see their own commands, go back to the title.
// ---------------------------------------------------------------------------

#define COOP_BSTATE_BATTLE_MAIN      9 // ccd battle_022378C0.c BattleState

// ccd OverlayManager: the app's state, its data.
#define OVM_PROC_STATE 0x14
#define OVM_DATA       0x1C
// pret BattleSystem (include/battle/battle.h; hg-engine pads these): the link
// tasks' state pointers, battleFinished (ov12_0223BC2C), battleOutcomeFlag.
#define BSYS_LINK_SEND_STATE 0x23F4
#define BSYS_LINK_RECV_STATE 0x23F8
#define BSYS_BG_TILES        0x220 // bgTiles (then bgPalette, 0x224): allocated by the server's
                                   // SetBattleBackground command (ccd battle_system.c)
#define BSYS_IS_SERVER       0x23FC // isServer (ccd BattleSystem): the host runs the battle
#define BSYS_FINISHED        0x23FE
#define BSYS_OUTCOME         0x2420
#define BATTLE_LINK_STATE_END 0xFF
#define BATTLE_OUTCOME_DRAW   3

// A word of a vanilla struct hg-engine does not declare (no type-punning).
static u32 Coop_ReadWord(const u8 *p)
{
    u32 v;

    __builtin_memcpy(&v, p, 4);
    return v;
}

// The host's own game has done every command its server sent it (ServerQue[0]: an entry
// per command sent, cleared when that game reports it done; src/battle/battle_script_commands.c).
static BOOL Coop_HostGameIdle(const u8 *bsys)
{
    const struct BattleStruct *sp = ((const struct BattleSystem *)bsys)->sp;
    int battler, j;

    for (battler = 0; battler < 4; battler++)
        for (j = 0; j < 16; j++)
            if (sp->ServerQue[0][battler][j] != 0)
                return FALSE;
    return TRUE;
}

// ---------------------------------------------------------------------------
// The exchange before a link battle (docs/LINK_BATTLE_REWORK.md, phase 5).
//
// Vanilla's Battle_LinkMain (ccd battle_main.c) sends six pieces one at a
// time (AI flags, profile, trainer, party, Chatot, Pal Pad), each behind its
// own LinkSync and a wait for every game's copy: about seven round trips.
// The syncs protect two things: on a child, slot 0 of each setup array
// holds its own data until its "prepare" step copies it out, and a piece
// from the parent lands in slot 0; and every piece's receive callback adds
// to one shared counter, `received`, that each wait compares to the number
// of games. Here:
// - each game's own profile, trainer, party and Chatot are kept at the start
//   (before the first sync completes, nothing has arrived), and written into
//   the send buffer at each send, so a child never sends what landed in slot 0;
// - the receive callbacks of the six pieces are wrapped to count per piece
//   (the shared counter is left as it was);
// - so the pieces stream: the syncs of the AI flags, profile, trainer, party
//   and Chatot complete at once, their waits pass at once, and the last wait
//   (the Pal Pad's) passes only once every piece of every game is in. The
//   first sync and the Pal Pad's stay (the Pal Pad merges into the save's on
//   arrival, so it must not come before this game built its own).
// - the exchange's two 1800-frame timers (CommState_SetError, the error
//   screen) are held at 0: the relay decides when a game is gone (part 3).
// Anything unexpected (a NULL source, the exchange already past its first
// sync when first seen) leaves vanilla's flow alone.
// ---------------------------------------------------------------------------

#define COOP_BSTATE_LINK_MAIN 2 // ccd battle_022378C0.c BattleState

// ccd BattleLinkWork (include/battle_link_comm.h): setup, ..., buffer @0x20,
// received, state, timer.
#define BLW_BUFFER   0x20
#define BLW_RECEIVED 0x1020
#define BLW_STATE    0x1021
#define BLW_TIMER    0x1022

// ccd Battle_LinkMain link states.
#define LS_WAIT_SYNC       3
#define LS_SEND_AI_FLAGS   4
#define LS_WAIT_AI_FLAGS   5
#define LS_SEND_PAL_PAD    19
#define LS_WAIT_PAL_PAD    20

// hg-engine struct BattleSetup: party[4] @0x04, trainer[4] @0x28 (0x34 each),
// profile[4] @0xF8, chatot[4] @0x118.
#define SETUP_PARTY   0x04
#define SETUP_TRAINER 0x28
#define SETUP_PROFILE 0xF8
#define SETUP_CHATOT  0x118

#define COOP_PROFILE_SIZE 0x20  // ccd PlayerProfile
#define COOP_TRAINER_SIZE 0x34  // TRAINER_DATA
#define COOP_PARTY_SIZE   0x5B0 // ccd Party: PartyCore (8 + 6 * 0xEC) and PartyExtra (6 * 5), padded
#define COOP_CHATOT_SIZE  0x3EC // ccd SOUND_CHATOT

_Static_assert(sizeof(TRAINER_DATA) == COOP_TRAINER_SIZE, "TRAINER_DATA size");

// gBattleLinkCommands (ccd battle_link_comm.c): 12 entries of { receive,
// size, receive buffer }, commands 22-33. The six pieces' entries.
#define COOP_EXCHANGE_PIECES 6
static const u8 sPieceEntry[COOP_EXCHANGE_PIECES] = { 2, 3, 4, 5, 6, 11 }; // AI flags .. Chatot, Pal Pad
// The LinkSync id each piece's send waits for, completed at once (0: kept).
static const u8 sPieceSync[COOP_EXCHANGE_PIECES] = { 0x33, 0x34, 0x35, 0x36, 0x37, 0 };

struct CoopRegisteredTable // ccd CommCommandTable (head)
{
    u32 *commands;
};
extern struct CoopRegisteredTable *sCommCommandTable; // 0x021D412C (rom.ld)

void LONG_CALL LinkSync_RecvDone(int sender, int size, void *data, void *context);
int LONG_CALL Comm_CountConnected(void);

typedef void (*CoopRecvFunc)(int sender, int size, void *data, void *context);

struct CoopExchange
{
    u8 armed; // 0 not yet; 1 this exchange runs the co-op way; 2 done, or given up (vanilla's)
    u8 count[COOP_EXCHANGE_PIECES];
    u32 *table;
    CoopRecvFunc orig[COOP_EXCHANGE_PIECES];
};

struct CoopExchangeData // written at the start, before any read
{
    u8 profile[COOP_PROFILE_SIZE];
    u8 trainer[COOP_TRAINER_SIZE];
    u8 party[COOP_PARTY_SIZE];
    u8 chatot[COOP_CHATOT_SIZE];
};

// Overlay 130 is loaded again with every battle: its .data comes from the ROM
// each time, so `armed` starts at 0 (in .data, not .bss, which nothing clears).
static struct CoopExchange sExchange __attribute__((section(".data"))) = { 0 };
static struct CoopExchangeData sExchangeData;

static void Coop_ExchangeRecv(int piece, int sender, int size, void *data, void *context)
{
    u8 *work = context;
    u8 received = work[BLW_RECEIVED];

    sExchange.orig[piece](sender, size, data, context);
    work[BLW_RECEIVED] = received; // the wait reads this piece's own count
    sExchange.count[piece]++;
}

static void Coop_RecvAiFlags(int s, int z, void *d, void *c) { Coop_ExchangeRecv(0, s, z, d, c); }
static void Coop_RecvProfile(int s, int z, void *d, void *c) { Coop_ExchangeRecv(1, s, z, d, c); }
static void Coop_RecvTrainer(int s, int z, void *d, void *c) { Coop_ExchangeRecv(2, s, z, d, c); }
static void Coop_RecvParty(int s, int z, void *d, void *c) { Coop_ExchangeRecv(3, s, z, d, c); }
static void Coop_RecvChatot(int s, int z, void *d, void *c) { Coop_ExchangeRecv(4, s, z, d, c); }
static void Coop_RecvPalPad(int s, int z, void *d, void *c) { Coop_ExchangeRecv(5, s, z, d, c); }

static const CoopRecvFunc sPieceRecv[COOP_EXCHANGE_PIECES] = {
    Coop_RecvAiFlags, Coop_RecvProfile, Coop_RecvTrainer, Coop_RecvParty, Coop_RecvChatot, Coop_RecvPalPad,
};

static void Coop_ExchangeRestore(void)
{
    int i;

    for (i = 0; i < COOP_EXCHANGE_PIECES; i++)
        sExchange.table[sPieceEntry[i] * 3] = (u32)sExchange.orig[i];
}

// Called every frame of the battle app (after its frame) while it exchanges.
static void Coop_LinkExchange(u8 *app)
{
    u8 *work, *setup, *src[4];
    u32 *table;
    int i, piece, games;
    u8 state;

    if (Coop_ReadWord(app + OVM_PROC_STATE) != COOP_BSTATE_LINK_MAIN)
    {
        if (sExchange.armed == 1)
        {
            Coop_ExchangeRestore(); // the battle's own commands follow; 130 leaves with the app
            sExchange.armed = 2;
        }
        return;
    }
    work = (u8 *)Coop_ReadWord(app + OVM_DATA);
    if (work == NULL)
        return;
    __builtin_memcpy(&work[BLW_TIMER], &(u16){ 0 }, 2); // the relay decides, not a timer
    state = work[BLW_STATE];

    if (sExchange.armed == 0)
    {
        // Only while nothing can have arrived yet (no piece is sent before the first sync).
        sExchange.armed = 2;
        table = sCommCommandTable != NULL ? sCommCommandTable->commands : NULL;
        setup = (u8 *)Coop_ReadWord(work);
        if (state > LS_WAIT_SYNC || table == NULL || setup == NULL)
            return;
        src[0] = (u8 *)Coop_ReadWord(setup + SETUP_PROFILE);
        src[1] = setup + SETUP_TRAINER;
        src[2] = (u8 *)Coop_ReadWord(setup + SETUP_PARTY);
        src[3] = (u8 *)Coop_ReadWord(setup + SETUP_CHATOT);
        if (src[0] == NULL || src[2] == NULL || src[3] == NULL)
            return;
        __builtin_memcpy(sExchangeData.profile, src[0], COOP_PROFILE_SIZE);
        __builtin_memcpy(sExchangeData.trainer, src[1], COOP_TRAINER_SIZE);
        __builtin_memcpy(sExchangeData.party, src[2], COOP_PARTY_SIZE);
        __builtin_memcpy(sExchangeData.chatot, src[3], COOP_CHATOT_SIZE);
        sExchange.table = table;
        for (i = 0; i < COOP_EXCHANGE_PIECES; i++)
        {
            sExchange.orig[i] = (CoopRecvFunc)table[sPieceEntry[i] * 3];
            sExchange.count[i] = 0;
            table[sPieceEntry[i] * 3] = (u32)sPieceRecv[i];
        }
        sExchange.armed = 1;
        return;
    }
    if (sExchange.armed != 1 || state < LS_SEND_AI_FLAGS || state > LS_WAIT_PAL_PAD)
        return;
    piece = (state - LS_SEND_AI_FLAGS) / 3;
    switch ((state - LS_SEND_AI_FLAGS) % 3)
    {
    case 0: // a send (the AI flags' and the Pal Pad's need no copy): my own data, and no sync wait
        if (piece == 1)
            __builtin_memcpy(work + BLW_BUFFER, sExchangeData.profile, COOP_PROFILE_SIZE);
        else if (piece == 2)
            __builtin_memcpy(work + BLW_BUFFER, sExchangeData.trainer, COOP_TRAINER_SIZE);
        else if (piece == 3)
            __builtin_memcpy(work + BLW_BUFFER, sExchangeData.party, COOP_PARTY_SIZE);
        else if (piece == 4)
            __builtin_memcpy(work + BLW_BUFFER, sExchangeData.chatot, COOP_CHATOT_SIZE);
        if (sPieceSync[piece] != 0)
            LinkSync_RecvDone(0, 1, (void *)&sPieceSync[piece], NULL);
        break;
    case 1: // a wait: none but the last, which waits for every piece of every game
        games = Comm_CountConnected();
        if (piece < COOP_EXCHANGE_PIECES - 1)
        {
            work[BLW_RECEIVED] = (u8)games;
            break;
        }
        for (i = 0; i < COOP_EXCHANGE_PIECES; i++)
            if (sExchange.count[i] < games)
                break;
        work[BLW_RECEIVED] = i == COOP_EXCHANGE_PIECES ? (u8)games : 0;
        break;
    }
}

static void Coop_LinkBattleLost(u8 *app)
{
    u8 *bsys, *send, *recv;
    u32 state;

    if (!Coop_LinkBattleIsLost())
    {
        gCoopCommBlock.track_link_lost_frames = 0;
        return;
    }
    if (gCoopCommBlock.track_link_lost_frames < COOP_LINK_BATTLE_LOST_FRAMES)
    {
        gCoopCommBlock.track_link_lost_frames++;
        return;
    }
    // The battle app before its main phase (the exchange of parties,
    // profiles and LinkSync) or in it; after it, the link screen needs no link.
    state = Coop_ReadWord(app + OVM_PROC_STATE);
    if (state > COOP_BSTATE_BATTLE_MAIN)
        return;
    bsys = (u8 *)Coop_ReadWord(app + OVM_DATA);
    // Lost before the battle could end cleanly: in the exchange before it
    // (docs/KNOWN_RISKS.md #30), or in its intro, before the server's
    // SetBattleBackground command came (the battle's exit would free its NULL
    // background buffers and crash both games: black screens, #52). Back to
    // the title screen instead, as a disconnect on the field does: every
    // co-op battle saved just before it started, so nothing is lost.
    // (Vanilla's error screen cannot be used: its reset waits for the radio
    // to stop, which ours only reports from the co-op SysTask.)
    if (state != COOP_BSTATE_BATTLE_MAIN || Coop_ReadWord(bsys + BSYS_BG_TILES) == 0) // bgPalette follows it
        OS_ResetSystem(0);
    if (bsys[BSYS_FINISHED])
        return;
    // The battle's exit frees its menus and sprites under any task still running a command:
    // a player choosing a move when the link was lost crashed the game (a data abort in
    // BattleInput_CheckTouch, link rework phase 3). The host sees its own game's commands and
    // ends once they are done (a choice in progress ends with the player's choice, which goes
    // nowhere). The other games can't tell theirs: they go back to the title as above.
    if (!bsys[BSYS_IS_SERVER])
        OS_ResetSystem(0);
    if (!Coop_HostGameIdle(bsys))
        return;
    send = (u8 *)Coop_ReadWord(bsys + BSYS_LINK_SEND_STATE);
    recv = (u8 *)Coop_ReadWord(bsys + BSYS_LINK_RECV_STATE);
    if (send != NULL)
        *send = BATTLE_LINK_STATE_END;
    if (recv != NULL)
        *recv = BATTLE_LINK_STATE_END;
    if (bsys[BSYS_OUTCOME] == 0)
        bsys[BSYS_OUTCOME] = BATTLE_OUTCOME_DRAW;
    bsys[BSYS_FINISHED] = 1;
    gCoopCommBlock.track_link_battle = COOP_LINK_BATTLE_LOST;
}

static void Coop_LinkBattleFrame(u8 *app)
{
    Coop_LinkExchange(app);
    Coop_LinkBattleLost(app);
}

const struct CoopBattleExt gCoopBattleExt __attribute__((section(".coop_battle_ext"), used)) = {
    .magic = COOP_BATTLE_EXT_MAGIC,
    .linkWatchdog = Coop_LinkBattleFrame,
};
