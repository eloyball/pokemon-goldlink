#!/usr/bin/env python3
"""Milestone 4 Phase 2: writes the plan's wild Pokemon into hg-engine's tables.

Reads the placement tables of scripts/rebalance_dex_plan.py and rewrites, in place:
  - data/Encounters.c: grass / cave slots (morning, day, night), surfing, the three rods
    and Rock Smash of every table an area maps to (TABLES below). Encounter rates, swarms,
    night fishing and the Hoenn / Sinnoh Sound species (the radio stays vanilla) are kept.
  - data/Headbutt.c: the 12 normal tree slots of the routes that have Headbutt lists.
  - data/SafariEncounters.c: every area's slots; bonus slots copy the base ones and their
    unlock rules can never be met (docs/GAME_REBALANCE_UPDATE.md §6.3, §18 B).

Slot layout (docs/REBALANCE_DEX_PLAN.md): grass slots 0-7 core (night swaps take slots 0-5 at
night), 8-9 the first two Rare entries (4 %), 10-11 starters, then further Rare entries (1 %).
A starter appears as the stage its slot level reaches. Areas spanning several tables spread
their lists over them. Methods a table does not have (rate 0) are reported, not written.

  scripts/rebalance_encounters.py          # dry run: report only
  scripts/rebalance_encounters.py --write  # rewrite the three files
"""
import importlib.util
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
HG = ROOT / 'hg-engine'

spec = importlib.util.spec_from_file_location('dexplan', ROOT / 'scripts' / 'rebalance_dex_plan.py')
plan = importlib.util.module_from_spec(spec)
spec.loader.exec_module(plan)

