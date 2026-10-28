#!/usr/bin/env python3
"""Milestone 4 Phase 3: the boss rosters (docs/GAME_REBALANCE_UPDATE.md §4).

Each boss has a solo roster (singles, written into its own trainer entry in data/Trainers.c)
and a team roster (6 Pokemon, doubles, a new trainer entry; slots 1-3 sit in enemy seat 1
and slots 4-6 in seat 3, so the leads are slots 1 and 4: §4.2). The co-op battle code swaps
the team entry in (src/field/coop_battle.c, gCoopTeamRosters, generated into
src/field/coop_team_rosters.c).

Checks before writing: every species, move, item and ability exists; every move is
implemented and learnable by the species (level-up at or below its level, TM, egg or tutor
moves, from data/learnsets/learnsets.json); every ability is one of the species' own
(Species.c abilities or its hidden ability).

Roster syntax, one Pokemon per string:
    "SPECIES level [@ITEM] ABILITY: MOVE, MOVE, MOVE, MOVE"
Form species use their form constant (NINETALES_ALOLAN, INDEEDEE_FEMALE, ...).

  scripts/rebalance_bosses.py          # validate only
  scripts/rebalance_bosses.py --write  # write data/Trainers.c and src/field/coop_team_rosters.c
"""
import json
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
HG = ROOT / 'hg-engine'

AI_BOSS = 'F_TRAINER_EXPERT_AI | F_PRIORITIZE_DAMAGE'
AI_WEATHER = AI_BOSS + ' | F_USE_WEATHER'

# id, name, ivs, ai, solo roster, team roster (None: built from the solo one, see team_from_solo)
BOSSES = []


def boss(tid, name, ivs, solo, team=None, ai=AI_BOSS, items=('NONE', 'NONE', 'NONE', 'NONE')):
    BOSSES.append(dict(id=tid, name=name, ivs=ivs, solo=solo, team=team, ai=ai, items=items))


