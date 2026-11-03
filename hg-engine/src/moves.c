#include "../include/constants/moves.h"

#include "../include/battle.h"
#include "../include/config.h"
#include "../include/constants/ability.h"
#include "../include/constants/battle_script_constants.h"
#include "../include/constants/file.h"
#include "../include/constants/item.h"
#include "../include/pokemon.h"
#include "../include/types.h"

void GetMoveDataTable(void *dest)
{
    ReadFromNarcMemberByIdPair(dest, ARC_MOVE_DATA, 0, 0, sizeof(struct BattleMove) * (NUM_OF_MOVES + 1));
}

/**
 *  @brief get move data field requested from ARC_MOVE_DATA
 *
 *  @param id move index
 *  @param field MOVE_DATA_* constant requesting data
 *  @return requested data
 */
u32 LONG_CALL GetMoveData(u16 id, u32 field)
{
    struct BattleMove *bm = sys_AllocMemory(0, sizeof(struct BattleMove));
    ReadWholeNarcMemberByIdPair(bm, ARC_MOVE_DATA, id);
    u32 ret = 0;

    switch (field) {
    case MOVE_DATA_EFFECT:
        ret = bm->effect;
        break;
    case MOVE_DATA_PSS_SPLIT:
        ret = bm->effect;
        break;
    case MOVE_DATA_BASE_POWER:
        ret = bm->power;
        break;
    case MOVE_DATA_TYPE:
        ret = bm->type;
        break;
    case MOVE_DATA_ACCURACY:
        ret = bm->accuracy;
        break;
    case MOVE_DATA_BASE_PP:
        ret = bm->pp;
        break;
    case MOVE_DATA_SECONDARY_EFFECT_CHANCE:
        ret = bm->secondaryEffectChance;
        break;
    case MOVE_DATA_TARGET:
        ret = bm->target;
        break;
    case MOVE_DATA_PRIORITY:
        ret = bm->priority;
        break;
    case MOVE_DATA_FLAGS:
        ret = bm->flag;
        break;
    }

    sys_FreeMemoryEz(bm);

    return ret;
}

/**
 *  @brief check if a move is unimplemented
 *
 *  @param move move ID to check
 *  @return TRUE if move is unimplemented; FALSE otherwise
 */
BOOL LONG_CALL IsMoveUnimplemented(u16 move)
{
#ifdef BLOCK_LEARNING_UNIMPLEMENTED_MOVES
    return (GetMoveData(move, MOVE_DATA_FLAGS) & FLAG_UNUSED_MOVE) != 0;
#else
    return FALSE;
#endif
}
