#ifndef COOP_CHALLENGE_H
#define COOP_CHALLENGE_H

#include "types.h"

// ---------------------------------------------------------------------------
// Milestone 5 challenge runs (docs/MILESTONE5_PLAN.md).
//
// The host picks the room's rules in the bridge; the relay fixes them when
// the roster locks and sends them, with this player's Monolocke type and the
// shared ledgers, in a trailer the bridge writes into CoopChallengeSync below.
// The ROM latches the rules into its own save at the adventure start: from
// then on the save is authoritative.
// ---------------------------------------------------------------------------

// Rule bits: must match coop-relay/src/protocol.rs CHALLENGE_RULE_* and the
// bridge's ChallengeRules.
#define COOP_RULE_RANDOM_MASK      0x0003 // 0 none, 1 semi-random, 2 preserve difficulty, 3 fully random
#define COOP_RULE_LEGENDS_WILD     0x0004
#define COOP_RULE_RANDOM_ABILITIES 0x0008
#define COOP_RULE_NUZLOCKE         0x0010
#define COOP_RULE_SOUL_LINK_SHIFT  5      // 0 off, 1 room-wide, 2 duo
#define COOP_RULE_SOUL_LINK_MASK   0x0060
#define COOP_RULE_MONOLOCKE        0x0080
#define COOP_RULE_LEVEL_CAP        0x0100
#define COOP_RULES_VALID           0x01FF

#define COOP_RANDOM_NONE     0
#define COOP_RANDOM_SEMI     1
#define COOP_RANDOM_PRESERVE 2
#define COOP_RANDOM_FULL     3

#define COOP_SOUL_LINK_OFF       0
#define COOP_SOUL_LINK_ROOM_WIDE 1
#define COOP_SOUL_LINK_DUO       2

// Monolocke types are hg-engine's 0..17 (type 9 is Fairy); 18 is "none",
// the relay's value for players without one.
#define COOP_MONO_TYPE_COUNT 18
#define COOP_MONO_TYPE_NONE  18

// One bit per place (docs/MILESTONE5_PLAN.md §4.3): 0-108 HGSS map section
// - 126, 109-120 the Safari areas, 121 the Bug-Catching Contest, 122 the
// starters (Milestone 6 §3.5: Soul-Link links every player's starter).
#define COOP_LOCATION_BYTES   16
#define COOP_MAPSEC_FIRST     126
#define COOP_LOCATION_SAFARI  109
#define COOP_LOCATION_CONTEST 121
#define COOP_LOCATION_STARTER 122
#define COOP_SAFARI_AREAS     12

// CoopChallengeSave.state bits (also the up trailer's state).
#define COOP_CHALLENGE_LATCHED     0x01
#define COOP_CHALLENGE_SPIRIT      0x02
#define COOP_CHALLENGE_RUN_LOST    0x04
#define COOP_CHALLENGE_HAD_POKEMON 0x08
#define COOP_CHALLENGE_GRAVE_NAMED 0x10 // box 30 renamed GRAVEYARD
#define COOP_CHALLENGE_GRAVE2      0x20 // box 29 is the second Graveyard box (Q11)
#define COOP_CHALLENGE_ANCHOR_WARP 0x40 // Phase 5: a new spirit still has to reach its anchor
#define COOP_CHALLENGE_RETURN_WARP 0x80 // Phase 5: revived, still has to return to return_*

// CoopChallengeSave.spirit_flags (Phase 5).
#define COOP_SPIRIT_INTRO 0x01 // the spirit task's first page (and save) is still to come

// CoopChallengeSave.pending_flags: the wild Pokemon of the battle in progress
// (docs/MILESTONE5_PLAN.md §5.4.2). Resolved on the first quiet field frame after it.
#define COOP_PENDING_FIRST          0x01 // this location's first encounter: catchable
#define COOP_PENDING_EXEMPT         0x02 // a static (no location): catchable
#define COOP_PENDING_PARTNER_FAILED 0x04 // Soul-Link: the group already lost this place (Phase 6)
#define COOP_PENDING_OPEN           0x80 // a battle to resolve

// pret FLAG_WATCHED_CATCHING_TUTORIAL: set on Route 29 right after the first
// five Poke Balls. Nuzlocke first encounters count only from then on.
#define COOP_FLAG_CATCHING_TUTORIAL 0x9A