# ---------------- Johto gyms (§4.3) ----------------
boss(20, 'Falkner', 50, [
    'WATTREL 12 VOLT_ABSORB: PECK, THUNDER_SHOCK, QUICK_ATTACK, GROWL',
    'CORVISQUIRE 13 KEEN_EYE: PECK, FURY_ATTACK, HONE_CLAWS, LEER',
    'GLIGAR 13 HYPER_CUTTER: MUD_SLAP, WING_ATTACK, POISON_STING, QUICK_ATTACK',
    'PIDGEOTTO 14 @BERRY_JUICE TANGLED_FEET: GUST, TACKLE, QUICK_ATTACK, ROOST',
], team=[
    'WATTREL 12 VOLT_ABSORB: PECK, THUNDER_SHOCK, QUICK_ATTACK, GROWL',
    'HOOTHOOT 12 INSOMNIA: PECK, CONFUSION, REFLECT, TACKLE',
    'GLIGAR 13 HYPER_CUTTER: MUD_SLAP, WING_ATTACK, POISON_STING, QUICK_ATTACK',
    'CORVISQUIRE 13 KEEN_EYE: PECK, FURY_ATTACK, HONE_CLAWS, LEER',
    'SPEAROW 12 KEEN_EYE: PECK, LEER, FURY_ATTACK, ASSURANCE',
    'PIDGEOTTO 14 @BERRY_JUICE TANGLED_FEET: GUST, TACKLE, QUICK_ATTACK, ROOST',
])
boss(21, 'Bugsy', 80, [
    'MASQUERAIN 17 INTIMIDATE: BUG_BITE, ICY_WIND, GUST, SCARY_FACE',
    'DWEBBLE 17 STURDY: SMACK_DOWN, BUG_BITE, STEALTH_ROCK, WITHDRAW',
    'YANMA 18 SPEED_BOOST: BUG_BITE, AIR_CUTTER, DOUBLE_TEAM, QUICK_ATTACK',
    'HERACROSS 18 GUTS: AERIAL_ACE, ARM_THRUST, BULLET_SEED, LEER',
    'SCYTHER 19 @SITRUS_BERRY TECHNICIAN: U_TURN, WING_ATTACK, QUICK_ATTACK, FOCUS_ENERGY',
], team=[
    'MASQUERAIN 17 INTIMIDATE: BUG_BITE, ICY_WIND, GUST, SCARY_FACE',
    'YANMA 18 SPEED_BOOST: BUG_BITE, AIR_CUTTER, DOUBLE_TEAM, QUICK_ATTACK',
    'ARIADOS 17 SWARM: POISON_STING, NIGHT_SHADE, BUG_BITE, STRING_SHOT',
    'DWEBBLE 17 STURDY: SMACK_DOWN, BUG_BITE, STEALTH_ROCK, WITHDRAW',
    'HERACROSS 18 GUTS: AERIAL_ACE, ARM_THRUST, BULLET_SEED, LEER',
    'SCYTHER 19 @SITRUS_BERRY TECHNICIAN: U_TURN, WING_ATTACK, QUICK_ATTACK, FOCUS_ENERGY',
])
boss(30, 'Whitney', 100, [
    'INDEEDEE_FEMALE 22 PSYCHIC_SURGE: PSYBEAM, DISARMING_VOICE, SWIFT, REFLECT',
    'WIGGLYTUFF 22 COMPETITIVE: DISARMING_VOICE, BODY_SLAM, DISABLE, CHARM',
    'GIRAFARIG 23 INNER_FOCUS: STOMP, PSYBEAM, ASSURANCE, THUNDER_WAVE',
    'BEWEAR 23 FLUFFY: PAYBACK, BABY_DOLL_EYES, BRUTAL_SWING, BIND',
    'MILTANK 24 @LUM_BERRY SCRAPPY: STOMP, ROLLOUT, ZEN_HEADBUTT, ATTRACT',
], team=[
    'INDEEDEE_FEMALE 22 PSYCHIC_SURGE: PSYBEAM, DISARMING_VOICE, HELPING_HAND, REFLECT',
    'GIRAFARIG 23 INNER_FOCUS: STOMP, PSYBEAM, ASSURANCE, THUNDER_WAVE',
    'CLEFAIRY 22 FRIEND_GUARD: DISARMING_VOICE, HELPING_HAND, ENCORE, METRONOME',
    'WIGGLYTUFF 22 COMPETITIVE: DISARMING_VOICE, BODY_SLAM, DISABLE, CHARM',
    'BEWEAR 23 FLUFFY: PAYBACK, BABY_DOLL_EYES, BRUTAL_SWING, BIND',
    'MILTANK 24 @LUM_BERRY SCRAPPY: STOMP, ROLLOUT, ZEN_HEADBUTT, ATTRACT',
])
boss(31, 'Morty', 120, [
    'SABLEYE 27 PRANKSTER: WILL_O_WISP, KNOCK_OFF, SHADOW_SNEAK, CONFUSE_RAY',
    'DRIFBLIM 27 @SITRUS_BERRY UNBURDEN: HEX, GUST, ICY_WIND, WILL_O_WISP',
    'LAMPENT 28 FLASH_FIRE: INCINERATE, HEX, NIGHT_SHADE, WILL_O_WISP',
    'MIMIKYU 28 DISGUISE: SHADOW_CLAW, PLAY_ROUGH, SHADOW_SNEAK, SWORDS_DANCE',
    'MISMAGIUS 28 LEVITATE: HEX, MYSTICAL_FIRE, DAZZLING_GLEAM, THUNDER_WAVE',
    'GENGAR 29 @SPELL_TAG CURSED_BODY: SHADOW_BALL, HEX, SUCKER_PUNCH, WILL_O_WISP',
])
boss(34, 'Chuck', 160, [
    'HITMONTOP 33 INTIMIDATE: FAKE_OUT, BRICK_BREAK, SUCKER_PUNCH, BULLDOZE',
    'HAWLUCHA 33 @SITRUS_BERRY UNBURDEN: ACROBATICS, BRICK_BREAK, ROOST, FEATHER_DANCE',
    'TOXICROAK 34 DRY_SKIN: POISON_JAB, DRAIN_PUNCH, SUCKER_PUNCH, ICE_PUNCH',
    'GALLADE 34 SHARPNESS: PSYCHO_CUT, SACRED_SWORD, NIGHT_SLASH, LEAF_BLADE',
    'HARIYAMA 34 THICK_FAT: FAKE_OUT, CLOSE_COMBAT, KNOCK_OFF, BULLET_PUNCH',
    'POLIWRATH 35 @LEFTOVERS WATER_ABSORB: WATERFALL, DRAIN_PUNCH, ICE_PUNCH, BULK_UP',
])
boss(33, 'Jasmine', 160, [
    'BRONZONG 37 LEVITATE: GYRO_BALL, ZEN_HEADBUTT, TRICK_ROOM, STEALTH_ROCK',
    'MAGNEZONE 37 STURDY: THUNDERBOLT, FLASH_CANNON, VOLT_SWITCH, TRI_ATTACK',
    'TINKATON 38 MOLD_BREAKER: GIGATON_HAMMER, PLAY_ROUGH, ICE_HAMMER, KNOCK_OFF',
    'EMPOLEON 38 COMPETITIVE: SCALD, FLASH_CANNON, ICE_BEAM, AQUA_JET',
    'SCIZOR 38 TECHNICIAN: BULLET_PUNCH, U_TURN, DUAL_WINGBEAT, SWORDS_DANCE',
    'STEELIX 39 @PASSHO_BERRY SHEER_FORCE: IRON_HEAD, EARTHQUAKE, ROCK_SLIDE, CRUNCH',
])
boss(32, 'Pryce', 200, [
    'NINETALES_ALOLAN 41 @LIGHT_CLAY SNOW_WARNING: AURORA_VEIL, BLIZZARD, MOONBLAST, ICE_BEAM',
    'CETITAN 41 SLUSH_RUSH: ICICLE_CRASH, LIQUIDATION, HIGH_HORSEPOWER, ICE_SHARD',
    'ROTOM_FROST 42 LEVITATE: BLIZZARD, THUNDERBOLT, VOLT_SWITCH, WILL_O_WISP',
    'WEAVILE 42 PRESSURE: TRIPLE_AXEL, THROAT_CHOP, LOW_KICK, FAKE_OUT',
    'LAPRAS 42 WATER_ABSORB: FREEZE_DRY, SURF, PSYCHIC, CONFUSE_RAY',
    'MAMOSWINE 43 @NEVER_MELT_ICE THICK_FAT: ICICLE_CRASH, EARTHQUAKE, ROCK_SLIDE, ICE_SHARD',
], ai=AI_WEATHER)
boss(35, 'Clair', 220, [
    'PELIPPER 48 @DAMP_ROCK DRIZZLE: HURRICANE, SCALD, TAILWIND, U_TURN',
    'DRAGALGE 48 ADAPTABILITY: SLUDGE_WAVE, DRAGON_PULSE, THUNDERBOLT, FLIP_TURN',
    'GARCHOMP 49 ROUGH_SKIN: EARTHQUAKE, DRAGON_CLAW, ROCK_SLIDE, POISON_JAB',
    'GYARADOS 49 INTIMIDATE: WATERFALL, CRUNCH, ICE_FANG, DRAGON_DANCE',
    'ARCHALUDON 49 @ASSAULT_VEST STAMINA: ELECTRO_SHOT, FLASH_CANNON, DRAGON_PULSE, BODY_PRESS',
    'KINGDRA 50 @SCOPE_LENS SWIFT_SWIM: MUDDY_WATER, DRAGON_PULSE, ICE_BEAM, FLASH_CANNON',
], ai=AI_WEATHER)