# Plan area -> Encounters.c tables (ENCDATA_ names without the prefix).
TABLES = {
    'Route 29': ['R29_ROUTE_29'],
    'Route 46 (south)': ['R46_ROUTE_46'],
    'Route 30': ['R30_ROUTE_30'],
    'Route 31': ['R31_ROUTE_31'],
    'Dark Cave (Route 31 side)': ['D42R0102_DARK_CAVE_ROUTE_31_ENTRANCE'],
    'Sprout Tower': ['D15R0102_SPROUT_TOWER_2F', 'D15R0103_SPROUT_TOWER_3F'],
    'Route 32': ['R32_ROUTE_32'],
    'Ruins of Alph': ['D24R0101_RUINS_OF_ALPH_OUTSIDE'],
    'Union Cave 1F / B1F': ['D25R0101_UNION_CAVE_1F', 'D25R0102_UNION_CAVE_B1F'],
    'Route 33 (always raining)': ['R33_ROUTE_33'],
    'Slowpoke Well': ['D26R0102_SLOWPOKE_WELL_1F', 'D26R0103_SLOWPOKE_WELL_B2F'],
    'Ilex Forest': ['D36R0101_ILEX_FOREST'],
    'Route 34': ['R34_ROUTE_34'],
    'National Park': ['D22R0101_NATIONAL_PARK'],
    'Bug-Catching Contest (Tue/Thu/Sat)': ['D22R0102_NATIONAL_PARK_BUG_CATCHING_CONTEST'],
    'Route 35': ['R35_ROUTE_35'],
    'Route 36': ['R36_ROUTE_36'],
    'Route 37': ['R37_ROUTE_37'],
    'Burned Tower 1F': ['D18R0101_BURNED_TOWER_1F'],
    'Burned Tower B1F': ['D18R0102_BURNED_TOWER_B1F'],
    'Route 38': ['R38_ROUTE_38'],
    'Route 39': ['R39_ROUTE_39'],
    'Routes 40 / 41 (sea)': ['W40_ROUTE_40', 'W41_ROUTE_41'],
    'Cianwood City': ['T24_CIANWOOD_CITY'],
    'Whirl Islands': ['D40R0101_WHIRL_ISLANDS_1F', 'D40R0102_WHIRL_ISLANDS_B1F', 'D40R0104_WHIRL_ISLANDS_B2F',
                      'D40R0106_WHIRL_ISLANDS_B3F_LEDGE_OVERLOOKING_LUGIA_ROOM'],
    'Route 42': ['R42_ROUTE_42'],
    'Mt. Mortar': ['D38R0101_MT_MORTAR_WATERFALL_ROOM', 'D38R0102_MT_MORTAR_CENTRAL_ROOM',
                   'D38R0103_MT_MORTAR_ROOM_ABOVE_WATERFALL'],
    'Mt. Mortar B1F (Karate King dojo)': ['D38R0104_MT_MORTAR_B1F'],
    'Route 47': ['R47_ROUTE_47'],
    'Route 48 (Safari gate)': ['R48_ROUTE_48'],
    'Route 43': ['R43_ROUTE_43'],
    'Lake of Rage (water only)': ['T29_LAKE_OF_RAGE'],
    'Route 44': ['R44_ROUTE_44'],
    'Ice Path': ['D39R0101_ICE_PATH_1F', 'D39R0102_ICE_PATH_B1F', 'D39R0103_ICE_PATH_B2F', 'D39R0104_ICE_PATH_B3F'],
    "Dragon's Den": ['D44R0102_DRAGONS_DEN'],
    'Dark Cave (Route 45 side)': ['D42R0101_DARK_CAVE_ROUTE_45_ENTRANCE'],
    'Route 45': ['R45_ROUTE_45'],
    'Route 27': ['R27_ROUTE_27'],
    'Tohjo Falls': ['D45R0101_TOHJO_FALLS'],
    'Route 26': ['R26_ROUTE_26'],
    'Victory Road': ['D43R0101_VICTORY_ROAD_1F', 'D43R0102_VICTORY_ROAD_2F', 'D43R0103_VICTORY_ROAD_3F'],
    'Routes 6 & 11': ['R06_ROUTE_6', 'R11_ROUTE_11'],
    "Diglett's Cave": ['D01R0101_DIGLETTS_CAVE'],
    'Routes 5, 7 & 8': ['R05_ROUTE_5', 'R07_ROUTE_7', 'R08_ROUTE_8'],
    'Routes 9 & 10': ['R09_ROUTE_9', 'R10_ROUTE_10'],
    'Rock Tunnel': ['D05R0101_ROCK_TUNNEL_1F', 'D05R0102_ROCK_TUNNEL_B1F'],
    'Routes 24 & 25': ['R24_ROUTE_24', 'R25_ROUTE_25'],
    'Routes 16, 17 & 18 (Cycling Road)': ['R16R0301_ROUTE_16', 'R17_ROUTE_17', 'R18_ROUTE_18'],
    'Routes 12 to 15': ['R12_ROUTE_12', 'R13_ROUTE_13', 'R14_ROUTE_14', 'R15_ROUTE_15'],
    'Routes 19 to 21 (sea)': ['W19_ROUTE_19', 'W20_ROUTE_20', 'W21_ROUTE_21'],
    'Seafoam Islands': ['D11R0101_SEAFOAM_ISLANDS_1F', 'D11R0102_SEAFOAM_ISLANDS_B1F', 'D11R0103_SEAFOAM_ISLANDS_B2F',
                        'D11R0104_SEAFOAM_ISLANDS_B3F', 'D11R0105_SEAFOAM_ISLANDS_B4F'],
    'Routes 3 & 4': ['R03_ROUTE_3', 'R04_ROUTE_4'],
    'Mt. Moon (fossil cave)': ['D02R0101_MT_MOON_1F', 'D02R0102_MT_MOON_2F'],
    'Routes 1, 2 & 22, Viridian Forest': ['R01_ROUTE_1', 'R02_ROUTE_2_SOUTH_BELOW_VIRIDIAN_FOREST',
                                          'R02R0101_ROUTE_2_NORTH_ABOVE_VIRIDIAN_FOREST', 'R22_ROUTE_22',
                                          'D46R0101_VIRIDIAN_FOREST'],
    'Route 28 (Mt. Silver approach)': ['R28_ROUTE_28'],
    'Cerulean Cave': ['D03R0101_CERULEAN_CAVE_1F', 'D03R0102_CERULEAN_CAVE_B1F', 'D03R0103_CERULEAN_CAVE_B2F'],
    'Mt. Silver': ['T31_MT_SILVER_OUTSIDE_POKEMON_CENTER', 'D41R0101_MT_SILVER_1F', 'D41R0103_MT_SILVER_MOUNTAINSIDE',
                   'D41R0104_MT_SILVER_EXPERT_BELT_ROOM', 'D41R0102_MT_SILVER_TOP_SNOWY_AREA', 'D41R0106_MT_SILVER_3F',
                   'D41R0107_MT_SILVER_4F', 'D41R0105_MT_SILVER_MOLTRES_ROOM'],
}

