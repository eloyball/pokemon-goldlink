#include "config.h"
#include "types.h"

#include "constants/maps.h"

#include "save.h"
#include "script.h"
#include "coop_wild.h"

// ---------------------------------------------------------------------------
// Kanto wild levels (docs/GAME_REBALANCE_UPDATE.md §3.3), Overlay 131. Kanto's
// gyms can be beaten in any order, so its wild Pokemon follow the player:
// every wild Pokemon met on a Kanto map gains COOP_KANTO_LEVELS_PER_BADGE
// levels per Kanto badge. Called by hg-engine's two wild-creation hooks
// (asm/other_hook.s, Overlay 2), which only run in the field.
// ---------------------------------------------------------------------------

#define COOP_KANTO_LEVELS_PER_BADGE 2

// Kanto maps with wild Pokemon (the plan's Act 5 tables and the towns' water).
// Routes 26-27 come before the Elite Four; Route 28, Cerulean Cave and
// Mt. Silver are post-game: none of them is scaled.
static const u16 sKantoWildMaps[] = {
    MAP_D01R0101, MAP_D02R0101, MAP_D02R0102, MAP_D02R0103, MAP_D02R0104, MAP_D05R0101,
    MAP_D05R0102, MAP_D11R0101, MAP_D11R0102, MAP_D11R0103, MAP_D11R0104, MAP_D11R0105,
    MAP_D46R0101, MAP_R01, MAP_R02, MAP_R02R0101, MAP_R03, MAP_R04,
    MAP_R05, MAP_R06, MAP_R07, MAP_R08, MAP_R09, MAP_R10,
    MAP_R11, MAP_R12, MAP_R13, MAP_R14, MAP_R15, MAP_R16R0301,
    MAP_R17, MAP_R18, MAP_R22, MAP_R24, MAP_R25, MAP_T01,
    MAP_T02, MAP_T04, MAP_T06, MAP_T07, MAP_T08, MAP_T09,
    MAP_W19, MAP_W20, MAP_W21,
};

// Kanto maps whose trainers scale too (src/field/coop_battle.c): the wild maps,
// plus the gyms, Blaine's Seafoam gym, the Fighting Dojo and the Power Plant.
static const u16 sKantoTrainerMaps[] = {
    MAP_T02GYM0101, MAP_T03GYM0101, MAP_T04GYM0101, MAP_T06GYM0101, MAP_T07GYM0101, MAP_T08GYM0101,
    MAP_T11GYM0101, MAP_D11R0106, MAP_T11R0101, MAP_R10R0101,
};

static BOOL Coop_InList(const u16 *list, u32 count, u32 map)
{
    u32 i;

    for (i = 0; i < count; i++)
        if (list[i] == map)
            return TRUE;
    return FALSE;
}

BOOL Coop_IsKantoTrainerMap(u32 map)
{
    return Coop_InList(sKantoWildMaps, NELEMS(sKantoWildMaps), map) || Coop_InList(sKantoTrainerMaps, NELEMS(sKantoTrainerMaps), map);
}

u32 Coop_WildLevelAdjust(u32 level)
{
    FieldSystem *fsys = gFieldSysPtr;
    struct PlayerProfile *profile;
    u32 map, badges, bits;

    if (fsys == NULL || fsys->location == NULL || fsys->savedata == NULL)
        return level;
    map = fsys->location->mapId;
    if (!Coop_InList(sKantoWildMaps, NELEMS(sKantoWildMaps), map))
        return level;
    profile = Sav2_PlayerData_GetProfileAddr(fsys->savedata);
    if (profile == NULL)
        return level;
    for (badges = 0, bits = profile->kantoBadges; bits != 0; bits &= bits - 1)
        badges++;
    level += COOP_KANTO_LEVELS_PER_BADGE * badges;
    return level > 100 ? 100 : level;
}