# ---------------- Elite Four and Lance (§4.4) ----------------
def e4(rid1, rid2, name, r1, r2, ai=AI_BOSS):
    boss(rid1, name, 230, r1, ai=ai)
    boss(rid2, name, 255, r2, ai=ai)


def lv(roster, add):
    out = []
    for m in roster:
        head, moves = m.split(':', 1)
        parts = head.split()
        parts[1] = str(int(parts[1]) + add)
        out.append(' '.join(parts) + ':' + moves)
    return out


WILL = [
    'INDEEDEE 53 @TERRAIN_EXTENDER PSYCHIC_SURGE: PSYCHIC, SHADOW_BALL, HYPER_VOICE, CALM_MIND',
    'ARMAROUGE 53 FLASH_FIRE: EXPANDING_FORCE, FLAMETHROWER, ENERGY_BALL, AURA_SPHERE',
    'SLOWBRO 54 REGENERATOR: SCALD, PSYCHIC, ICE_BEAM, SLACK_OFF',
    'BRONZONG 54 LEVITATE: GYRO_BALL, BODY_PRESS, TRICK_ROOM, HYPNOSIS',
    'XATU 54 MAGIC_BOUNCE: TAILWIND, AIR_SLASH, HEAT_WAVE, DAZZLING_GLEAM',
    'GARDEVOIR 55 @LIFE_ORB TRACE: EXPANDING_FORCE, MOONBLAST, MYSTICAL_FIRE, SHADOW_BALL',
]
KOGA = [
    'GLIMMORA 55 TOXIC_DEBRIS: SLUDGE_WAVE, POWER_GEM, EARTH_POWER, MORTAL_SPIN',
    'CROBAT 55 INNER_FOCUS: TAILWIND, BRAVE_BIRD, CROSS_POISON, U_TURN',
    'OVERQWIL 56 INTIMIDATE: BARB_BARRAGE, CRUNCH, LIQUIDATION, SWORDS_DANCE',
    'MUK_ALOLAN 56 POISON_TOUCH: KNOCK_OFF, GUNK_SHOT, DRAIN_PUNCH, ICE_PUNCH',
    'VENOMOTH 56 TINTED_LENS: BUG_BUZZ, SLUDGE_BOMB, SLEEP_POWDER, QUIVER_DANCE',
    'DRAPION 57 @BLACK_SLUDGE BATTLE_ARMOR: CROSS_POISON, KNOCK_OFF, EARTHQUAKE, SWORDS_DANCE',
]
BRUNO = [
    'HITMONTOP 56 INTIMIDATE: FAKE_OUT, CLOSE_COMBAT, SUCKER_PUNCH, WIDE_GUARD',
    'ANNIHILAPE 56 DEFIANT: DRAIN_PUNCH, ICE_PUNCH, BULK_UP, CLOSE_COMBAT',
    'CONKELDURR 57 IRON_FIST: MACH_PUNCH, DRAIN_PUNCH, THUNDER_PUNCH, KNOCK_OFF',
    'KROOKODILE 57 MOXIE: EARTHQUAKE, ROCK_SLIDE, CRUNCH, CLOSE_COMBAT',
    'MACHAMP 57 NO_GUARD: DYNAMIC_PUNCH, STONE_EDGE, BULLET_PUNCH, KNOCK_OFF',
    'LUCARIO 58 @LIFE_ORB INNER_FOCUS: CLOSE_COMBAT, METEOR_MASH, EXTREME_SPEED, BULLET_PUNCH',
]
KAREN = [
    'GRIMMSNARL 57 @LIGHT_CLAY PRANKSTER: REFLECT, LIGHT_SCREEN, SPIRIT_BREAK, PARTING_SHOT',
    'HOUNDOOM 57 FLASH_FIRE: HEAT_WAVE, DARK_PULSE, SLUDGE_BOMB, NASTY_PLOT',
    'UMBREON 58 SYNCHRONIZE: FOUL_PLAY, SNARL, MOONLIGHT, YAWN',
    'HONCHKROW 58 SUPER_LUCK: BRAVE_BIRD, NIGHT_SLASH, SUCKER_PUNCH, HEAT_WAVE',
    'WEAVILE 58 PRESSURE: ICICLE_CRASH, KNOCK_OFF, LOW_KICK, ICE_SHARD',
    'KINGAMBIT 59 @BLACK_GLASSES SUPREME_OVERLORD: KOWTOW_CLEAVE, IRON_HEAD, SUCKER_PUNCH, SWORDS_DANCE',
]
LANCE = [
    'AERODACTYL 59 UNNERVE: ROCK_SLIDE, TAILWIND, DUAL_WINGBEAT, AQUA_TAIL',
    'GARCHOMP 60 ROUGH_SKIN: EARTHQUAKE, DRAGON_CLAW, ROCK_SLIDE, POISON_JAB',
    'GYARADOS 60 INTIMIDATE: WATERFALL, ICE_FANG, EARTHQUAKE, DRAGON_DANCE',
    'CHARIZARD 60 SOLAR_POWER: HEAT_WAVE, AIR_SLASH, SCORCHING_SANDS, DRAGON_PULSE',
    'GOODRA_HISUIAN 60 SAP_SIPPER: HEAVY_SLAM, DRACO_METEOR, FLAMETHROWER, THUNDERBOLT',
    'DRAGONITE 61 @LUM_BERRY MULTISCALE: EXTREME_SPEED, OUTRAGE, EARTHQUAKE, DRAGON_DANCE',
]