# Plan area -> Headbutt.c members (normal tree slots).
HEADBUTT = {
    'Route 29': ['route29'], 'Route 46 (south)': ['route46'], 'Route 30': ['route30'], 'Route 32': ['route32'],
    'Route 33 (always raining)': ['route33'], 'Ilex Forest': ['ilexForest'], 'National Park': ['nationalPark'],
    'Route 36': ['route36'], 'Route 37': ['route37'], 'Route 38': ['route38'], 'Route 39': ['route39'],
    'Route 42': ['route42'], 'Route 45': ['route45'], 'Routes 12 to 15': ['route12', 'route13', 'route14', 'route15'],
    'Routes 1, 2 & 22, Viridian Forest': ['route1', 'route2', 'route22', 'viridianForest'],
}

SAFARI_AREAS = {'Plains': 'PLAINS', 'Meadow': 'MEADOW', 'Savannah': 'SAVANNAH', 'Peak': 'PEAK', 'Rocky Beach': 'ROCKY_BEACH',
                'Wetland': 'WETLAND', 'Forest': 'FOREST', 'Swamp': 'SWAMP', 'Marshland': 'MARSHLAND',
                'Wasteland': 'WASTELAND', 'Mountain': 'MOUNTAIN', 'Desert': 'DESERT'}
# Water forms for Safari areas the plan gives no water list (their ponds still need alternate forms).
SAFARI_WATER = ['SHELLOS_EAST_SEA', 'CORSOLA_GALARIAN', 'QWILFISH_HISUIAN', 'SLOWPOKE_GALARIAN', 'BASCULIN_BLUE_STRIPED',
                'BASCULIN_WHITE_STRIPED', 'WOOPER_PALDEAN', 'TATSUGIRI_DROOPY', 'TATSUGIRI_STRETCHY', 'GASTRODON_EAST_SEA']
SAFARI_LEVELS = (28, 38)

STARTERS = {'BULBASAUR', 'CHARMANDER', 'SQUIRTLE', 'CHIKORITA', 'CYNDAQUIL', 'TOTODILE', 'TREECKO', 'TORCHIC', 'MUDKIP',
            'TURTWIG', 'CHIMCHAR', 'PIPLUP', 'SNIVY', 'TEPIG', 'OSHAWOTT', 'CHESPIN', 'FENNEKIN', 'FROAKIE', 'ROWLET',
            'LITTEN', 'POPPLIO', 'GROOKEY', 'SCORBUNNY', 'SOBBLE', 'SPRIGATITO', 'FUECOCO', 'QUAXLY'}


def load_forms():
    """Form-species constant -> (base species, form number), from PokeFormDataTbl.c's order."""
    forms = {}
    base = None
    n = 0
    for line in open(HG / 'data/PokeFormDataTbl.c'):
        m = re.match(r'\s*\[SPECIES_(\w+)\] = \{', line)
        if m:
            base, n = m.group(1), 0
            continue
        m = re.match(r'\s*(NEEDS_REVERSION \| )?SPECIES_(\w+),', line)
        if m and base:
            n += 1
            forms.setdefault(m.group(2), (base, n))
    return forms


def load_level_evos():
    evo, cur = {}, None
    for line in open(HG / 'data/Evolutions.c'):
        m = re.match(r'\s*\[SPECIES_(\w+)\]\s*=', line)
        if m:
            cur = m.group(1)
            continue
        m = re.search(r'\{ EVO_LEVEL, (\d+), SPECIES_(\w+) \}', line)
        if m and cur and cur not in evo:
            evo[cur] = (int(m.group(1)), m.group(2))
    return evo


