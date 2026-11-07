#!/usr/bin/env python3
"""Milestone 5 species pool for the randomizer (docs/MILESTONE5_PLAN.md §4.5).

Writes hg-engine/src/field/coop_challenge_pool.c: every species and wild alternate form
with its family, types, BST tier and legend pool, sorted by its encounter value
(species | form << 11) so the ROM can find a wild slot's original quickly; and the
random-ability allow-list.

    scripts/challenge_pool.py           write the table
    scripts/challenge_pool.py --check   exit 1 if the written table is out of date

Sources: hg-engine's species constants, data/Species.c (base stats, types),
data/Evolutions.c and data/BabyMons.c (families), data/PokeFormDataTbl.c (form numbers),
and scripts/rebalance_dex_plan.py (the 105 legendary, mythical and Ultra Beast species,
the Safari's alternate forms). Legend pools follow docs/CHALLENGE_RULES.md §2.3.
"""

import importlib.util
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
HG = ROOT / 'hg-engine'
OUT = HG / 'src' / 'field' / 'coop_challenge_pool.c'

spec = importlib.util.spec_from_file_location('dexplan', ROOT / 'scripts' / 'rebalance_dex_plan.py')
plan = importlib.util.module_from_spec(spec)
spec.loader.exec_module(plan)

# Pool B: box / restricted legends and mythicals (CHALLENGE_RULES.md §2.3). Every other
# deferred legendary, mythical or Ultra Beast species is Pool A.
POOL_B = {
    'MEWTWO', 'LUGIA', 'HO_OH', 'KYOGRE', 'GROUDON', 'RAYQUAZA', 'DIALGA', 'PALKIA', 'GIRATINA',
    'RESHIRAM', 'ZEKROM', 'KYUREM', 'XERNEAS', 'YVELTAL', 'ZYGARDE', 'COSMOG', 'COSMOEM', 'SOLGALEO',
    'LUNALA', 'NECROZMA', 'ZACIAN', 'ZAMAZENTA', 'ETERNATUS', 'CALYREX', 'KORAIDON', 'MIRAIDON',
    'TERAPAGOS', 'MEW', 'CELEBI', 'JIRACHI', 'DEOXYS', 'MANAPHY', 'DARKRAI', 'SHAYMIN', 'ARCEUS',
    'VICTINI', 'KELDEO', 'MELOETTA', 'GENESECT', 'DIANCIE', 'HOOPA', 'VOLCANION', 'MAGEARNA',
    'MARSHADOW', 'ZERAORA', 'MELTAN', 'MELMETAL', 'ZARUDE', 'PECHARUNT',
}

# Pool A members the rebalance places as ordinary wild species (Cerulean Cave), so
# deferred_species() doesn't list them: the legendary paradoxes of CHALLENGE_RULES.md §2.3
# (Milestone 6 §3.4: with legends out of the wild they turned up in grass anyway).
POOL_A_EXTRA = {'RAGING_BOLT', 'GOUGING_FIRE', 'WALKING_WAKE', 'IRON_LEAVES', 'IRON_BOULDER', 'IRON_CROWN'}

# Never a replacement (still listed, so a wild slot holding one can be looked up).
NOT_PICKABLE = {'UNOWN'}

# Random abilities never rolled: bound to one species or form, or game-breaking.
BANNED_ABILITIES = {
    'NONE', 'WONDER_GUARD', 'MULTITYPE', 'RKS_SYSTEM', 'STANCE_CHANGE', 'SCHOOLING', 'BATTLE_BOND',
    'POWER_CONSTRUCT', 'SHIELDS_DOWN', 'DISGUISE', 'ICE_FACE', 'GULP_MISSILE', 'HUNGER_SWITCH',
    'ZERO_TO_HERO', 'COMMANDER', 'TERA_SHIFT', 'TERA_SHELL', 'TERAFORM_ZERO', 'ZEN_MODE', 'FORECAST',
    'FLOWER_GIFT', 'AS_ONE_GLASTRIER', 'AS_ONE_SPECTRIER', 'POISON_PUPPETEER', 'EMBODY_ASPECT',
    'EMBODY_ASPECT_2', 'EMBODY_ASPECT_3', 'EMBODY_ASPECT_4', 'ILLUSION', 'IMPOSTER', 'COMATOSE',
}

TYPES = ['NORMAL', 'FIGHTING', 'FLYING', 'POISON', 'GROUND', 'ROCK', 'BUG', 'GHOST', 'STEEL',
         'FAIRY', 'FIRE', 'WATER', 'GRASS', 'ELECTRIC', 'PSYCHIC', 'ICE', 'DRAGON', 'DARK']
TYPE_ALIASES = {'MYSTERY': 'FAIRY'}  # hg-engine: Fairy took type 9


def load_species_data():
    """SPECIES_X -> (bst, type1, type2) from data/Species.c."""
    data, cur, stats, types = {}, None, {}, None
    for line in open(HG / 'data' / 'Species.c', encoding='utf-8'):
        m = re.match(r'\s{4}\[SPECIES_(\w+)\] = \{', line)
        if m:
            if cur and len(stats) == 6 and types:
                data[cur] = (sum(stats.values()), *types)
            cur, stats, types = m.group(1), {}, None
            continue
        m = re.match(r'\s+\.(hp|attack|defense|spAttack|spDefense|speed) = (\d+),', line)
        if m and cur and m.group(1) not in stats:
            stats[m.group(1)] = int(m.group(2))
            continue
        m = re.match(r'\s+\.types = \{ TYPE_(\w+), TYPE_(\w+) \}', line)
        if m and cur and types is None:
            types = tuple(TYPES.index(TYPE_ALIASES.get(t, t)) for t in m.groups())
    if cur and len(stats) == 6 and types:
        data[cur] = (sum(stats.values()), *types)
    return data