def mega(roster, i, line):
    out = list(roster)
    out[i] = line
    return out


e4(245, 702, 'Will', WILL, mega(lv(WILL, 27), 5, 'GARDEVOIR 82 @GARDEVOIRITE TRACE: HYPER_VOICE, MOONBLAST, MYSTICAL_FIRE, SHADOW_BALL'))
e4(247, 703, 'Koga', KOGA, mega(lv(KOGA, 27), 5, 'GENGAR 84 @GENGARITE CURSED_BODY: SHADOW_BALL, SLUDGE_WAVE, DAZZLING_GLEAM, WILL_O_WISP'))
e4(418, 704, 'Bruno', BRUNO, mega(lv(BRUNO, 27), 5, 'LUCARIO 85 @LUCARIONITE INNER_FOCUS: CLOSE_COMBAT, METEOR_MASH, EXTREME_SPEED, BULLET_PUNCH'))
e4(246, 705, 'Karen', KAREN, mega(lv(KAREN, 27), 3, 'ABSOL 85 @ABSOLITE PRESSURE: KNOCK_OFF, PLAY_ROUGH, SUCKER_PUNCH, SWORDS_DANCE'))
e4(244, 701, 'Lance', LANCE, mega(lv(LANCE, 25), 5, 'SALAMENCE 86 @SALAMENCITE INTIMIDATE: DOUBLE_EDGE, EARTHQUAKE, ROOST, DRAGON_DANCE'))