FORMS = load_forms()
LEVEL_EVOS = load_level_evos()


def species(name):
    if name in FORMS:
        base, n = FORMS[name]
        return f'MON_WITH_FORM(SPECIES_{base}, {n})'
    return f'SPECIES_{name}'


def staged(name, level):
    """A starter at the stage its level reaches."""
    while name in LEVEL_EVOS and LEVEL_EVOS[name][0] <= level:
        name = LEVEL_EVOS[name][1]
    return name


def band(lv):
    lo, hi = (int(x) for x in lv.split('-'))
    return lo, hi


def spread(lst, t, n, size):
    """Table t of n: the list rotated so every entry lands in some table, `size` entries (cycled)."""
    if not lst:
        return []
    rot = lst[(t * size) % len(lst):] + lst[:(t * size) % len(lst)] if len(lst) > size else lst
    out = [rot[i % len(rot)] for i in range(size)]
    return out


def land_slots(m, t, n, lo, hi):
    core = spread(m.get('day', []), t, n, 8) if len(m.get('day', [])) > 8 else m.get('day', [])
    night = spread(m.get('night', []), t, n, 6) if len(m.get('night', [])) > 6 else m.get('night', [])
    rare = m.get('rare', [])
    starters = m.get('starter', [])
    if not core:
        core = night or rare
    if not core:
        return None
    levels = [lo + round((hi - 1 - lo) * k / 7) for k in range(8)] + [hi - 1, hi - 1, hi, hi]
    day = [core[i % len(core)] for i in range(8)]
    nite = list(day)
    for i, s in enumerate(night[:6]):
        nite[i] = s
    # Rare: 4 % slots get this table's turn of the first two; 1 % slots its starters, then the rest.
    r4 = rare[:2] if rare else day[:2]
    if len(r4) == 1:
        r4 = r4 * 2
    tail = [s for i, s in enumerate(starters) if i % n == t] + [s for i, s in enumerate(rare[2:]) if i % n == t]
    if not tail:
        tail = [s for i, s in enumerate(starters + rare[2:]) if i % n == t % max(1, len(starters + rare[2:]))]
    r1 = (tail + r4)[:2] if len(tail) < 2 else tail[:2]
    rest = r4 + r1
    out = []
    for lst in (day, day, nite):
        full = lst + rest
        out.append([staged(s, levels[i]) if s in STARTERS else s for i, s in enumerate(full)])
    return levels, out


def water_slots(lst, lo, hi, offset=0):
    if not lst:
        return None
    return [(lo, hi, lst[(i + offset) % len(lst)]) for i in range(5)]


def rods(fish, lo, hi):
    if not fish:
        return None
    f = lambda k: fish[k % len(fish)]
    old = [(max(2, lo - 15), max(2, lo - 10), f(k)) for k in (0, 0, 1, 1, 2)]
    good = [(max(2, lo - 5), lo, f(k)) for k in (0, 1, 2, 1, 3)]
    sup = [(lo, hi, f(k)) for k in (1, 2, 3, 4, 5)]
    return old, good, sup


def sub_list(block, field, entries, fmt):
    """Replace the entries of `.field = { ... }` (first occurrence) in block."""
    i = block.index(f'.{field} =')
    j = block.index('{', i)
    depth, k = 0, j
    while True:
        if block[k] == '{':
            depth += 1
        elif block[k] == '}':
            depth -= 1
            if depth == 0:
                break
        k += 1
    inner = block[j + 1:k]
    indent = re.search(r'\n(\s+)\S', inner).group(1)
    body = '\n'.join(indent + fmt(e) + ',' for e in entries)
    close = block[block.rindex('\n', 0, k) + 1:k]
    return block[:j + 1] + '\n' + body + '\n' + close + block[k:]


