#!/usr/bin/env python3
"""Milestone 4 Phase 5: route and building trainers follow the new level curve.

Every trainer tied to a map (scripts/rebalance_trainer_maps.json, built from a read-only
dump of the vanilla map scripts and event data: docs/GAME_REBALANCE_UPDATE.md §18 E) gets
its levels moved into that map's band (docs/GAME_REBALANCE_UPDATE.md §3.4):
  - the band is the map's wild band (scripts/rebalance_dex_plan.py, through the encounter
    tables) plus 1-2 levels, or the explicit band below for gyms, buildings and towns;
  - within a map, trainers keep their vanilla order (their aces spread over the band), and
    each party keeps its internal level gaps.
Bosses (scripts/rebalance_bosses.py) and trainers without a map (rematches, unused, Battle
Frontier) are left alone. Kanto trainers also gain 2 levels per Kanto badge in the game
(src/field/coop_battle.c, Coop_ScaleBossParty), so their data sits at the Kanto base band.

  scripts/rebalance_trainers.py          # report
  scripts/rebalance_trainers.py --write  # rewrite data/Trainers.c
"""
import importlib.util
import json
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
HG = ROOT / 'hg-engine'


def load(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


enc = load('enc', ROOT / 'scripts' / 'rebalance_encounters.py')
bosses = load('bosses', ROOT / 'scripts' / 'rebalance_bosses.py')

# Maps without a wild band of their own: (lo, hi) of the trainers' aces.
EXTRA = {
    # Johto gyms: 2-5 levels under the leader's ace (§3.2).
    'T22GYM0101': (10, 12), 'T23GYM0102': (15, 17), 'T25GYM0101': (19, 22), 'T27GYM0101': (24, 27),
    'T24GYM0101': (30, 33), 'T26GYM0101': (34, 37), 'T28GYM0101': (38, 41), 'T28GYM0102': (38, 41),
    'T30GYM0101': (45, 48),
    # Kanto gyms, the Fighting Dojo, Blaine's Seafoam gym, the Power Plant: the Kanto base band.
    'T02GYM0101': (60, 63), 'T03GYM0101': (56, 60), 'T04GYM0101': (56, 60), 'T06GYM0101': (56, 60),
    'T07GYM0101': (56, 60), 'T08GYM0101': (56, 60), 'T11GYM0101': (56, 60), 'D11R0106': (56, 60),
    'T11R0101': (56, 60), 'R10R0101': (56, 60),
    # Story buildings, by when the story sends the player there.
    'D15R0101': (6, 10),                                     # Sprout Tower 1F
    'D23R0101': (37, 42), 'D23R0102': (37, 42), 'D23R0103': (37, 42), 'D23R0104': (37, 42),
    'D23R0105': (37, 42), 'D23R0106': (37, 42),             # Team Rocket HQ (Mahogany)
    'D27R0102': (28, 34), 'D27R0104': (28, 34), 'D27R0105': (28, 34), 'D27R0106': (28, 34),  # Olivine Lighthouse
    'D35R0102': (40, 45), 'D35R0103': (40, 45), 'D35R0104': (40, 45),  # Goldenrod Radio Tower (takeover)
    'D37R0102': (40, 45), 'D37R0104': (40, 45), 'D37R0105': (40, 45),  # Goldenrod Underground (takeover)
    'D38R0104': (34, 38),                                    # Mt. Mortar B1F, the Karate King
    'D45R0102': (46, 50),                                    # Tohjo Falls
    'P01R0303': (50, 54), 'P01R0304': (50, 54), 'P01R0305': (50, 54), 'P01R0306': (50, 54),
    'P01R0307': (50, 54),                                    # S.S. Aqua, on the way to Kanto
    'T21': (5, 6), 'T23': (17, 19), 'T24': (32, 34), 'T29': (36, 40),  # rivals / town trainers
    'T27R0501': (27, 31),                                    # the Kimono Girls
    'T26PC0101': (34, 37),
    'D25R0103': (24, 28),                                    # Union Cave B2F (by Surf)
    'T10R0101': (50, 53),                                    # Indigo Plateau lobby
}
# Trainer bands sit a little above the wild band.
WILD_PLUS = (1, 2)


def wild_bands():
    bands = {}
    for name, act, lv, m in enc.plan.A:
        lo, hi = enc.band(lv)
        for tab in enc.TABLES.get(name, []):
            bands.setdefault(tab.split('_')[0], (lo + WILD_PLUS[0], hi + WILD_PLUS[1]))
    return bands


def main():
    tmaps = json.load(open(ROOT / 'scripts' / 'rebalance_trainer_maps.json'))
    bands = wild_bands()
    bands.update(EXTRA)
    skip = {b['id'] for b in bosses.BOSSES}
    path = HG / 'data/Trainers.c'
    s = open(path).read()

    def block(tid):
        i = s.index(f'    [{tid}] = {{')
        j = s.index('\n    [', i + 5) if f'\n    [' in s[i + 5:] else len(s)
        return i, j

    parties, unbanded = {}, set()
    for tid, maps in tmaps.items():
        tid = int(tid)
        if tid in skip or tid >= bosses.FIRST_TEAM_ID:
            continue
        mp = next((m for m in maps if m in bands), None)
        if mp is None:
            unbanded.update(maps)
            continue
        i, j = block(tid)
        levels = [int(x) for x in re.findall(r'\.level = (\d+),', s[i:j])]
        if levels:
            parties[tid] = (mp, levels)
    by_map = {}
    for tid, (mp, levels) in parties.items():
        by_map.setdefault(mp, []).append((max(levels), tid))
    new_levels = {}
    for mp, lst in by_map.items():
        lo, hi = bands[mp]
        aces = [a for a, _ in lst]
        vmin, vmax = min(aces), max(aces)
        for ace, tid in lst:
            new_ace = round(lo + (ace - vmin) * (hi - lo) / (vmax - vmin)) if vmax > vmin else (lo + hi) // 2
            new_levels[tid] = [max(2, new_ace - (ace - l)) for l in parties[tid][1]]
    print(f'{len(new_levels)} trainers remapped over {len(by_map)} maps; maps without a band: {sorted(unbanded)}')
    if '--write' in sys.argv:
        for tid, levels in new_levels.items():
            i, j = block(tid)
            it = iter(levels)
            blk = re.sub(r'\.level = \d+,', lambda m: f'.level = {next(it)},', s[i:j])
            s = s[:i] + blk + s[j:]
        open(path, 'w').write(s)
        print('wrote data/Trainers.c')
    return 0


if __name__ == '__main__':
    sys.exit(main())