# ---------------- Kanto (§4.5): levels at 0 Kanto badges; the game adds 2 per Kanto badge (§3.3) ----------------
boss(255, 'Lt. Surge', 230, [
    'PINCURCHIN 61 ELECTRIC_SURGE: ZING_ZAP, SPIKES, SUCKER_PUNCH, ACUPRESSURE',
    'ROTOM_WASH 61 LEVITATE: HYDRO_PUMP, THUNDERBOLT, VOLT_SWITCH, WILL_O_WISP',
    'RAICHU_ALOLAN 62 SURGE_SURFER: THUNDERBOLT, PSYCHIC, FOCUS_BLAST, NASTY_PLOT',
    'KILOWATTREL 62 VOLT_ABSORB: HURRICANE, THUNDERBOLT, TAILWIND, VOLT_SWITCH',
    'ELECTIVIRE 62 MOTOR_DRIVE: WILD_CHARGE, CROSS_CHOP, ICE_PUNCH, EARTHQUAKE',
    'RAICHU 63 @RAICHUNITE_X LIGHTNING_ROD: WILD_CHARGE, BRICK_BREAK, KNOCK_OFF, NASTY_PLOT',
], ai=AI_WEATHER)
boss(258, 'Sabrina', 230, [
    'INDEEDEE_FEMALE 61 PSYCHIC_SURGE: PSYCHIC, DAZZLING_GLEAM, FOLLOW_ME, CALM_MIND',
    'ESPEON 61 MAGIC_BOUNCE: PSYCHIC, DAZZLING_GLEAM, SHADOW_BALL, MORNING_SUN',
    'HATTERENE 62 MAGIC_BOUNCE: PSYCHIC, DAZZLING_GLEAM, MYSTICAL_FIRE, TRICK_ROOM',
    'MALAMAR 62 CONTRARY: SUPERPOWER, PSYCHO_CUT, NIGHT_SLASH, KNOCK_OFF',
    'METAGROSS 62 CLEAR_BODY: METEOR_MASH, ZEN_HEADBUTT, EARTHQUAKE, BULLET_PUNCH',
    'ALAKAZAM 63 @ALAKAZITE SYNCHRONIZE: PSYCHIC, SHADOW_BALL, FOCUS_BLAST, DAZZLING_GLEAM',
])
boss(254, 'Misty', 230, [
    'POLITOED 61 @DAMP_ROCK DRIZZLE: SCALD, ICE_BEAM, HYPNOSIS, ENCORE',
    'BARRASKEWDA 61 SWIFT_SWIM: LIQUIDATION, CLOSE_COMBAT, PSYCHIC_FANGS, AQUA_JET',
    'LANTURN 62 VOLT_ABSORB: SCALD, THUNDERBOLT, ICE_BEAM, VOLT_SWITCH',
    'PALAFIN 62 ZERO_TO_HERO: JET_PUNCH, WAVE_CRASH, CLOSE_COMBAT, BULK_UP',
    'GYARADOS 62 INTIMIDATE: WATERFALL, CRUNCH, EARTHQUAKE, DRAGON_DANCE',
    'STARMIE 63 @STARMINITE ANALYTIC: HYDRO_PUMP, PSYCHIC, ICE_BEAM, THUNDERBOLT',
], ai=AI_WEATHER)
boss(256, 'Erika', 230, [
    'RILLABOOM 61 GRASSY_SURGE: GRASSY_GLIDE, WOOD_HAMMER, KNOCK_OFF, U_TURN',
    'LILLIGANT_HISUIAN 61 HUSTLE: CLOSE_COMBAT, LEAF_BLADE, ICE_SPINNER, VICTORY_DANCE',
    'BRAMBLEGHAST 62 WIND_RIDER: POWER_WHIP, PHANTOM_FORCE, GIGA_DRAIN, STRENGTH_SAP',
    'LUDICOLO 62 OWN_TEMPO: GIGA_DRAIN, SURF, ICE_BEAM, FAKE_OUT',
    'SCOVILLAIN 62 CHLOROPHYLL: FLAMETHROWER, ENERGY_BALL, ZEN_HEADBUTT, CRUNCH',
    'VENUSAUR 63 @VENUSAURITE OVERGROW: SLUDGE_BOMB, GIGA_DRAIN, EARTH_POWER, SYNTHESIS',
])
boss(257, 'Janine', 230, [
    'WEEZING_GALARIAN 61 LEVITATE: STRANGE_STEAM, SLUDGE_BOMB, WILL_O_WISP, DEFOG',
    'SNEASLER 61 UNBURDEN: DIRE_CLAW, CLOSE_COMBAT, SHADOW_CLAW, SWORDS_DANCE',
    'CLODSIRE 62 WATER_ABSORB: POISON_JAB, EARTHQUAKE, RECOVER, TOXIC',
    'SALAZZLE 62 CORROSION: SLUDGE_WAVE, FLAMETHROWER, NASTY_PLOT, TOXIC',
    'CROBAT 62 INNER_FOCUS: BRAVE_BIRD, CROSS_POISON, U_TURN, TAILWIND',
    'BEEDRILL 63 @BEEDRILLITE SWARM: U_TURN, POISON_JAB, DRILL_RUN, KNOCK_OFF',
])
boss(253, 'Brock', 230, [
    'GIGALITH 61 SAND_STREAM: STONE_EDGE, EARTHQUAKE, HEAVY_SLAM, STEALTH_ROCK',
    'EXCADRILL 61 SAND_RUSH: EARTHQUAKE, IRON_HEAD, ROCK_SLIDE, SWORDS_DANCE',
    'ARCANINE_HISUIAN 62 INTIMIDATE: STONE_EDGE, FLARE_BLITZ, EXTREME_SPEED, WILD_CHARGE',
    'GARGANACL 62 PURIFYING_SALT: IRON_DEFENSE, STONE_EDGE, BODY_PRESS, RECOVER',
    'CRADILY 62 STORM_DRAIN: GIGA_DRAIN, STONE_EDGE, SLUDGE_BOMB, RECOVER',
    'AERODACTYL 63 @AERODACTYLITE ROCK_HEAD: ROCK_SLIDE, DUAL_WINGBEAT, AQUA_TAIL, FIRE_FANG',
], ai=AI_WEATHER)
boss(259, 'Blaine', 230, [
    'TORKOAL 61 DROUGHT: LAVA_PLUME, SOLAR_BEAM, EARTH_POWER, STEALTH_ROCK',
    'CERULEDGE 61 WEAK_ARMOR: BITTER_BLADE, POLTERGEIST, SHADOW_SNEAK, SWORDS_DANCE',
    'ROTOM_HEAT 62 LEVITATE: OVERHEAT, THUNDERBOLT, VOLT_SWITCH, WILL_O_WISP',
    'VOLCARONA 62 FLAME_BODY: FIERY_DANCE, BUG_BUZZ, GIGA_DRAIN, QUIVER_DANCE',
    'MAGMORTAR 62 VITAL_SPIRIT: FLAMETHROWER, THUNDERBOLT, FOCUS_BLAST, PSYCHIC',
    'HOUNDOOM 63 @HOUNDOOMINITE FLASH_FIRE: HEAT_WAVE, DARK_PULSE, SOLAR_BEAM, SLUDGE_BOMB',
], ai=AI_WEATHER)
boss(261, 'Blue', 255, [
    'ARCANINE 76 INTIMIDATE: FLARE_BLITZ, EXTREME_SPEED, WILD_CHARGE, CLOSE_COMBAT',
    'TYRANITAR 76 SAND_STREAM: STONE_EDGE, CRUNCH, EARTHQUAKE, ICE_PUNCH',
    'EXEGGUTOR_ALOLAN 77 HARVEST: DRACO_METEOR, FLAMETHROWER, GIGA_DRAIN, TRICK_ROOM',
    'ALAKAZAM 77 MAGIC_GUARD: PSYCHIC, SHADOW_BALL, FOCUS_BLAST, DAZZLING_GLEAM',
    'GYARADOS 77 INTIMIDATE: WATERFALL, CRUNCH, EARTHQUAKE, DRAGON_DANCE',
    'PIDGEOT 78 @PIDGEOTITE KEEN_EYE: HURRICANE, HEAT_WAVE, TAILWIND, U_TURN',
], ai=AI_WEATHER)
boss(260, 'Red', 255, [
    'PIKACHU 90 @LIGHT_BALL LIGHTNING_ROD: VOLT_TACKLE, FAKE_OUT, QUICK_ATTACK, SURF',
    'CHARIZARD 90 @CHARIZARDITE_Y BLAZE: HEAT_WAVE, AIR_SLASH, SOLAR_BEAM, SCORCHING_SANDS',
    'VENUSAUR 88 @LIFE_ORB CHLOROPHYLL: GIGA_DRAIN, SLUDGE_BOMB, WEATHER_BALL, SLEEP_POWDER',
    'ESPEON 88 @FOCUS_SASH MAGIC_BOUNCE: PSYCHIC, DAZZLING_GLEAM, SHADOW_BALL, MORNING_SUN',
    'SNORLAX 89 @LEFTOVERS THICK_FAT: BODY_SLAM, HIGH_HORSEPOWER, CRUNCH, HEAVY_SLAM',
    'BLASTOISE 89 @WHITE_HERB TORRENT: MUDDY_WATER, ICE_BEAM, AURA_SPHERE, SHELL_SMASH',
], ai=AI_WEATHER)