// Elm's lab, where the starter is given (Milestone 6 §3.2): pret
// MAP_NEW_BARK_ELMS_LAB_1F, FLAG_GOT_STARTER (set right after ChooseStarter)
// and VAR_SCENE_ELMS_LAB (still 0 until the end of that script).
#define COOP_MAP_ELMS_LAB        61
#define COOP_FLAG_GOT_STARTER    0x6A
#define COOP_VAR_SCENE_ELMS_LAB  0x4108

// CoopChallengeSave.flags (Milestone 6).
#define COOP_FLAG_STARTER_DONE   0x0001 // the starter got its rules once (starter_pid valid)
#define COOP_FLAG_STARTER_NAMED  0x0002 // Nuzlocke: Elm's "Give a nickname?" was answered Yes once

// PC boxes (0-based): the Graveyard is box 30, then box 29 (Q11).
#define COOP_GRAVEYARD_BOX  29
#define COOP_GRAVEYARD_BOX2 28

// gates.challenge_room_state bits.
#define COOP_ROOM_RUN_LOST 0x01

// Player state 0x3F (challenge_flags) bits.
#define COOP_PLAYER_FLAG_SPIRIT 0x01

// ---------------------------------------------------------------------------
// The save block (docs/MILESTONE5_PLAN.md §4.3)
//
// The general save has no room to grow (Phase 0: 47 of 47 pages; growing it
// would stop every existing save from loading). The block lives in the
// unused tail of the Pokewalker save array, which needs infrared hardware the
// emulator does not have: normal play only writes unlockedCourses (0x130),
// and the new-game setup randomizes 0xFC..0x123 once, before any block exists.
// Older saves read zero there, which is "no rules".
// ---------------------------------------------------------------------------

#define COOP_SAVE_POKEWALKER       39    // pret SAVE_POKEWALKER
#define COOP_POKEWALKER_SIZE       0x134 // pret sizeof(POKEWALKER)
#define COOP_CHALLENGE_SAVE_OFFSET 0x8C  // after the walker's Pokemon (0x04..0x8B)
#define COOP_CHALLENGE_SAVE_END    0x130 // unlockedCourses
#define COOP_CHALLENGE_SAVE_MAGIC  0x3543 // "C5"
#define COOP_CHALLENGE_SAVE_VERSION 1

struct CoopChallengeSave
{
    u16 magic;                         // 0x00 COOP_CHALLENGE_SAVE_MAGIC; anything else = no rules
    u8  version;                       // 0x02
    u8  state;                         // 0x03 COOP_CHALLENGE_*
    u16 rules;                         // 0x04 COOP_RULE_*
    u8  mono_type;                     // 0x06 0..17, COOP_MONO_TYPE_NONE
    u8  spirit_flags;                  // 0x07 COOP_SPIRIT_* (Phase 5)
    u32 seed;                          // 0x08 the room's room_uid at the adventure start
    u16 starters[3];                   // 0x0C species | form << 11 (Phase 2); 0 = vanilla
    u8  spent[COOP_LOCATION_BYTES];    // 0x12 first encounter used
    u8  failed[COOP_LOCATION_BYTES];   // 0x22 my linked locations that failed or died
    u16 flags;                         // 0x32 COOP_FLAG_* (Milestone 6; 0 in older saves)
    u32 safari_pid[COOP_SAFARI_AREAS]; // 0x34 the first catch of each Safari area
    u32 contest_pid;                   // 0x64 the Bug Contest's first catch
    u32 pending_pid;                   // 0x68 the wild Pokemon of a first encounter in progress
    u8  pending_loc;                   // 0x6C its location, 0xFF none
    u8  pending_flags;                 // 0x6D
    u16 deaths;                        // 0x6E
    u16 anchor_map;                    // 0x70 where the spirit fell
    s16 anchor_x;
    s16 anchor_z;
    u16 return_map;                    // 0x76 the blackout spot to return to on revival
    s16 return_x;
    s16 return_z;
    u16 prebattle_map;                 // 0x7C the last field tile before the current battle (0: none)
    s16 prebattle_x;
    s16 prebattle_z;
    u16 pad82;
    u32 starter_pid;                   // 0x84 Milestone 6: valid with COOP_FLAG_STARTER_DONE
};                                     // 0x88

