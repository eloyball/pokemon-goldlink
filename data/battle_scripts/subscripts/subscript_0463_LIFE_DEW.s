.include "asm/include/battle_commands.inc"

.data

_000:
    UpdateMonDataFromVar OPCODE_GET, BATTLER_CATEGORY_MSG_BATTLER_TEMP, BMON_DATA_MAXHP, BSCRIPT_VAR_HP_CALC
    DivideVarByValueRoundUp BSCRIPT_VAR_HP_CALC, 4
    Call BATTLE_SUBSCRIPT_UPDATE_HP
    // {0} had its HP restored.
    PrintMessage 1396, TAG_NICKNAME, BATTLER_CATEGORY_MSG_BATTLER_TEMP
    Wait
    WaitButtonABTime 30
    End