# Badge-scaled bosses (§3.3): trainer id -> (expected badges, levels per badge, mode).
# 'all' counts all sixteen badges, 'kanto' only Kanto's; the C side clamps to +-2 steps
# for 'all' and to 0..7 Kanto badges for 'kanto'.
SCALED = {34: (4, 4, 'all'), 33: (5, 4, 'all'), 32: (6, 4, 'all'),
          255: (0, 2, 'kanto'), 258: (0, 2, 'kanto'), 254: (0, 2, 'kanto'), 256: (0, 2, 'kanto'),
          257: (0, 2, 'kanto'), 253: (0, 2, 'kanto'), 259: (0, 2, 'kanto')}

FIRST_TEAM_ID = 738


# ---------------- data loading and checks ----------------

def consts(path, prefix):
    out = {}
    for line in open(HG / path):
        m = re.match(rf'#define {prefix}(\w+)\s+\(?(\w+)', line)
        if m:
            out[m.group(1)] = m.group(2)
    return out


SPECIES = consts('include/constants/species.h', 'SPECIES_')
MOVES = consts('include/constants/moves.h', 'MOVE_')
ITEMS = consts('include/constants/item.h', 'ITEM_')
ABILITIES = consts('include/constants/ability.h', 'ABILITY_')
LEARN = json.load(open(HG / 'data/learnsets/learnsets.json'))