_Static_assert(__builtin_offsetof(struct CoopChallengeSave, seed) == 0x08, "CoopChallengeSave.seed");
_Static_assert(__builtin_offsetof(struct CoopChallengeSave, spent) == 0x12, "CoopChallengeSave.spent");
_Static_assert(__builtin_offsetof(struct CoopChallengeSave, safari_pid) == 0x34, "CoopChallengeSave.safari_pid");
_Static_assert(__builtin_offsetof(struct CoopChallengeSave, pending_pid) == 0x68, "CoopChallengeSave.pending_pid");
_Static_assert(__builtin_offsetof(struct CoopChallengeSave, prebattle_z) == 0x80, "CoopChallengeSave.prebattle_z");
_Static_assert(__builtin_offsetof(struct CoopChallengeSave, starter_pid) == 0x84, "CoopChallengeSave.starter_pid");
_Static_assert(sizeof(struct CoopChallengeSave) == 0x88, "CoopChallengeSave size");
_Static_assert(COOP_CHALLENGE_SAVE_OFFSET + sizeof(struct CoopChallengeSave) <= COOP_CHALLENGE_SAVE_END,
               "CoopChallengeSave must end before the Pokewalker's unlockedCourses");

// ---------------------------------------------------------------------------
// The sync block (docs/MILESTONE5_PLAN.md §4.4), in Overlay 131's .data and
// published through its pinned header (CoopFieldEntryHeader.challenge). It
// exists only while the field runs: the ROM refills `out` from the save every
// field frame, and the bridge rewrites `in` from every server packet.
// ---------------------------------------------------------------------------

#define COOP_CHALLENGE_SYNC_MAGIC   0x31534843 // "CHS1"
#define COOP_CHALLENGE_SYNC_VERSION 1
#define COOP_CHALLENGE_WIRE_MAGIC   0x314C4843 // "CHL1" (the trailers)
#define COOP_CHALLENGE_WIRE_VERSION 1
#define COOP_CHALLENGE_UP_SIZE      48
#define COOP_CHALLENGE_DOWN_SIZE    64
#define COOP_LIVING_UNKNOWN         0xFF // not counted (the up trailer's living)

// The up trailer as the ROM fills it (the bridge sets magic, version and the
// checksum itself): coop-relay protocol.rs ChallengeUp.
struct CoopChallengeUp
{
    u32 magic;                      // 0x00
    u8  version;                    // 0x04
    u8  state;                      // 0x05 COOP_CHALLENGE_*
    u8  living;                     // 0x06
    u8  pad07;
    u16 rules;                      // 0x08 what the save latched
    u16 deaths;                     // 0x0A
    u8  spent[COOP_LOCATION_BYTES]; // 0x0C
    u8  failed[COOP_LOCATION_BYTES]; // 0x1C
    u16 checksum;                   // 0x2C
    u16 pad2E;
};

// The relay's down trailer, written verbatim by the bridge: protocol.rs
// ChallengeDown. The ROM checks the magic and checksum itself.
struct CoopChallengeDown
{
    u32 magic;                          // 0x00
    u8  version;                        // 0x04
    u8  mono_type;                      // 0x05
    u8  room_state;                     // 0x06 COOP_ROOM_*
    u8  pad07;
    u16 rules;                          // 0x08
    u16 team_deaths;                    // 0x0A every roster trainer's deaths (the "Run lost" summary)
    u8  link_union[COOP_LOCATION_BYTES]; // 0x0C
    u8  spent[COOP_LOCATION_BYTES];     // 0x1C the relay's copy of mine
    u8  failed[COOP_LOCATION_BYTES];    // 0x2C the relay's copy of mine
    u16 checksum;                       // 0x3C
    u16 pad3E;
};

struct CoopChallengeSync
{
    u32 magic;   // 0x00 COOP_CHALLENGE_SYNC_MAGIC
    u16 version; // 0x04
    u16 size;    // 0x06 sizeof(struct CoopChallengeSync)
    struct CoopChallengeUp out;  // 0x08
    struct CoopChallengeDown in; // 0x38
};