def write_encounters(report):
    path = HG / 'data/Encounters.c'
    s = open(path).read()
    for name, act, lv, m in plan.A:
        tabs = TABLES.get(name)
        if not tabs:
            report.append(f'no tables for area {name}')
            continue
        lo, hi = band(lv)
        grass = [tab for tab in tabs if re.search(r'\.rateWalk = [1-9]', s[s.index(f'    [ENCDATA_{tab}] = {{'):][:400])]
        for t, tab in enumerate(tabs):
            i = s.index(f'    [ENCDATA_{tab}] = {{')
            j = s.find('\n    [ENCDATA_', i + 10)
            j = len(s) if j < 0 else j
            b = s[i:j]
            rate = {k: int(re.search(rf'\.{k} = (\d+)', b).group(1)) for k in
                    ('rateWalk', 'rateSurf', 'rateRockSmash', 'rateOldRod', 'rateGoodRod', 'rateSuperRod')}
            ls = land_slots(m, grass.index(tab), len(grass), lo, hi) if tab in grass else None
            if ls and rate['rateWalk']:
                levels, (morn, day, nite) = ls
                b = sub_list(b, 'levels', [', '.join(str(x) for x in levels)], lambda e: e)
                b = re.sub(r'(\.levels = \{\n\s+[\d, ]+),', r'\1', b)
                b = sub_list(b, 'speciesMorning', morn, species)
                b = sub_list(b, 'speciesDay', day, species)
                b = sub_list(b, 'speciesNight', nite, species)
            if not grass and m.get('day') and t == 0:
                report.append(f'{name}: no table with grass or cave encounters; the core list is not placed')
            surf = m.get('surf', [])
            if not rate['rateWalk']:
                # Sea tables: the rares and starters ride the surfing slots 2-4.
                extra = [s_ for i_, s_ in enumerate(m.get('rare', []) + m.get('starter', [])) if i_ % len(tabs) == t]
                if surf:
                    surf = [surf[t % len(surf)], surf[(t + 1) % len(surf)]] + (extra + surf)[:3]
            ws = water_slots(surf, lo, hi)
            if ws and rate['rateSurf']:
                b = sub_list(b, 'surfSlots', ws, lambda e: f'{{ {e[0]}, {e[1]}, {species(e[2])} }}')
            elif ws:
                report.append(f'{tab}: no surfing here')
            rs = rods(m.get('fish', []), lo, hi)
            if rs and rate['rateOldRod']:
                for field, lst in zip(('oldRodSlots', 'goodRodSlots', 'superRodSlots'), rs):
                    b = sub_list(b, field, lst, lambda e: f'{{ {e[0]}, {e[1]}, {species(e[2])} }}')
            elif rs:
                report.append(f'{tab}: no fishing here')
            sm = m.get('smash', [])
            if sm and rate['rateRockSmash']:
                part = [x for i_, x in enumerate(sm) if i_ % len(tabs) == t] or sm
                pair = (part + part)[:2]
                b = sub_list(b, 'rockSmashSlots', [(lo, hi, pair[0]), (lo, hi, pair[1])],
                             lambda e: f'{{ {e[0]}, {e[1]}, {species(e[2])} }}')
            elif sm:
                report.append(f'{tab}: no Rock Smash here')
            s = s[:i] + b + s[j:]
    return path, s


