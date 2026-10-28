#ifndef MEGA_H
#define MEGA_H

#include "types.h"

#include "battle.h"

BOOL LONG_CALL CheckCanMega(struct BattleStruct *battle, int client);
BOOL CheckCanDrawMegaButton(struct BI_PARAM *bip);
BOOL CheckIsMega(struct BI_PARAM *bip);
BOOL CheckIsPrimalGroudon(struct BI_PARAM *bip);
BOOL CheckIsPrimalKyogre(struct BI_PARAM *bip);
BOOL LONG_CALL CheckCanSpeciesMegaEvolveByMove(struct BattleStruct *sp, u32 client);
BOOL IsMegaSpeciesByMove(u32 species, u32 form);
BOOL LONG_CALL CheckMegaData(u32 mon, u32 item, u32 form);

// co-op: byte 1 of a link battle's move or target reply when the player asked to
// Mega Evolve (src/battle/mega.c, read by src/individual/ServerBeforeAct.c).
#define COOP_MEGA_REQUEST_TAG 0x4D

#endif