_Static_assert(sizeof(struct CoopChallengeUp) == COOP_CHALLENGE_UP_SIZE, "CoopChallengeUp size");
_Static_assert(__builtin_offsetof(struct CoopChallengeUp, failed) == 0x1C, "CoopChallengeUp.failed");
_Static_assert(__builtin_offsetof(struct CoopChallengeUp, checksum) == 0x2C, "CoopChallengeUp.checksum");
_Static_assert(sizeof(struct CoopChallengeDown) == COOP_CHALLENGE_DOWN_SIZE, "CoopChallengeDown size");
_Static_assert(__builtin_offsetof(struct CoopChallengeDown, link_union) == 0x0C, "CoopChallengeDown.link_union");
_Static_assert(__builtin_offsetof(struct CoopChallengeDown, failed) == 0x2C, "CoopChallengeDown.failed");
_Static_assert(__builtin_offsetof(struct CoopChallengeDown, checksum) == 0x3C, "CoopChallengeDown.checksum");
_Static_assert(__builtin_offsetof(struct CoopChallengeSync, out) == 0x08, "CoopChallengeSync.out");
_Static_assert(__builtin_offsetof(struct CoopChallengeSync, in) == 0x38, "CoopChallengeSync.in");
_Static_assert(sizeof(struct CoopChallengeSync) == 0x78, "CoopChallengeSync size (bridge Mailbox.ChallengeSyncSize)");

// ---------------------------------------------------------------------------
// The randomizer pool (docs/MILESTONE5_PLAN.md §4.5), generated by
// scripts/challenge_pool.py into src/field/coop_challenge_pool.c (Overlay 131).
// ---------------------------------------------------------------------------

#define COOP_POOL_PICK     0x01 // may replace a wild Pokemon
#define COOP_POOL_BASE     0x02 // nothing evolves into it (starters, gifts)
#define COOP_POOL_LEGEND_A 0x04 // sub-legendaries, Ultra Beasts, the draft's paradoxes
#define COOP_POOL_LEGEND_B 0x08 // box legends and mythicals
#define COOP_POOL_LEGENDS  (COOP_POOL_LEGEND_A | COOP_POOL_LEGEND_B)

struct CoopPoolEntry
{
    u16 value;  // species | form << 11 (the encounter tables' encoding)
    u16 family; // index into the family list
    u8  type1;
    u8  type2;
    u8  flags;  // COOP_POOL_*
    u8  tier;   // BST tier 1-4: < 340, < 450, < 540, the rest
};

extern const struct CoopPoolEntry gCoopPool[];
extern const u16 gCoopPoolCount;
extern const u16 gCoopPoolFamilyCount;
extern const u16 gCoopAbilityPool[];
extern const u16 gCoopAbilityPoolCount;

// MON_DATA_RESERVED_114 bits Milestone 5 owns (hg-engine uses 0x0001-0x0800;
// docs/MILESTONE5_PLAN.md §4.3).
#define COOP_MON_ABILITY_LOCKED 0x1000 // a random ability: SetBoxMonAbility keeps it
#define COOP_MON_DEAD           0x2000
#define COOP_MON_LINKED         0x4000
#define COOP_MON_WILD_CATCH     0x8000

// hg-engine's SaveArray_Get (src/save.c, Overlay 129); no header declares it.
void *LONG_CALL SaveArray_Get(void *saveData, int id);
void *LONG_CALL SaveBlock2_get(void);

// The challenge block of the loaded save, or NULL. Inline so the battle
// extension and Overlay 131 each have it (neither can call the other).
static inline struct CoopChallengeSave *Coop_ChallengeSaveOf(void *saveData)
{
    u8 *walker = saveData != NULL ? SaveArray_Get(saveData, COOP_SAVE_POKEWALKER) : NULL;
    struct CoopChallengeSave *save;

    if (walker == NULL)
        return NULL;
    save = (struct CoopChallengeSave *)(walker + COOP_CHALLENGE_SAVE_OFFSET);
    return save->magic == COOP_CHALLENGE_SAVE_MAGIC && save->version == COOP_CHALLENGE_SAVE_VERSION ? save : NULL;
}

// ---------------------------------------------------------------------------
// Overlay 131 (src/field/coop_challenge.c)
// ---------------------------------------------------------------------------

struct FieldSystem;

extern struct CoopChallengeSync gCoopChallengeSync;

