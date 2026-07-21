.include "asm/include/battle_commands.inc"

.data

_000:
    UpdateVar OPCODE_SET, BSCRIPT_VAR_SIDE_EFFECT_TYPE, SIDE_EFFECT_TYPE_ABILITY
    TryPerishSong _045
    PlayBattleAnimation BATTLER_CATEGORY_DEFENDER, BATTLE_ANIMATION_PERISH_SONG
    // All Pokémon hearing the song will faint in three turns!
    PrintMessage 822, TAG_NONE
    Wait 
    WaitButtonABTime 30

_045:
    End 
