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

const struct CoopBattleExt gCoopBattleExt __attribute__((section(".coop_battle_ext"), used)) = {
    .magic = COOP_BATTLE_EXT_MAGIC,
    .linkWatchdog = Coop_LinkBattleLost,
};