// This save's challenge block, or NULL when it has none (a save from before
// Milestone 5, or a room without rules that never latched).
struct CoopChallengeSave *Coop_ChallengeSave(void);

// The latched rules (0 when the save has none).
u16 Coop_ChallengeRules(void);

// The relay's trailer as the bridge last wrote it, if its magic, version and
// checksum check out (NULL otherwise).
const struct CoopChallengeDown *Coop_ChallengeIn(void);

// Once per field frame: latch the rules, take back the relay's copy of the
// ledgers, and publish this player's report for the bridge.
void Coop_ChallengeFieldFrame(struct FieldSystem *fsys);

// The adventure-start message's rule pages (docs/MILESTONE5_PLAN.md §1):
// writes page `index` (ASCII, at most two lines) and returns TRUE, or FALSE
// past the last page. Latches the rules first, so the save that follows
// carries them.
BOOL Coop_ChallengeStartPage(u32 index, char *buf, u32 size);

// ---------------------------------------------------------------------------
// Phase 2: random encounters (src/field/coop_challenge_random.c, Overlay 131)
// ---------------------------------------------------------------------------

struct PartyPokemon;
struct BATTLE_PARAM;

// The wild Pokemon about to join the battle (hg-engine's AddWildPartyPokemon,
// before it applies forms, items and abilities): replaced by the room's
// randomizer when its rules say so, and given a random ability.
void Coop_ChallengeWildMon(struct PartyPokemon *pp, struct BATTLE_PARAM *bp);

// Elm's three choices for a new save's challenge block (0 = vanilla). Fully
// random mixes in this player's trainer ID: an offer of their own (M6 §3.2).
void Coop_ChallengeStarters(struct CoopChallengeSave *save, u32 trainerId);

// Milestone 6 §3.2, §3.5: once, on the field frames right after Elm gives the
// starter: its random ability and its Soul-Link (location 122).
void Coop_ChallengeStarterFrame(struct FieldSystem *fsys, struct CoopChallengeSave *save);

// Milestone 6 §3.2: GetMenuChoice's callback (pret sub_020477C0, `hooks`):
// under Nuzlocke, Elm's "Give a nickname?" always answers Yes.
struct SCRIPTCONTEXT;
BOOL Coop_ScrNative_GetMenuChoice(struct SCRIPTCONTEXT *ctx);

// Overlay 129 calls these through Overlay 131's pinned header, in the field.
// A script gift (GiveMon) and a roamer's species (Save_CreateRoamerByID):
// species | form << 11 in, the room's replacement out (or the same value).
u32 Coop_ChallengeGiftSpecies(u32 value);
u32 Coop_ChallengeRoamerSpecies(u32 roamerIdx, u32 vanilla);

// Phase 4: trades (src/field/coop_trade.c). Why this party Pokemon can't be
// offered (NULL: it can); why an offered one can't be received (Monolocke:
// neither it nor its trade evolution has the type; NULL: it can); and the
// bits a received Pokemon drops.
struct Party;
const char *Coop_ChallengeTradeOfferRule(struct PartyPokemon *mon);
const char *Coop_ChallengeTradeRefusal(struct Party *party, struct PartyPokemon *incoming);
void Coop_ChallengeTradeReceived(struct PartyPokemon *mon);

// Phase 4: the level cap for this progress (docs/MILESTONE5_PLAN.md §5.8).
u32 Coop_ChallengeLevelCapFor(u32 johto, u32 kanto, BOOL gameClear, BOOL round2Beaten);

// The pool entry for an encounter value (or its base species), NULL if none.
const struct CoopPoolEntry *Coop_PoolFind(u32 value);

// ---------------------------------------------------------------------------
// Phase 3: Nuzlocke (src/field/coop_challenge_nuzlocke.c, Overlay 131)
// ---------------------------------------------------------------------------

// The location of a wild battle (docs/MILESTONE5_PLAN.md §4.3), 0xFF none.
u32 Coop_ChallengeLocation(struct FieldSystem *fsys, u32 fightType, u32 map);

// TRUE when this player already caught the family of `value` (dupes clause).
BOOL Coop_ChallengeFamilyCaught(u32 value);
const u8 *Coop_ChallengeFamilyMask(void); // the same list as a bitmask (pool family index)