def write_headbutt(report):
    path = HG / 'data/Headbutt.c'
    s = open(path).read()
    for name, act, lv, m in plan.A:
        trees = m.get('headbutt')
        if not trees:
            continue
        members = HEADBUTT.get(name)
        if not members:
            report.append(f'no Headbutt table for area {name}')
            continue
        lo, hi = band(lv)
        for mem in members:
            i = s.find(f'    .{mem} = {{')
            if i < 0:
                report.append(f'Headbutt member {mem} not found')
                continue
            j = s.index('.treeCoords', i)
            b = s[i:j]
            if int(re.search(r'\.normalTreeCount = (\d+)', b).group(1)) == 0:
                report.append(f'Headbutt {mem}: no normal trees')
                continue
            slots = [(lo if k < 6 else (lo + hi) // 2, (lo + hi) // 2 if k < 6 else hi, trees[k % len(trees)]) for k in range(12)]
            b = sub_list(b, 'normalSlots', slots, lambda e: f'{{ {species(e[2])}, {e[0]}, {e[1]} }}')
            s = s[:i] + b + s[j:]
    return path, s


def write_safari(report):
    path = HG / 'data/SafariEncounters.c'
    s = open(path).read()
    lo, hi = SAFARI_LEVELS
    unlock = '{ .objects = { { SAFARI_ZONE_OBJECT_TYPE_PLAINS, 255 }, { SAFARI_ZONE_OBJECT_TYPE_NONE, 0 } } }'
    for k, (biome, theme, m) in enumerate(plan.SAFARI):
        area = SAFARI_AREAS[biome]
        i = s.index(f'    [SAFARI_ZONE_AREA_{area}] = {{')
        j = s.find('\n    [SAFARI_ZONE_AREA_', i + 10)
        j = len(s) if j < 0 else j
        b = s[i:j]
        land, night, rare = m.get('land', []), m.get('night', []), m.get('rare', [])
        extra = m.get('vivillon', []) + m.get('minior', [])
        # Ten slots, one in ten each: every form gets a slot in some time of day.
        morn = (land + rare + extra)
        day = (land + rare + extra[::-1])
        nite = (night + land + rare + extra)
        lists = []
        for pool in (morn, day, nite):
            pool = list(dict.fromkeys(pool))
            lists.append([pool[x % len(pool)] for x in range(10)])
        allforms = set(land + night + rare + extra)
        placed = set(lists[0] + lists[1] + lists[2])
        if allforms - placed:
            # More forms than slots: spread the leftovers over the night list's repeats.
            left = sorted(allforms - placed)
            for x, f in enumerate(left):
                lists[2][9 - x] = f
            if set(land + night + rare + extra) - set(lists[0] + lists[1] + lists[2]):
                report.append(f'Safari {biome}: forms without a slot: {sorted(allforms - set(sum(lists, [])))}')
        lv = lambda x, f: hi - 2 + x % 3 if f in rare else lo + x % 7
        water = m.get('surf', []) + [f for f in m.get('fish', []) if f not in m.get('surf', [])]
        if not water:
            water = [SAFARI_WATER[(k + x) % len(SAFARI_WATER)] for x in range(3)]
        sections = {'land': lists, 'surf': [[water[x % len(water)] for x in range(10)]] * 3}
        rodfish = m.get('fish', []) or water
        for rod in ('oldRod', 'goodRod', 'superRod'):
            sections[rod] = [[rodfish[x % len(rodfish)] for x in range(10)]] * 3
        for sec, (lm, ld, ln) in sections.items():
            si = b.index(f'        .{sec} = {{')
            sj = b.find('\n        .', si + 10)
            sj = len(b) if sj < 0 else sj
            sb = b[si:sj]
            for field, lst in (('speciesMorning', lm), ('speciesDay', ld), ('speciesNight', ln)):
                sb = sub_list(sb, field, [(f, lv(x, f)) for x, f in enumerate(lst)], lambda e: f'{{ {species(e[0])}, {e[1]} }}')
            nb = {'land': 10, 'surf': 3}.get(sec, 2)
            for field, lst in (('bonusSpeciesMorning', lm), ('bonusSpeciesDay', ld), ('bonusSpeciesNight', ln)):
                sb = sub_list(sb, field, [(f, lv(x, f)) for x, f in enumerate(lst[:nb])], lambda e: f'{{ {species(e[0])}, {e[1]} }}')
            sb = sub_list(sb, 'bonusUnlockConditions', [unlock] * nb, lambda e: e)
            b = b[:si] + sb + b[sj:]
        s = s[:i] + b + s[j:]
    return path, s


def main():
    report = []
    outs = [write_encounters(report), write_headbutt(report), write_safari(report)]
    for r in report:
        print(r)
    if '--write' in sys.argv:
        for path, text in outs:
            open(path, 'w').write(text)
            print(f'wrote {path.relative_to(ROOT)}')
    return 0


if __name__ == '__main__':
    sys.exit(main())