def load_forms():
    """Form constant -> (base species, form number, battle-only)."""
    forms, base, n = {}, None, 0
    for line in open(HG / 'data' / 'PokeFormDataTbl.c', encoding='utf-8'):
        m = re.match(r'\s*\[SPECIES_(\w+)\] = \{', line)
        if m:
            base, n = m.group(1), 0
            continue
        m = re.match(r'\s*(NEEDS_REVERSION \| )?SPECIES_(\w+),', line)
        if m and base:
            n += 1
            forms.setdefault(m.group(2), (base, n, bool(m.group(1))))
    return forms


def load_abilities():
    out = []
    for line in open(HG / 'include' / 'constants' / 'ability.h', encoding='utf-8'):
        m = re.match(r'#define ABILITY_(\w+)\s+(\d+)', line)
        if m and m.group(1) not in BANNED_ABILITIES:
            out.append((int(m.group(2)), m.group(1)))
    return sorted(set(out))


def tier(bst):
    return 1 if bst < 340 else 2 if bst < 450 else 3 if bst < 540 else 4


def build():
    sp, _, baby, evo = plan.load_engine()
    fam = plan.families(sp, baby, evo)
    family_of = {}
    for i, root in enumerate(sorted(fam, key=sp.get)):
        for member in fam[root]:
            family_of[member] = i
    legends = plan.deferred_species(evo) | POOL_A_EXTRA
    stats = load_species_data()
    forms = load_forms()

    # A species is a base form when nothing evolves into it and it is its own baby form
    # (BabyMons.c maps every species to the lowest form of its family).
    targets = {t for lst in evo.values() for _, t in lst} | {n for n, b in baby.items() if b != n}

    safari_forms = []
    for _, _, m in plan.SAFARI:
        for lst in m.values():
            safari_forms += lst
    legend_forms = [n for n in legends if n in forms]

    entries, problems = {}, []

    def add(name, species_name, form):
        if name not in stats:
            problems.append(f'no species data for {name}')
            return
        bst, t1, t2 = stats[name]
        value = sp[species_name] | (form << 11)
        flags = 0x01 if species_name not in NOT_PICKABLE else 0
        if species_name not in targets:
            flags |= 0x02
        if name in legends or species_name in legends:
            flags |= 0x08 if species_name in POOL_B else 0x04
        entries[value] = (name, family_of.get(species_name, 0xFFFF), t1, t2, flags, tier(bst))

    for name in sp:
        add(name, name, 0)
    for name in sorted(set(safari_forms) | set(legend_forms)):
        if name not in forms:
            problems.append(f'unknown form {name}')
            continue
        base, n, battle_only = forms[name]
        if battle_only:
            problems.append(f'battle-only form {name} listed as wild')
            continue
        add(name, base, n)
    return [entries[v] + (v,) for v in sorted(entries)], load_abilities(), len(fam), problems


def render(rows, abilities, family_count):
    lines = [
        '// GENERATED by scripts/challenge_pool.py: do not edit.',
        '// Milestone 5 randomizer pool (docs/MILESTONE5_PLAN.md §4.5), Overlay 131.',
        '',
        '#include "config.h"',
        '#include "types.h"',
        '#include "coop_challenge.h"',
        '',
        f'const u16 gCoopPoolFamilyCount = {family_count};',
        f'const u16 gCoopPoolCount = {len(rows)};',
        '',
        '// Sorted by value (species | form << 11).',
        'const struct CoopPoolEntry gCoopPool[] = {',
    ]
    for name, family, t1, t2, flags, t, value in rows:
        lines.append(f'    {{ 0x{value:04X}, {family}, {t1}, {t2}, 0x{flags:02X}, {t} }}, // {name}')
    lines += [
        '};',
        '',
        f'const u16 gCoopAbilityPoolCount = {len(abilities)};',
        'const u16 gCoopAbilityPool[] = {',
    ]
    for value, name in abilities:
        lines.append(f'    {value}, // {name}')
    lines += ['};', '']
    return '\n'.join(lines)


def main():
    rows, abilities, family_count, problems = build()
    text = render(rows, abilities, family_count)
    pickable = sum(1 for r in rows if r[4] & 0x01 and not r[4] & 0x0C)
    pool_a = sum(1 for r in rows if r[4] & 0x04)
    pool_b = sum(1 for r in rows if r[4] & 0x08)
    print(f'{len(rows)} entries ({pickable} wild-eligible, Pool A {pool_a}, Pool B {pool_b}), '
          f'{family_count} families, {len(abilities)} abilities')
    for p in problems:
        print('problem:', p)
    if '--check' in sys.argv:
        current = OUT.read_text(encoding='utf-8') if OUT.exists() else ''
        if current != text:
            print(f'{OUT} is out of date: run scripts/challenge_pool.py')
            return 1
        return 1 if problems else 0
    OUT.write_text(text, encoding='utf-8')
    print(f'wrote {OUT.relative_to(ROOT)}')
    return 1 if problems else 0


if __name__ == '__main__':
    sys.exit(main())