// Dupes clause with the randomizer off: another species from the same table
// section of the current map, whose family isn't caught yet; 0 = none.
u32 Coop_ChallengeTableReroll(struct FieldSystem *fsys, u32 original);

// The wild battle recorded in pending_*: caught or failed. Once the field is
// quiet after it, or by the next wild hook if another battle came first.
void Coop_ChallengeResolvePending(struct FieldSystem *fsys, struct CoopChallengeSave *save);

// Phase 6: Soul-Link is on in this save's rules.
static inline BOOL Coop_ChallengeSoulLinkOn(const struct CoopChallengeSave *save)
{
    return save != NULL && (save->rules & COOP_RULE_NUZLOCKE) && (save->rules & COOP_RULE_SOUL_LINK_MASK);
}

// Once per field frame (from Coop_ChallengeFieldFrame): deaths, the Graveyard,
// the battle just ended, Day Care eggs, Soul-Link failures (Phase 6).
void Coop_ChallengeNuzlockeFrame(struct FieldSystem *fsys, struct CoopChallengeSave *save);

// Poke Marts leave revives out in a Nuzlocke (src/field/mart.c).
BOOL Coop_ChallengeHidesItem(u16 item);
void Coop_ChallengeFilterMarts(void); // src/field/mart.c

// ---------------------------------------------------------------------------
// Phase 3: battle side (src/battle/coop_challenge_battle.c, battle extension)
// ---------------------------------------------------------------------------

struct BattleSystem;
struct BattleStruct;

// The ball the player just threw may not catch this wild Pokemon (it is
// refunded). Called by hg-engine's ShouldPreventMonCapture.
BOOL LONG_CALL Coop_ChallengeBallBlocked(struct BattleSystem *bsys);

// Why Coop_ChallengeBallBlocked last blocked a ball (PrintBallBlockedMessage),
// and the Monolocke type for its message.
#define COOP_BALL_ALLOWED   0
#define COOP_BALL_MONOLOCKE 1
#define COOP_BALL_NUZLOCKE  2
#define COOP_BALL_SOUL_LINK 3 // Phase 6: the link group already lost this first encounter's place
u32 LONG_CALL Coop_ChallengeBallReason(u32 *monoType);

// A player Pokemon fainted (host / local battles: the faint command) or the
// battle ended (every machine, link battles included): mark it dead.
void LONG_CALL Coop_ChallengeMarkFainted(struct BattleSystem *bsys, struct BattleStruct *ctx, int battlerId);
void LONG_CALL Coop_ChallengeMarkBattleEnd(struct BattleSystem *bsys);

// TRUE: the capture skips "Give a nickname?" and opens the naming screen
// (asm/battle/battle_hooks.s, Coop_ForceNickname_hook).
BOOL LONG_CALL Coop_ChallengeForceNickname(void);

// ---------------------------------------------------------------------------
// Phase 5: Ghost Mode, the spirit state (src/field/coop_spirit.c, Overlay 131;
// docs/MILESTONE5_PLAN.md §5.5)
// ---------------------------------------------------------------------------

// A battle is being built (the wild hook, the trainer party): where the
// player stands now is the spirit's anchor if the battle wipes the party.
void Coop_ChallengeRecordPrebattle(struct FieldSystem *fsys);

// The sweep found no living Pokemon anywhere (§5.4.5 step 6): become a spirit.
void Coop_SpiritEnter(struct FieldSystem *fsys, struct CoopChallengeSave *save);

// Once per field frame of a Nuzlocke run, after the sweep: the warps to the
// anchor and back, the spirit task, the "Run lost" ending.
void Coop_SpiritFieldFrame(struct FieldSystem *fsys, struct CoopChallengeSave *save);

// The spirit task is running and only roaming (not in its menu): teammates
// can talk to this player.
BOOL Coop_SpiritIsRoaming(struct FieldSystem *fsys);

// This player is a spirit (the save's state).
BOOL Coop_IsSpirit(void);

// Revival by donation (§5.5.4). Why the donor can't give party `slot` (or,
// with slot 0xFF, can't give at all), NULL when it can; and the spirit's
// side once the donated Pokemon is in its party.
const char *Coop_SpiritDonorRule(struct Party *party, u32 slot);
void Coop_SpiritRevived(struct FieldSystem *fsys);


#endif // COOP_CHALLENGE_H