def unimplemented_moves():
    out = set()
    cur = None
    for line in open(HG / 'data/Moves.c'):
        m = re.match(r'\s*\[MOVE_(\w+)\]\s*=', line)
        if m:
            cur = m.group(1)
        elif cur and '.flags' in line and 'FLAG_UNUSABLE_UNIMPLEMENTED' in line:
            out.add(cur)
    return out


UNIMPL = unimplemented_moves()


def species_abilities():
    out, cur = {}, None
    for line in open(HG / 'data/Species.c'):
        m = re.match(r'\s*\[SPECIES_(\w+)\]\s*=', line)
        if m:
            cur = m.group(1)
        m = re.search(r'\.abilities = \{ ABILITY_(\w+), ABILITY_(\w+) \}', line)
        if m and cur:
            out.setdefault(cur, set()).update(m.groups())
    for line in open(HG / 'data/HiddenAbilityTable.c'):
        m = re.match(r'\s*\[SPECIES_(\w+)\s*\]\s*=\s*ABILITY_(\w+)', line)
        if m:
            out.setdefault(m.group(1), set()).add(m.group(2))
    return out


ABIL = species_abilities()

spec = __import__('importlib.util').util.spec_from_file_location('enc', ROOT / 'scripts' / 'rebalance_encounters.py')
enc = __import__('importlib.util').util.module_from_spec(spec)
spec.loader.exec_module(enc)


def parse(line):
    m = re.match(r'(\w+) (\d+)(?: @(\w+))? (\w+): (.*)$', line)
    if not m:
        raise ValueError(f'bad roster line: {line}')
    sp, lvl, item, ab, moves = m.groups()
    return dict(species=sp, level=int(lvl), item=item or 'NONE', ability=ab, moves=[x.strip() for x in moves.split(',')])


# Moves an appliance Rotom gets with its form (hg-engine has no learnset for the forms).
FORM_MOVES = {('ROTOM_HEAT', 'OVERHEAT'), ('ROTOM_WASH', 'HYDRO_PUMP'), ('ROTOM_FROST', 'BLIZZARD'),
              ('ROTOM_FAN', 'AIR_SLASH'), ('ROTOM_MOW', 'LEAF_STORM')}


def legal(mon):
    errs = []
    sp = mon['species']
    if sp not in SPECIES:
        return [f'unknown species {sp}']
    learn = LEARN.get('SPECIES_' + sp) or LEARN.get('SPECIES_' + enc.FORMS.get(sp, (sp, 0))[0], {})
    lvlmoves = {x['Move'][5:] for x in learn.get('LevelMoves', []) if x['Level'] <= mon['level']}
    other = {x[5:] for k in ('MachineMoves', 'EggMoves', 'TutorMoves') for x in learn.get(k, [])}
    for mv in mon['moves']:
        if mv not in MOVES:
            errs.append(f'unknown move {mv}')
        elif mv in UNIMPL:
            errs.append(f'unimplemented move {mv}')
        elif mv not in lvlmoves and mv not in other and (sp, mv) not in FORM_MOVES:
            errs.append(f'{sp} cannot learn {mv}')
    if mon['item'] != 'NONE' and mon['item'] not in ITEMS:
        errs.append(f'unknown item {mon["item"]}')
    if mon['ability'] not in ABILITIES:
        errs.append(f'unknown ability {mon["ability"]}')
    else:
        own = ABIL.get(sp) or ABIL.get(enc.FORMS.get(sp, (sp, 0))[0], set())
        if mon['ability'] not in own:
            errs.append(f'{sp} cannot have {mon["ability"]} (has {sorted(own)})')
    return errs


def team_from_solo(solo):
    """Lead pair (solo slots 1 and 2) into team slots 1 and 4; 3, 4 into 2, 5; 5, 6 into 3, 6."""
    s = solo + solo[:6 - len(solo)] if len(solo) < 6 else solo
    return [s[0], s[2], s[4], s[1], s[3], s[5]]


def check():
    errs = []
    for b in BOSSES:
        team = b['team'] or team_from_solo(b['solo'])
        if len(team) != 6:
            errs.append(f'{b["name"]} ({b["id"]}): team roster has {len(team)} Pokemon')
        stones = [m for m in team if parse(m)['item'].endswith(('ITE', 'ITE_X', 'ITE_Y', 'ITE_Z'))]
        if len(stones) > 1:
            errs.append(f'{b["name"]}: more than one Mega Stone in the team roster')
        for line in b['solo'] + team:
            for e in legal(parse(line)):
                errs.append(f'{b["name"]} ({b["id"]}): {e}')
    return sorted(set(errs))


# ---------------- writing ----------------

def mon_c(mon, ivs):
    sp = mon['species']
    spc = enc.species(sp)
    moves = ', '.join(f'MOVE_{x}' for x in (mon['moves'] + ['NONE'] * 4)[:4])
    return f'''            {{
                .ivs = {ivs},
                .abilitySlot = TRAINER_POKEMON_ABILITY_1,
                .level = {mon['level']},
                .species = {spc},
                .item = ITEM_{mon['item']},
                .moves = {{ {moves} }},
                .ability = ABILITY_{mon['ability']},
                .ballSeal = 0,
            }},'''


def entry_body(b, roster, tclass, text):
    party = '\n'.join(mon_c(parse(m), b['ivs']) for m in roster)
    items = ', '.join(f'ITEM_{i}' for i in b['items'])
    return f'''        .data = {{
            .trainerType = TRAINER_DATA_TYPE_MOVES | TRAINER_DATA_TYPE_ITEMS | TRAINER_DATA_TYPE_ABILITY,
            .trainerClass = {tclass},
            .items = {{ {items} }},
            .aiFlags = {b['ai']},
            .battleType = SINGLE_BATTLE,
        }},
        .party = {{
{party}
        }},
{text}'''


def write():
    path = HG / 'data/Trainers.c'
    s = open(path).read()
    # Rerunnable: drop the team entries (and their text order lines) a previous run wrote.
    s = re.sub(r'\n    \[\d+\] = \{ // team roster of .*?\n    \},\n', '\n', s, flags=re.S)
    tids = {str(FIRST_TEAM_ID + k) for k in range(len(BOSSES))}
    s = re.sub(r'(const u16 sTrainerTextOrder\[\] = \{)(.*?)(\n\};)',
               lambda m: m.group(1) + ''.join(l for l in re.findall(r'\n[^\n]*', m.group(2))
                                              if l.strip().rstrip(',') not in tids) + m.group(3), s, flags=re.S)
    team_entries = []
    team_ids = []
    for k, b in enumerate(BOSSES):
        i = s.index(f'    [{b["id"]}] = {{')
        j = s.index('\n    [', i + 5)
        blk = s[i:j]
        tclass = re.search(r'\.trainerClass = (\w+)', blk).group(1)
        ti = blk.index('        .text = {')
        text = blk[ti:blk.rindex('    },')].rstrip() + '\n'
        new = f'    [{b["id"]}] = {{\n        .name = "{b["name"]}",\n' + entry_body(b, b['solo'], tclass, text) + '    },\n'
        s = s[:i] + new + s[j:]
        tid = FIRST_TEAM_ID + k
        team_ids.append((b['id'], tid))
        team = b['team'] or team_from_solo(b['solo'])
        team_entries.append(f'    [{tid}] = {{ // team roster of {b["name"]} ({b["id"]}), docs/GAME_REBALANCE_UPDATE.md §4.2\n'
                            f'        .name = "{b["name"]}",\n' + entry_body(b, team, tclass, text) + '    },\n')
    # Team entries go after the last trainer, and into the text order (their battle texts).
    end = s.index('\n};\n', 0, s.index('const u16 sTrainerTextOrder[]'))
    s = s[:end].rstrip() + '\n\n' + '\n'.join(team_entries).rstrip() + s[end:]
    order_end = s.index('\n};\n', s.index('const u16 sTrainerTextOrder[]'))
    s = s[:order_end].rstrip() + '\n' + '\n'.join(f'    {t},' for _, t in team_ids) + s[order_end:]
    open(path, 'w').write(s)
    rows = '\n'.join(f'    {{ {a}, {t} }},' for a, t in team_ids)
    scaled = '\n'.join(f'    {{ {tid}, {exp}, {step}, {1 if mode == "kanto" else 0} }},' for tid, (exp, step, mode) in SCALED.items())
    c = f'''#include "types.h"
#include "coop_boss.h"

// Generated by scripts/rebalance_bosses.py (docs/GAME_REBALANCE_UPDATE.md §3.3, §4.2): do not edit.

// Each team boss's roster for a team: its own trainer entry, 6 Pokemon,
// slots 1-3 in enemy seat 1 and 4-6 in seat 3.
const struct CoopTeamRoster gCoopTeamRosters[] = {{
{rows}
}};
const u32 gCoopTeamRosterCount = NELEMS(gCoopTeamRosters);

// Badge-scaled bosses: trainer, expected badges, levels per badge, Kanto badges only.
const struct CoopScaledBoss gCoopScaledBosses[] = {{
{scaled}
}};
const u32 gCoopScaledBossCount = NELEMS(gCoopScaledBosses);
'''
    open(HG / 'src/field/coop_team_rosters.c', 'w').write(c)


def main():
    errs = check()
    for e in errs:
        print(e)
    print(f'{len(BOSSES)} bosses, {len(errs)} problems')
    if '--write' in sys.argv:
        if errs:
            print('not writing: fix the problems first')
            return 1
        write()
        print('wrote data/Trainers.c, src/field/coop_team_rosters.c')
    return 1 if errs else 0


if __name__ == '__main__':
    sys.exit(main())
