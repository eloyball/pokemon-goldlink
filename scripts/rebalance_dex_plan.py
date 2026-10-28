#!/usr/bin/env python3
"""Milestone 4 Pokédex placement plan: data, coverage check and appendix generator.

The placement tables below are the plan's source of truth for where every species is
obtained (docs/GAME_REBALANCE_UPDATE.md). The script reads hg-engine's species list,
evolution table and baby table, then checks that:
  - every one of the 1,025 species is placed, or reachable from a placed one by an
    evolution the ROM can perform (after the planned evolution fixes) or by breeding;
  - every regional and wild alternate form is placed in the Safari Zone, and nothing else is.
It exits 1 when a check fails. With --write it regenerates docs/REBALANCE_DEX_PLAN.md. With
--built it checks hg-engine's built wild tables (Encounters.c, Headbutt.c, SafariEncounters.c) plus
the plan's gifts and statics, instead of the plan's tables (Milestone 4 Phase 2).

Names are hg-engine SPECIES_ constants without the prefix; regional forms use the
form-species constants (SPECIES_RATTATA_ALOLAN...), which the build packs with
MON_WITH_FORM when the tables are written into data/*.c.
"""
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
HG = ROOT / 'hg-engine'
OUT = ROOT / 'docs' / 'REBALANCE_DEX_PLAN.md'

A = []
def area(name, act, lv, **m):
    A.append((name, act, lv, m))

# ---------------- Act 1: New Bark to Azalea (badges 0-1) ----------------
area('Route 29', 1, '2-5',
     day=['SENTRET', 'PIDGEY', 'ROOKIDEE', 'LECHONK', 'ZIGZAGOON', 'BIDOOF'],
     night=['HOOTHOOT', 'RATTATA', 'NICKIT', 'KRICKETOT'],
     rare=['TANDEMAUS', 'YUNGOOS'],
     headbutt=['PINECO', 'SPEAROW', 'PIKIPEK', 'SKWOVET'])
area('Route 46 (south)', 1, '3-6',
     day=['GEODUDE', 'SPEAROW', 'PHANPY', 'NIDORAN_F', 'NIDORAN_M'],
     night=['RATTATA', 'POOCHYENA', 'TEDDIURSA'],
     rare=['ROCKRUFF', 'GLIGAR'],
     headbutt=['AIPOM', 'BURMY', 'COMBEE'])
area('Route 30', 1, '4-7',
     day=['CATERPIE', 'WEEDLE', 'LEDYBA', 'WURMPLE', 'SCATTERBUG', 'PIDOVE'],
     night=['SPINARAK', 'HOOTHOOT', 'PURRLOIN', 'KRICKETOT'],
     rare=['RALTS', 'NINCADA'], starter=['CHIKORITA'],
     surf=['POLIWAG', 'MARILL', 'SURSKIT'], fish=['MAGIKARP', 'POLIWAG', 'GOLDEEN', 'ARROKUDA'],
     headbutt=['EXEGGCUTE', 'CHERUBI', 'PINECO'])
area('Route 31', 1, '5-8',
     day=['BELLSPROUT', 'LOTAD', 'SEEDOT', 'BLIPBUG', 'FLETCHLING', 'PATRAT'],
     night=['SPINARAK', 'ODDISH', 'POOCHYENA', 'NICKIT'],
     rare=['WATTREL', 'TAILLOW'], starter=['CYNDAQUIL'],
     surf=['POLIWAG', 'WOOPER'], fish=['MAGIKARP', 'POLIWAG', 'BARBOACH'])
area('Dark Cave (Route 31 side)', 1, '4-9',
     day=['ZUBAT', 'GEODUDE', 'DUNSPARCE', 'WOOBAT', 'ROGGENROLA', 'WHISMUR'],
     night=['ZUBAT', 'NOIBAT', 'WOBBUFFET'], rare=['NACLI', 'TEDDIURSA'],
     smash=['NOSEPASS', 'DWEBBLE'], surf=['WOOPER'], fish=['MAGIKARP', 'BARBOACH', 'GOLDEEN'])
area('Sprout Tower', 1, '5-9',
     day=['RATTATA', 'BELLSPROUT', 'MEDITITE', 'TADBULB'],
     night=['GASTLY', 'MISDREAVUS', 'LITWICK'], rare=['CHINGLING', 'SINISTEA'])
area('Route 32', 1, '6-11',
     day=['WOOPER', 'MAREEP', 'HOPPIP', 'EKANS', 'PAWMI', 'SHINX'],
     night=['MURKROW', 'ZUBAT', 'WOOPER', 'GULPIN'],
     rare=['FLITTLE', 'TINKATINK'], starter=['TOTODILE'],
     surf=['TENTACOOL', 'QWILFISH', 'FINNEON'], fish=['MAGIKARP', 'TENTACOOL', 'QWILFISH', 'CHINCHOU', 'REMORAID'],
     headbutt=['PINECO', 'AIPOM', 'SKWOVET'])
area('Ruins of Alph', 1, '7-12',
     day=['NATU', 'SMEARGLE', 'BALTOY', 'ELGYEM', 'SOLOSIS'], night=['NATU', 'YAMASK', 'SIGILYPH'],
     rare=['BRONZOR', 'GIMMIGHOUL'], inside=['UNOWN'],
     surf=['POLIWAG', 'WOOPER'], fish=['MAGIKARP', 'POLIWAG'])
area('Union Cave 1F / B1F', 1, '8-13',
     day=['SANDSHREW', 'GEODUDE', 'ONIX', 'FERROSEED', 'ROLYCOLY', 'ARON'],
     night=['ZUBAT', 'WOOBAT', 'MAWILE', 'SABLEYE'], rare=['CUBONE', 'DRILBUR'],
     surf=['WOOPER', 'QUAGSIRE'], fish=['MAGIKARP', 'GOLDEEN', 'KRABBY', 'CORPHISH'])
area('Route 33 (always raining)', 1, '9-14',
     day=['HOPPIP', 'EKANS', 'CROAGUNK', 'MUDBRAY', 'TYMPOLE', 'CHEWTLE'],
     night=['SALANDIT', 'ODDISH', 'STUNFISK', 'SHROOMISH'], rare=['TOXEL', 'GOOMY'],
     headbutt=['AIPOM', 'CHERUBI', 'COMBEE'])
area('Slowpoke Well', 1, '9-14',
     day=['ZUBAT', 'SLOWPOKE', 'INKAY', 'TRUBBISH'], night=['ZUBAT', 'GRIMER', 'TRUBBISH'],
     rare=['MAREANIE', 'GRIMER'],
     surf=['SLOWPOKE', 'TENTACOOL'], fish=['MAGIKARP', 'SHELLDER', 'MAREANIE'])
area('Ilex Forest', 1, '10-15',
     day=['ODDISH', 'PARAS', 'VENIPEDE', 'SEWADDLE', 'FOONGUS', 'SKIDDO'],
     night=['VENONAT', 'PHANTUMP', 'MORELULL', 'PUMPKABOO'], rare=['COTTONEE', 'ZORUA'], starter=['SNIVY', 'ROWLET'],
     surf=['PSYDUCK', 'DUCKLETT', 'DEWPIDER'], fish=['MAGIKARP', 'GOLDEEN', 'BARBOACH'],
     headbutt=['PANSAGE', 'PANSEAR', 'PANPOUR', 'PINECO', 'HERACROSS'])

# ---------------- Act 2: Goldenrod to Ecruteak (badges 2-3) ----------------
area('Route 34', 2, '12-17',
     day=['SNUBBULL', 'ABRA', 'DROWZEE', 'STUFFUL', 'LILLIPUP', 'JIGGLYPUFF', 'MILCERY'],
     night=['MUNNA', 'GOTHITA', 'IMPIDIMP', 'ESPURR'], rare=['DITTO', 'EEVEE', 'RALTS'], starter=['TREECKO'],
     surf=['CORSOLA', 'STARYU', 'WINGULL'], fish=['MAGIKARP', 'KRABBY', 'HORSEA', 'CLAUNCHER'])
area('National Park', 2, '13-18',
     day=['SUNKERN', 'COMBEE', 'PETILIL', 'NYMBLE', 'SPRITZEE', 'SWIRLIX', 'RELLOR'],
     night=['VENONAT', 'VOLBEAT', 'ILLUMISE', 'KRICKETOT', 'TAROUNTULA'], rare=['FOMANTIS', 'BOUNSWEET'],
     headbutt=['HERACROSS', 'PINECO', 'CUTIEFLY'])
area('Bug-Catching Contest (Tue/Thu/Sat)', 2, '13-20',
     day=['SCYTHER', 'PINSIR', 'HERACROSS', 'JOLTIK', 'CUTIEFLY', 'GRUBBIN', 'KARRABLAST', 'SHELMET'],
     rare=['LARVESTA', 'DEWPIDER'])
area('Route 35', 2, '15-20',
     day=['GROWLITHE', 'NIDORAN_F', 'NIDORAN_M', 'DEERLING', 'STARLY', 'YANMA'],
     night=['HOUNDOUR', 'MURKROW', 'NOCTOWL', 'DRIFLOON'], rare=['HATENNA', 'CLEFAIRY'], starter=['TURTWIG'],
     surf=['PSYDUCK', 'MARILL'], fish=['MAGIKARP', 'POLIWAG', 'BASCULIN'])
area('Route 36', 2, '16-21',
     day=['VULPIX', 'STANTLER', 'BUNEARY', 'SKITTY', 'FIDOUGH', 'SMOLIV'],
     night=['HOOTHOOT', 'SPINARAK', 'STUNKY', 'VOLTORB'], rare=['ORICORIO', 'MINCCINO'], starter=['CHESPIN'],
     static=['SUDOWOODO'], headbutt=['AIPOM', 'PINECO', 'SHROODLE'])
area('Route 37', 2, '18-24',
     day=['STANTLER', 'PLUSLE', 'MINUN', 'LUXIO', 'SPOINK', 'GOSSIFLEUR'],
     night=['ARIADOS', 'LEDIAN', 'MIGHTYENA', 'GREAVARD'], rare=['CYCLIZAR', 'SCYTHER'], starter=['TORCHIC'],
     headbutt=['PINECO', 'SHROODLE', 'BRAMBLIN'])
area('Burned Tower 1F', 2, '20-25',
     day=['KOFFING', 'SLUGMA', 'MAGMAR', 'CHARCADET', 'NUMEL', 'SIZZLIPEDE', 'DARUMAKA'],
     night=['KOFFING', 'MISDREAVUS', 'LITWICK', 'DUSKULL'], rare=['TORKOAL', 'TURTONATOR'], starter=['TEPIG'])
area('Burned Tower B1F', 2, '20-25',
     day=['MAGBY', 'ELEKID', 'ROLYCOLY', 'HEATMOR'], night=['SHUPPET', 'DUSKULL'], rare=['LITLEO', 'CAPSAKID'])

# ---------------- Act 3: the open middle (badges 4-6, Chuck / Jasmine / Pryce in any order) ----------------
area('Route 38', 3, '24-30',
     day=['MILTANK', 'TAUROS', 'MAGNEMITE', 'MUDBRAY', 'WOOLOO', 'YAMPER'],
     night=['MEOWTH', 'GLAMEOW', 'MORPEKO', 'NOCTOWL'], rare=['FARFETCHD', 'ORTHWORM'], starter=['CHIMCHAR', 'LITTEN'],
     headbutt=['AIPOM', 'PINECO', 'SKWOVET'])
area('Route 39', 3, '24-30',
     day=['MILTANK', 'TAUROS', 'CUFANT', 'BOUFFALANT', 'MAREEP', 'OINKOLOGNE'],
     night=['MEOWTH', 'MASCHIFF', 'NOCTOWL', 'HOOTHOOT'], rare=['KANGASKHAN', 'AUDINO'], starter=['FENNEKIN'],
     headbutt=['PINECO', 'SKWOVET'])
area('Routes 40 / 41 (sea)', 3, '26-32',
     surf=['TENTACOOL', 'FINIZEN', 'WINGULL', 'FRILLISH'], starter=['POPPLIO'],
     rare=['CRAMORANT', 'DHELMISE', 'LAPRAS'],
     fish=['MAGIKARP', 'KRABBY', 'STARYU', 'CHINCHOU', 'SHELLDER', 'ARROKUDA'])
area('Cianwood City', 3, '28-32', smash=['SHUCKLE', 'BINACLE'], surf=['TENTACOOL', 'MANTINE'],
     fish=['MAGIKARP', 'CORSOLA', 'QWILFISH', 'PYUKUMUKU', 'WIGLETT'])
area('Whirl Islands', 3, '26-34',
     day=['ZUBAT', 'GOLBAT', 'SEEL', 'KRABBY', 'SPHEAL', 'CLAMPERL'],
     night=['GOLBAT', 'SEEL', 'SPHEAL', 'SANDYGAST'], rare=['RELICANTH', 'WIMPOD'], starter=['OSHAWOTT'],
     surf=['HORSEA', 'CARVANHA', 'WISHIWASHI', 'SHELLOS', 'ALOMOMOLA'],
     fish=['MAGIKARP', 'LUVDISC', 'BRUXISH', 'HORSEA', 'CLAMPERL'])
area('Route 42', 3, '25-32',
     day=['MANKEY', 'MARACTUS', 'EKANS', 'FLAAFFY', 'SANDILE', 'HELIOPTILE'],
     night=['ZUBAT', 'GOLBAT', 'MANKEY', 'TRAPINCH'], rare=['CACNEA', 'SPINDA'], starter=['PIPLUP'],
     surf=['MARILL', 'GOLDEEN'], fish=['MAGIKARP', 'GOLDEEN', 'SEAKING'],
     headbutt=['AIPOM', 'PINECO', 'SPINDA'])
area('Mt. Mortar', 3, '25-32',
     day=['MACHOP', 'MARILL', 'GEODUDE', 'TIMBURR', 'RHYHORN', 'HIPPOPOTAS'],
     night=['CLEFAIRY', 'SABLEYE', 'MAWILE', 'NOIBAT'], rare=['TYROGUE', 'BELDUM'],
     surf=['MARILL', 'GOLDEEN'], fish=['MAGIKARP', 'GOLDEEN', 'BARBOACH'])
area('Mt. Mortar B1F (Karate King dojo)', 3, '30-36',
     day=['RIOLU', 'THROH', 'SAWK', 'MIENFOO', 'PANCHAM', 'CRABRAWLER'],
     night=['SCRAGGY', 'HAWLUCHA', 'PASSIMIAN', 'FALINKS', 'CLOBBOPUS'], rare=['TYROGUE', 'MAKUHITA'])
area('Route 47', 3, '32-38',
     day=['RATICATE', 'FEAROW', 'PONYTA', 'DODUO', 'TAILLOW', 'SILICOBRA'],
     night=['GLOOM', 'NOCTOWL', 'SEVIPER', 'ZANGOOSE'], rare=['KLAWF', 'CHANSEY', 'DITTO'],
     surf=['SEEL', 'TENTACOOL'], fish=['MAGIKARP', 'GYARADOS', 'LUVDISC'])
area('Route 48 (Safari gate)', 3, '33-39',
     day=['FARIGIRAF', 'DIGLETT', 'GIRAFARIG', 'TAUROS', 'KECLEON', 'CASTFORM'],
     night=['NOCTOWL', 'MIGHTYENA', 'SKORUPI', 'LUNATONE'], rare=['SOLROCK', 'TOGEDEMARU'])
area('Route 43', 3, '32-38',
     day=['GIRAFARIG', 'FLAAFFY', 'ZANGOOSE', 'SEVIPER', 'BLITZLE', 'FLAMIGO', 'TOEDSCOOL', 'SWABLU'],
     night=['VENOMOTH', 'NOCTOWL', 'LOKIX', 'PACHIRISU', 'HONEDGE', 'ELECTRIKE'], rare=['SPIRITOMB', 'DREEPY', 'ELECTABUZZ'],
     surf=['MAGIKARP', 'POLIWHIRL'], fish=['MAGIKARP', 'POLIWAG', 'BASCULIN'])
area('Lake of Rage (water only)', 3, '34-40',
     surf=['MAGIKARP', 'GYARADOS', 'TATSUGIRI', 'BUIZEL'], fish=['MAGIKARP', 'GYARADOS', 'FEEBAS', 'VELUZA', 'DONDOZO'])

# ---------------- Act 4: Mahogany to the League (badges 6-8) ----------------
area('Route 44', 4, '38-42',
     day=['TANGELA', 'LICKITUNG', 'WEEPINBELL', 'SNOVER', 'TROPIUS', 'CHATOT', 'CARNIVINE', 'KOMALA'],
     night=['POLIWHIRL', 'GRAFAIAI', 'INDEEDEE', 'GLIGAR'], rare=['BISHARP', 'SQUAWKABILLY'], starter=['MUDKIP'],
     surf=['POLITOED', 'REMORAID', 'OCTILLERY', 'ALOMOMOLA'], fish=['MAGIKARP', 'REMORAID', 'POLIWHIRL'])
area('Ice Path', 4, '39-44',
     day=['SWINUB', 'EISCUE', 'SNEASEL', 'JYNX', 'BERGMITE', 'VANILLITE', 'STONJOURNER'],
     night=['DELIBIRD', 'SNORUNT', 'CETODDLE', 'CUBCHOO'], rare=['CRYOGONAL', 'SNOM', 'FRIGIBAX'])
area("Dragon's Den", 4, '41-46',
     surf=['DRATINI', 'HORSEA', 'SKRELP', 'GOOMY'], rare=['DRAGONAIR', 'DURALUDON'],
     fish=['MAGIKARP', 'DRATINI', 'HORSEA', 'SEADRA'], gift=['DRATINI'])
area('Dark Cave (Route 45 side)', 4, '41-46',
     day=['LARVITAR', 'DEINO', 'GLIMMET', 'CARBINK', 'GRAVELER', 'GOLBAT', 'DURANT'],
     night=['GOLBAT', 'SABLEYE', 'URSARING', 'KLEFKI', 'BOLDORE'], rare=['AXEW', 'DRUDDIGON'])
area('Route 45', 4, '42-46',
     day=['GLIGAR', 'SKARMORY', 'DONPHAN', 'URSARING', 'CORVISQUIRE', 'PHANPY'],
     night=['HOUNDOOM', 'ABSOL', 'LYCANROC', 'BAGON'], rare=['LARVITAR', 'JANGMO_O'],
     headbutt=['HERACROSS', 'PINECO', 'AIPOM'])
area('Route 27', 4, '44-49',
     day=['DODRIO', 'PONYTA', 'TAUROS', 'ARCANINE', 'STARAVIA', 'MAGNETON'],
     night=['NOCTOWL', 'MIGHTYENA', 'ZOROARK', 'BANETTE'], rare=['GIBLE', 'SALANDIT'],
     surf=['TENTACRUEL', 'WAILMER', 'PELIPPER'], fish=['MAGIKARP', 'SEAKING', 'KINGLER', 'LANTURN'])
area('Tohjo Falls', 4, '44-49',
     day=['GOLBAT', 'SLOWPOKE', 'RATICATE', 'CROBAT'], night=['GOLBAT', 'GOLDUCK'], rare=['ROTOM'],
     surf=['GOLDUCK', 'SLOWPOKE', 'BUIZEL'], fish=['MAGIKARP', 'POLIWHIRL', 'SEAKING'])
area('Route 26', 4, '45-50',
     day=['DODRIO', 'ARCANINE', 'PONYTA', 'FEAROW', 'SANDSLASH', 'DIGGERSBY'],
     night=['NOCTOWL', 'RATICATE', 'LIEPARD', 'MURKROW'], rare=['AUDINO', 'BUNNELBY'],
     surf=['TENTACRUEL', 'QUAGSIRE'], fish=['MAGIKARP', 'KINGLER', 'SEAKING'])
area('Victory Road', 4, '46-52',
     day=['GRAVELER', 'GOLBAT', 'URSARING', 'DONPHAN', 'RHYDON', 'EXCADRILL'],
     night=['SABLEYE', 'MAWILE', 'ZWEILOUS', 'HAUNTER'], rare=['PUPITAR', 'GABITE', 'METANG', 'GOLETT'],
     smash=['ONIX', 'GRAVELER'])

# ---------------- Act 5: Kanto (scaled: base level + 1 per Kanto badge) ----------------
area('Routes 6 & 11', 5, '54-58',
     day=['MANECTRIC', 'MAGNETON', 'KILOWATTREL', 'HYPNO', 'BELLIBOLT', 'EMOLGA'],
     night=['MABOSSTIFF', 'BANETTE', 'LOKIX', 'DEDENNE'], rare=['PINCURCHIN', 'PORYGON'])
area("Diglett's Cave", 5, '54-58', day=['DUGTRIO', 'EXCADRILL', 'DONPHAN', 'GRAVELER'],
     night=['SABLEYE', 'MAWILE'], rare=['ORTHWORM', 'GARCHOMP'])
area('Routes 5, 7 & 8', 5, '54-58',
     day=['KADABRA', 'KIRLIA', 'HOUNDOOM', 'PYROAR', 'ZEBSTRIKA', 'FURFROU'],
     night=['HONCHKROW', 'MISMAGIUS', 'ABSOL', 'PURUGLY'], rare=['GALLADE', 'CHANSEY'], starter=['GROOKEY', 'SCORBUNNY'])
area('Routes 9 & 10', 5, '55-59',
     day=['ELECTABUZZ', 'LUXRAY', 'MAGNEZONE', 'SKARMORY', 'KANGASKHAN', 'AGGRON'],
     night=['ELECTIVIRE', 'GARGANACL', 'COALOSSAL', 'LAIRON'], rare=['METANG', 'ARCHALUDON', 'TYNAMO'],
     surf=['GOLDUCK', 'POLIWHIRL'], fish=['MAGIKARP', 'SEAKING', 'BARBOACH'])
area('Rock Tunnel', 5, '55-59', day=['GRAVELER', 'ONIX', 'MACHOKE', 'BOLDORE', 'GURDURR', 'KLINK'],
     night=['GOLBAT', 'WOOBAT', 'ROTOM'], rare=['TYNAMO', 'KLANG'])
area('Routes 24 & 25', 5, '55-59',
     day=['GOLDUCK', 'STARAPTOR', 'BRELOOM', 'ROSERADE', 'SCYTHER', 'VIVILLON'],
     night=['NOIVERN', 'BISHARP', 'SHIINOTIC', 'VENOMOTH'], rare=['METAGROSS', 'MINIOR'], starter=['FROAKIE'],
     surf=['STARMIE', 'BARRASKEWDA', 'PALAFIN'], fish=['MAGIKARP', 'SEAKING', 'GOLDUCK'])
area('Routes 16, 17 & 18 (Cycling Road)', 5, '56-60',
     day=['BOMBIRDIER', 'CYCLIZAR', 'REVAVROOM', 'TALONFLAME', 'BRAVIARY', 'VAROOM'],
     night=['CROBAT', 'MANDIBUZZ', 'GRIMMSNARL', 'SCRAFTY'], rare=['SLAKING', 'DRAGAPULT', 'VULLABY', 'RUFFLET'])
area('Routes 12 to 15', 5, '56-60',
     day=['NIDOQUEEN', 'NIDOKING', 'TSAREENA', 'SCIZOR', 'PINSIR', 'FARIGIRAF'],
     night=['CLODSIRE', 'SKUNTANK', 'GOURGEIST', 'TREVENANT'], rare=['CHANSEY', 'AUDINO', 'BLISSEY', 'SNORLAX'], static=['SNORLAX'], starter=['SOBBLE'],
     surf=['TENTACRUEL', 'QUAGSIRE'], fish=['MAGIKARP', 'GYARADOS', 'KINGLER'],
     headbutt=['COMFEY', 'ORANGURU', 'FLABEBE'])
area('Routes 19 to 21 (sea)', 5, '56-60',
     surf=['TENTACRUEL', 'JELLICENT', 'PELIPPER', 'CLOYSTER', 'SHARPEDO', 'WAILORD'],
     rare=['MILOTIC', 'KINGDRA', 'DONDOZO', 'EELEKTROSS'], starter=['QUAXLY'],
     fish=['MAGIKARP', 'GYARADOS', 'HUNTAIL', 'GOREBYSS', 'LANTURN', 'DRAGALGE', 'CLAWITZER', 'BARBARACLE', 'GOLISOPOD'])
area('Seafoam Islands', 5, '57-61',
     day=['ABOMASNOW', 'GLALIE', 'FROSLASS', 'WALREIN', 'CETITAN', 'BEARTIC'],
     night=['TORKOAL', 'CERULEDGE', 'VOLCARONA', 'MAGCARGO'], rare=['AVALUGG', 'VANILLUXE', 'CRYOGONAL'],
     surf=['SEAKING', 'DEWGONG', 'LAPRAS'])
area('Routes 3 & 4', 5, '55-59',
     day=['ANNIHILAPE', 'KROOKODILE', 'HAWLUCHA', 'CLEFABLE', 'GIGALITH', 'GOLURK'],
     night=['CLEFABLE', 'LUNATONE', 'SOLROCK', 'MINIOR'],
     starter=['FUECOCO'])
area('Mt. Moon (fossil cave)', 5, '55-59',
     day=['KABUTO', 'OMANYTE', 'AERODACTYL', 'LILEEP', 'ANORITH', 'CRANIDOS', 'SHIELDON',
          'TIRTOUGA', 'ARCHEN', 'TYRUNT', 'AMAURA'],
     rare=['CLEFAIRY', 'MINIOR'])
area('Routes 1, 2 & 22, Viridian Forest', 5, '54-58',
     day=['HERACROSS', 'PINSIR', 'LEAVANNY', 'ORBEETLE', 'PIKACHU', 'BUTTERFREE'],
     night=['VIKAVOLT', 'TREVENANT', 'GOURGEIST', 'BEEDRILL'], rare=['DEDENNE', 'PICHU'], starter=['BULBASAUR', 'CHARMANDER', 'SQUIRTLE', 'SPRIGATITO'],
     headbutt=['ELDEGOSS', 'GREEDENT', 'TOUCANNON'])
area('Route 28 (Mt. Silver approach)', 6, '72-78',
     day=['RAPIDASH', 'ARCANINE', 'URSALUNA', 'TANGROWTH', 'CORVIKNIGHT', 'MUDSDALE'],
     night=['HOUNDOOM', 'WEAVILE', 'KINGAMBIT', 'HOUNDSTONE'], rare=['DRAMPA', 'DRAGONITE', 'BLISSEY'])

# ---------------- Act 6: post-game ----------------
area('Cerulean Cave', 6, '74-82',
     day=['GREAT_TUSK', 'FLUTTER_MANE', 'BRUTE_BONNET', 'SANDY_SHOCKS', 'SCREAM_TAIL', 'SLITHER_WING',
          'ROARING_MOON', 'WALKING_WAKE', 'GOUGING_FIRE', 'RAGING_BOLT'],
     night=['IRON_VALIANT', 'IRON_BUNDLE', 'IRON_HANDS', 'IRON_MOTH', 'IRON_TREADS', 'IRON_JUGULIS',
            'IRON_THORNS', 'IRON_LEAVES', 'IRON_BOULDER', 'IRON_CROWN'],
     surf=['DRACOVISH', 'ARCTOVISH'], rare=['DRACOZOLT', 'ARCTOZOLT', 'DITTO'])
area('Mt. Silver', 6, '76-85',
     day=['URSALUNA', 'MAMOSWINE', 'KLEAVOR', 'DRAGONITE', 'TYRANITAR', 'SALAMENCE', 'METAGROSS', 'GARCHOMP'],
     night=['HYDREIGON', 'GOODRA', 'KOMMO_O', 'DRAGAPULT', 'BAXCALIBUR', 'ARCHALUDON', 'GHOLDENGO', 'KINGAMBIT'],
     rare=['HYDRAPPLE', 'MIMIKYU'])

# ---------------- Gifts (C gift manager) ----------------
GIFTS = {
    'Professor Elm (new game)': ['CHIKORITA', 'CYNDAQUIL', 'TOTODILE'],
    'Mr. Pokémon (vanilla Togepi egg)': ['TOGEPI'],
    'Bill, Goldenrod (vanilla)': ['EEVEE'],
    'Pewter Museum fossil revival (Rock Smash fossils, Ruins of Alph and Cliff Cave)': [
        'OMANYTE', 'KABUTO', 'AERODACTYL', 'LILEEP', 'ANORITH', 'CRANIDOS', 'SHIELDON'],
}
# Babies that need an incense (Happiny, Mantyke, Mime Jr., Wynaut, Munchlax, Budew, Chingling,
# Bonsly, Azurill) hatch from eggs bred with it; the incenses are sold at Goldenrod 4F (§8.2).
# ---------------- Legendaries, mythicals and Ultra Beasts: DEFERRED (user, 2026-10-27) ----------------
# Not placed in this milestone's maps. The list is the proposal for the C legend manager (decision D5),
# kept for when it is scheduled; the check reports these families as deferred instead of failing.
DEFERRED_GIFTS = ['KUBFU', 'MELTAN', 'COSMOG', 'POIPOLE', 'TYPE_NULL']
LEGENDS = [
    ('Articuno', 'Seafoam Islands B4F', 'vanilla static', 70), ('Zapdos', 'Route 10 Power Plant', 'vanilla static', 70),
    ('Moltres', 'Mt. Silver (Moltres room)', 'vanilla static', 70), ('Mewtwo', 'Cerulean Cave B2F', 'vanilla static', 80),
    ('Mew', 'Route 1 Viridian Forest edge, post-Red', 'legend manager', 75),
    ('Raikou', 'Johto routes', 'vanilla roamer', 50), ('Entei', 'Johto routes', 'vanilla roamer', 50),
    ('Suicune', 'Route 25 / Bell Tower chase', 'vanilla static', 50),
    ('Lugia', 'Whirl Islands B3F', 'vanilla static', 60), ('Ho-Oh', 'Bell Tower roof', 'vanilla static', 60),
    ('Celebi', 'Ilex Forest shrine, post-E4', 'legend manager', 60),
    ('Regirock', 'Ruins of Alph Kabuto chamber, post-E4', 'legend manager', 65), ('Regice', 'Ice Path B3F, post-E4', 'legend manager', 65),
    ('Registeel', 'Mt. Mortar B1F, post-E4', 'legend manager', 65), ('Regieleki', 'Kanto Power Plant', 'legend manager', 70),
    ('Regidrago', "Dragon's Den shrine, after Rising Badge rematch", 'legend manager', 70),
    ('Regigigas', 'Ice Path B1F, with the other five Regis in the party', 'legend manager', 75),
    ('Latias', 'Kanto routes', 'vanilla roamer', 60), ('Latios', 'Kanto routes', 'vanilla roamer', 60),
    ('Kyogre', 'Embedded Tower (Route 47)', 'vanilla event, both versions', 75), ('Groudon', 'Embedded Tower (Route 47)', 'vanilla event, both versions', 75),
    ('Rayquaza', 'Embedded Tower (Route 47)', 'vanilla event', 75), ('Jirachi', 'Goldenrod Radio Tower roof, post-Red', 'legend manager', 75),
    ('Deoxys', 'Mt. Moon Square, post-Red', 'legend manager', 75),
    ('Uxie', 'Lake of Rage islet, post-E4', 'legend manager', 65), ('Mesprit', 'Cerulean Cape, post-E4', 'legend manager', 65),
    ('Azelf', 'Route 44 pond, post-E4', 'legend manager', 65),
    ('Dialga', 'Sinjoh Ruins', 'vanilla event', 80), ('Palkia', 'Sinjoh Ruins', 'vanilla event', 80),
    ('Giratina', 'Sinjoh Ruins', 'vanilla event', 80), ('Arceus', 'Sinjoh Ruins, post-Red', 'legend manager', 90),
    ('Heatran', 'Burned Tower B1F magma vent, post-E4', 'legend manager', 70), ('Cresselia', 'Route 26 (night), post-E4', 'legend manager', 70),
    ('Darkrai', 'Lavender Town, post-E4 (night)', 'legend manager', 70), ('Shaymin', 'Route 2 flower field, post-E4', 'legend manager', 60),
    ('Phione', 'Cianwood beach', 'legend manager', 50), ('Manaphy', 'Whirl Islands, post-E4', 'legend manager', 60),
    ('Victini', 'Ecruteak Dance Theater, post-E4', 'legend manager', 60),
    ('Cobalion', 'Route 45 cliff, post-E4', 'legend manager', 65), ('Terrakion', 'Victory Road, post-E4', 'legend manager', 65),
    ('Virizion', 'National Park, post-E4', 'legend manager', 65), ('Keldeo', 'Tohjo Falls, with the three Swords', 'legend manager', 65),
    ('Tornadus', 'Route 43 (storm), post-E4', 'legend manager', 70), ('Thundurus', 'Route 10, post-E4', 'legend manager', 70),
    ('Landorus', 'Route 47 cliffs, with Tornadus and Thundurus', 'legend manager', 70),
    ('Enamorus', 'Route 25 Cerulean Cape, with the three Forces', 'legend manager', 70),
    ('Reshiram', 'Dragon\'s Den shrine, post-Red', 'legend manager', 80), ('Zekrom', 'Dragon\'s Den, post-Red', 'legend manager', 80),
    ('Kyurem', 'Seafoam Islands B4F, post-Red', 'legend manager', 80), ('Meloetta', 'Goldenrod Radio Tower, post-E4', 'legend manager', 65),
    ('Genesect', 'Silph Co., post-Red', 'legend manager', 75),
    ('Xerneas', 'Viridian Forest heart, post-Red', 'legend manager', 80), ('Yveltal', 'Mt. Silver summit approach, post-Red', 'legend manager', 80),
    ('Zygarde', 'Route 28, post-Red', 'legend manager', 80), ('Diancie', 'Mt. Moon, post-Red', 'legend manager', 75),
    ('Hoopa', 'Ruins of Alph, post-Red', 'legend manager', 75), ('Volcanion', 'Cinnabar Island, post-Red', 'legend manager', 75),
    ('Tapu Koko', 'Route 10, post-E4', 'legend manager', 70), ('Tapu Lele', 'Route 36 Sinjoh path, post-E4', 'legend manager', 70),
    ('Tapu Bulu', 'Route 47, post-E4', 'legend manager', 70), ('Tapu Fini', 'Route 41, post-E4', 'legend manager', 70),
    ('Solgaleo / Lunala', 'via Cosmog gift (Lv 43 / 53, day or night)', 'evolution', 0),
    ('Nihilego', 'Ruins of Alph hidden chamber 1', 'legend manager (Ultra Beast)', 75), ('Buzzwole', 'Ruins of Alph chamber 2', 'legend manager (UB)', 75),
    ('Pheromosa', 'Ruins of Alph chamber 3', 'legend manager (UB)', 75), ('Xurkitree', 'Ruins of Alph chamber 4', 'legend manager (UB)', 75),
    ('Celesteela', 'Ruins of Alph main room', 'legend manager (UB)', 75), ('Kartana', 'Ilex Forest, post-Red', 'legend manager (UB)', 75),
    ('Guzzlord', 'Cerulean Cave B1F', 'legend manager (UB)', 75), ('Stakataka', 'Victory Road, post-Red', 'legend manager (UB)', 75),
    ('Blacephalon', 'Burned Tower, post-Red', 'legend manager (UB)', 75), ('Naganadel', 'via Poipole gift', 'evolution', 0),
    ('Necrozma', 'Mt. Silver cave summit, post-Red', 'legend manager', 85), ('Magearna', 'Silph Co. lab, post-Red', 'legend manager', 75),
    ('Marshadow', 'Lavender Radio Tower, post-Red', 'legend manager', 75), ('Zeraora', 'Power Plant, post-Red', 'legend manager', 75),
    ('Zacian', 'Route 2 forest, post-Red', 'legend manager', 85), ('Zamazenta', 'Route 2 forest, post-Red', 'legend manager', 85),
    ('Eternatus', 'Cerulean Cave depths, post-Red', 'legend manager', 90), ('Zarude', 'Viridian Forest, post-Red', 'legend manager', 75),
    ('Glastrier', 'Ice Path, post-Red', 'legend manager', 80), ('Spectrier', 'Sprout Tower, post-Red', 'legend manager', 80),
    ('Calyrex', 'Ruins of Alph, post-Red', 'legend manager', 80),
    ('Wo-Chien', 'Ilex Forest shrine, post-E4', 'legend manager', 70), ('Chien-Pao', 'Ice Path, post-E4', 'legend manager', 70),
    ('Ting-Lu', 'Mt. Mortar, post-E4', 'legend manager', 70), ('Chi-Yu', 'Burned Tower, post-E4', 'legend manager', 70),
    ('Koraidon', 'Route 26, post-Red', 'legend manager', 85), ('Miraidon', 'Route 27, post-Red', 'legend manager', 85),
    ('Okidogi', 'Route 12, post-E4', 'legend manager', 75), ('Munkidori', 'Route 13, post-E4', 'legend manager', 75),
    ('Fezandipiti', 'Route 14, post-E4', 'legend manager', 75), ('Pecharunt', 'Lavender Town, with the Loyal Three', 'legend manager', 80),
    ('Ogerpon', 'Route 2 north, post-E4', 'legend manager', 75), ('Terapagos', 'Cerulean Cave B2F lake, post-Red', 'legend manager', 90),
    ('Silvally', 'via Type: Null (friendship)', 'evolution', 0), ('Melmetal', 'via Meltan', 'evolution', 0),
    ('Urshifu', 'via Kubfu', 'evolution', 0),
    ('Articuno-Galar', 'Safari Zone Peak, post-Red', 'legend manager (alternate form)', 75),
    ('Zapdos-Galar', 'Safari Zone Plains, post-Red', 'legend manager (alternate form)', 75),
    ('Moltres-Galar', 'Safari Zone Wasteland, post-Red', 'legend manager (alternate form)', 75),
]

# ---------------- Safari Zone: alternate forms only ----------------
SAFARI = [
    ('Rocky Beach', 'Alola coast',
     dict(land=['EXEGGUTOR_ALOLAN', 'GEODUDE_ALOLAN', 'GRAVELER_ALOLAN', 'DIGLETT_ALOLAN', 'DUGTRIO_ALOLAN'],
          rare=['RAICHU_ALOLAN', 'GOLEM_ALOLAN'],
          surf=['SHELLOS_EAST_SEA', 'GASTRODON_EAST_SEA', 'CORSOLA_GALARIAN'],
          vivillon=['VIVILLON_OCEAN', 'VIVILLON_MARINE'])),
    ('Meadow', 'Alola flower fields',
     dict(land=['ORICORIO_POM_POM', 'ORICORIO_PAU', 'ORICORIO_SENSU', 'MEOWTH_ALOLAN', 'RATTATA_ALOLAN'],
          night=['RATICATE_ALOLAN', 'PERSIAN_ALOLAN'],
          rare=['FLABEBE_YELLOW_FLOWER', 'FLABEBE_ORANGE_FLOWER', 'FLABEBE_BLUE_FLOWER', 'FLABEBE_WHITE_FLOWER'],
          vivillon=['VIVILLON_MEADOW', 'VIVILLON_SUN', 'VIVILLON_FANCY', 'VIVILLON_POKE_BALL'])),
    ('Desert', 'Alola and Galar arid ruins',
     dict(land=['MAROWAK_ALOLAN', 'YAMASK_GALARIAN', 'DIGLETT_ALOLAN', 'ROCKRUFF_OWN_TEMPO'],
          night=['MAROWAK_ALOLAN', 'YAMASK_GALARIAN'], rare=['DUGTRIO_ALOLAN'],
          vivillon=['VIVILLON_SANDSTORM'])),
    ('Mountain', 'Alola and Galar snow',
     dict(land=['SANDSHREW_ALOLAN', 'VULPIX_ALOLAN', 'DARUMAKA_GALARIAN', 'MR_MIME_GALARIAN'],
          rare=['SANDSLASH_ALOLAN', 'NINETALES_ALOLAN', 'DARMANITAN_GALARIAN'],
          vivillon=['VIVILLON_POLAR'])),
    ('Peak', 'Hisui highlands',
     dict(land=['GROWLITHE_HISUIAN', 'SNEASEL_HISUIAN', 'ZORUA_HISUIAN'],
          night=['ZORUA_HISUIAN', 'SNEASEL_HISUIAN'],
          rare=['ARCANINE_HISUIAN', 'BRAVIARY_HISUIAN', 'TYPHLOSION_HISUIAN', 'AVALUGG_HISUIAN'],
          minior=['MINIOR_METEOR_ORANGE', 'MINIOR_METEOR_YELLOW', 'MINIOR_METEOR_GREEN', 'MINIOR_METEOR_BLUE',
                  'MINIOR_METEOR_INDIGO', 'MINIOR_METEOR_VIOLET'],
          vivillon=['VIVILLON_TUNDRA', 'VIVILLON_HIGH_PLAINS'])),
    ('Forest', 'Hisui woods and the Glimwood',
     dict(land=['VOLTORB_HISUIAN', 'PONYTA_GALARIAN', 'PUMPKABOO_SMALL', 'PUMPKABOO_LARGE'],
          night=['ZOROARK_HISUIAN', 'PUMPKABOO_SUPER'],
          rare=['ELECTRODE_HISUIAN', 'LILLIGANT_HISUIAN', 'DECIDUEYE_HISUIAN'],
          vivillon=['VIVILLON_GARDEN', 'VIVILLON_ELEGANT', 'VIVILLON_JUNGLE'])),
    ('Plains', 'Galar pasture',
     dict(land=['PONYTA_GALARIAN', 'FARFETCHD_GALARIAN', 'ZIGZAGOON_GALARIAN', 'MEOWTH_GALARIAN'],
          night=['LINOONE_GALARIAN', 'ZIGZAGOON_GALARIAN'],
          rare=['RAPIDASH_GALARIAN'], vivillon=['VIVILLON_CONTINENTAL'])),
    ('Wasteland', 'Galar industry',
     dict(land=['WEEZING_GALARIAN', 'MEOWTH_GALARIAN', 'GRIMER_ALOLAN', 'STUNFISK_GALARIAN'],
          night=['MUK_ALOLAN', 'GRIMER_ALOLAN'], rare=['MUK_ALOLAN'],
          vivillon=['VIVILLON_MODERN'])),
    ('Marshland', 'Galar and Hisui bog',
     dict(land=['SLOWPOKE_GALARIAN', 'STUNFISK_GALARIAN', 'SLIGGOO_HISUIAN', 'WOOPER_PALDEAN'],
          rare=['SLOWBRO_GALARIAN', 'SLOWKING_GALARIAN', 'GOODRA_HISUIAN', 'SAMUROTT_HISUIAN'],
          surf=['BASCULIN_WHITE_STRIPED', 'SLOWPOKE_GALARIAN'], fish=['QWILFISH_HISUIAN', 'BASCULIN_WHITE_STRIPED'],
          vivillon=['VIVILLON_MONSOON'])),
    ('Savannah', 'Paldea plains',
     dict(land=['TAUROS_COMBAT', 'TAUROS_BLAZE', 'TAUROS_AQUA', 'SQUAWKABILLY_BLUE_PLUMAGE'],
          night=['SQUAWKABILLY_YELLOW_PLUMAGE', 'SQUAWKABILLY_WHITE_PLUMAGE'],
          vivillon=['VIVILLON_SAVANNA'])),
    ('Wetland', 'Paldea and Hisui estuary',
     dict(land=['WOOPER_PALDEAN', 'SLIGGOO_HISUIAN', 'QWILFISH_HISUIAN'],
          surf=['TATSUGIRI_DROOPY', 'TATSUGIRI_STRETCHY', 'BASCULIN_BLUE_STRIPED'],
          fish=['QWILFISH_HISUIAN', 'BASCULIN_BLUE_STRIPED'],
          vivillon=['VIVILLON_ARCHIPELAGO', 'VIVILLON_RIVER'])),
    ('Swamp', 'Crimson Mire',
     dict(land=['CORSOLA_GALARIAN', 'MR_MIME_GALARIAN', 'SINISTEA_ANTIQUE', 'POLTCHAGEIST_MASTERPIECE'],
          night=['YAMASK_GALARIAN', 'ZORUA_HISUIAN'], rare=['URSALUNA_BLOODMOON'],
          vivillon=[])),
]

# Evolutions hg-engine lists but cannot perform (no case in GetMonEvolutionInternal), and what
# the plan replaces them with (docs/GAME_REBALANCE_UPDATE.md, evolution fixes).
EVO_FIXES = {
    ('PRIMEAPE', 'ANNIHILAPE'): 'level 35 (Rage Fist is unimplemented in hg-engine)',
    ('STANTLER', 'WYRDEER'): 'level 31 (Psyshield Bash is only an egg move here)',
    ('BISHARP', 'KINGAMBIT'): "Leader's Crest, used like a stone",
    ('GIMMIGHOUL', 'GHOLDENGO'): 'Relic Gold, used like a stone, or level 45',
    ('PAWMO', 'PAWMOT'): 'level 32',
    ('BRAMBLIN', 'BRAMBLEGHAST'): 'level 30',
    ('RELLOR', 'RABSCA'): 'level 30',
    ('MILCERY', 'ALCREMIE'): 'a Sweet, used like a stone (the Sweet picks the decoration)',
    ('MELTAN', 'MELMETAL'): 'level 48 (new entry: hg-engine has none)',
    ('DIPPLIN', 'HYDRAPPLE'): 'level 45 (Dragon Cheer is unimplemented in hg-engine)',
}
# Trade evolutions without an item path in hg-engine: the item becomes usable like a stone
# (data/itemdata: field use 20, party use), and plain trades take the Linking Cord. Co-op trades
# keep evolving them too (docs/MILESTONE3_PLAN.md §12).
TRADE_FIXES = {
    ('POLIWHIRL', 'POLITOED'): "King's Rock", ('SLOWPOKE', 'SLOWKING'): "King's Rock",
    ('ONIX', 'STEELIX'): 'Metal Coat', ('SCYTHER', 'SCIZOR'): 'Metal Coat',
    ('RHYDON', 'RHYPERIOR'): 'Protector', ('SEADRA', 'KINGDRA'): 'Dragon Scale',
    ('ELECTABUZZ', 'ELECTIVIRE'): 'Electirizer', ('MAGMAR', 'MAGMORTAR'): 'Magmarizer',
    ('PORYGON', 'PORYGON2'): 'Up-Grade', ('PORYGON2', 'PORYGON_Z'): 'Dubious Disc',
    ('FEEBAS', 'MILOTIC'): 'Prism Scale (HGSS has no Beauty stat)', ('DUSCLOPS', 'DUSKNOIR'): 'Reaper Cloth',
    ('CLAMPERL', 'HUNTAIL'): 'Deep Sea Tooth', ('CLAMPERL', 'GOREBYSS'): 'Deep Sea Scale',
    ('SPRITZEE', 'AROMATISSE'): 'Sachet', ('SWIRLIX', 'SLURPUFF'): 'Whipped Dream',
    ('PHANTUMP', 'TREVENANT'): 'Linking Cord', ('PUMPKABOO', 'GOURGEIST'): 'Linking Cord',
    ('PUMPKABOO_SMALL', 'GOURGEIST'): 'Linking Cord', ('PUMPKABOO_LARGE', 'GOURGEIST'): 'Linking Cord',
    ('PUMPKABOO_SUPER', 'GOURGEIST'): 'Linking Cord',
}
UNIMPLEMENTED = {'EVO_FORM_ARGUMENT', 'EVO_LETS_GO'}
TRADE = {'EVO_TRADE', 'EVO_TRADE_ITEM', 'EVO_TRADE_SPECIFIC_MON'}


def load_engine():
    sp = {}
    for line in open(HG / 'include/constants/species.h'):
        m = re.match(r'#define SPECIES_([A-Z0-9_]+)\s+(\d+)\s*(//.*)?$', line)
        if m and 1 <= int(m.group(2)) <= 1075 and not 494 <= int(m.group(2)) <= 543:
            sp[m.group(1)] = int(m.group(2))
    forms = set(re.findall(r'#define SPECIES_([A-Z0-9_]+)\s+\(', open(HG / 'include/constants/species.h').read()))
    base = {}
    for line in open(HG / 'data/BabyMons.c'):
        m = re.match(r'\s*\[SPECIES_([A-Z0-9_]+)\]\s*=\s*SPECIES_([A-Z0-9_]+)', line)
        if m:
            base[m.group(1)] = m.group(2)
    evo, cur = {}, None
    for line in open(HG / 'data/Evolutions.c'):
        m = re.match(r'\s*\[SPECIES_([A-Z0-9_]+)\]\s*=', line)
        if m:
            cur = m.group(1)
            continue
        m = re.search(r'\{ (EVO_[A-Z0-9_]+), ([^,]+), (?:MON_WITH_FORM\()?SPECIES_([A-Z0-9_]+)', line)
        if m and cur and m.group(1) != 'EVO_NONE':
            evo.setdefault(cur, []).append((m.group(1), m.group(3)))
    return sp, forms, base, evo


def families(sp, base, evo):
    parent = {n: n for n in sp}

    def find(x):
        while parent[x] != x:
            parent[x] = parent[parent[x]]
            x = parent[x]
        return x
    for a, lst in list(evo.items()) + [(a, [(None, b)]) for a, b in base.items()]:
        for _, t in lst:
            if a in parent and t in parent:
                ra, rb = find(a), find(t)
                if ra != rb:
                    parent[max(ra, rb, key=sp.get)] = min(ra, rb, key=sp.get)
    fam = {}
    for n in sp:
        fam.setdefault(find(n), []).append(n)
    return {k: sorted(v, key=sp.get) for k, v in fam.items()}


FAMILY = {}
DEFERRED = set()


def deferred_species(evo):
    """The legendary, mythical and Ultra Beast families, deferred by the user (2026-10-27)."""
    out = set(DEFERRED_GIFTS)
    for disp, loc, meth, lv in LEGENDS:
        for part in disp.split(' / '):
            out.add(legend_const(part.replace('-Galar', '_GALARIAN')))
    changed = True
    while changed:
        changed = False
        for a, lst in evo.items():
            for _, t in lst:
                if a in out and t not in out:
                    out.add(t)
                    changed = True
    return out


def family_key(n):
    return FAMILY.get(n, n)


def legend_const(disp):
    return re.sub(r'[^A-Z0-9]+', '_', disp.upper()).strip('_')


def placements():
    where = {}
    for name, act, lv, m in A:
        for meth, lst in m.items():
            for n in lst:
                where.setdefault(n, []).append((act, name, meth))
    for g, lst in GIFTS.items():
        for n in lst:
            where.setdefault(n, []).append((0, g, 'gift'))
    for biome, theme, m in SAFARI:
        for meth, lst in m.items():
            for n in lst:
                where.setdefault(n, []).append((3, 'Safari Zone: ' + biome, meth))
    return where


def check(sp, forms, base, evo, where):
    errors = []
    for n in where:
        if n not in sp and n not in forms:
            errors.append(f'unknown species {n}')
    # Reachable: placed, evolved by a working (or fixed) method, or bred (the family's baby).
    reach = set(where)
    changed = True
    while changed:
        changed = False
        for a in list(reach):
            for method, t in evo.get(a, []):
                ok = method not in UNIMPLEMENTED and not method.startswith('EVO_SPIN') or (a, t) in EVO_FIXES
                if ok and t not in reach:
                    reach.add(t)
                    changed = True
            b = base.get(a)
            if b and b not in reach:
                reach.add(b)
                changed = True
        for (a, t) in EVO_FIXES:
            if a in reach and t not in reach:
                reach.add(t)
                changed = True
    # Fairness: no family may depend on a 1 % slot alone (the third and later Rare entries).
    fam_of = {}
    for name, act, lv, m in A:
        for meth, lst in m.items():
            for i, n in enumerate(lst):
                one_percent = meth == 'rare' and i >= 2
                fam_of.setdefault(family_key(n), []).append(one_percent)
    for k, flags in fam_of.items():
        if all(flags) and not any(family_key(n) == k for n in where if any(w[2] != 'rare' for w in where[n])):
            errors.append(f'only in 1 % slots: {k}')
    deferred = deferred_species(evo)
    missing = sorted((n for n in sp if n not in reach and n not in deferred), key=sp.get)
    for n in missing:
        errors.append(f'not obtainable: {n}')
    for n in sorted(deferred & set(where)):
        errors.append(f'deferred species placed anyway: {n}')
    # Safari: alternate forms only, and all of them.
    safari = {n for _, _, m in SAFARI for lst in m.values() for n in lst}
    for n in sorted(safari):
        if n in sp:
            errors.append(f'Safari holds a base species: {n}')
    wanted = {c for c in forms if re.search(r'_(ALOLAN|GALARIAN|HISUIAN|PALDEAN)$', c)}
    wanted -= {'DARMANITAN_ZEN_MODE_GALARIAN', 'ARTICUNO_GALARIAN', 'ZAPDOS_GALARIAN', 'MOLTRES_GALARIAN'}
    wanted |= {c for c in forms if re.match(r'(ORICORIO_|SHELLOS_|GASTRODON_|VIVILLON_|FLABEBE_|SQUAWKABILLY_'
                                            r'|TATSUGIRI_|BASCULIN_|TAUROS_|PUMPKABOO_|MINIOR_METEOR)', c)}
    wanted |= {'ROCKRUFF_OWN_TEMPO', 'SINISTEA_ANTIQUE', 'POLTCHAGEIST_MASTERPIECE', 'URSALUNA_BLOODMOON'}
    for n in sorted(wanted - safari):
        errors.append(f'alternate form not in the Safari: {n}')
    # Trade-only evolutions need an item path for solo players.
    for a, lst in evo.items():
        for m, t in lst:
            if m in TRADE and (a, t) not in TRADE_FIXES and not any(
                    mm.startswith('EVO_STONE') or mm == 'EVO_OTHER_PARTY_MON' for mm, tt in lst if tt == t):
                errors.append(f'trade-only evolution {a} -> {t} (needs the item path, plan section 8.1)')
    return errors


ACT_TITLES = {
    1: 'Act 1: New Bark Town to Azalea (badges 0 to 1)',
    2: 'Act 2: Goldenrod to Ecruteak (badges 2 to 3)',
    3: 'Act 3: the open middle (badges 4 to 6: Chuck, Jasmine and Pryce in any order)',
    4: 'Act 4: Mahogany to the Pokémon League (badges 6 to 8)',
    5: 'Act 5: Kanto (open order; leaders, trainers and wild Pokémon add 2 levels per Kanto badge)',
    6: 'Act 6: post-game',
}


def title(n):
    n = n.replace('_ALOLAN', '-Alola').replace('_GALARIAN', '-Galar').replace('_HISUIAN', '-Hisui')
    n = n.replace('_PALDEAN', '-Paldea')
    special = {'FARFETCHD': "Farfetch'd", 'SIRFETCHD': "Sirfetch'd", 'MR_MIME': 'Mr. Mime', 'MIME_JR': 'Mime Jr.',
               'MR_RIME': 'Mr. Rime', 'HO_OH': 'Ho-Oh', 'PORYGON_Z': 'Porygon-Z', 'TYPE_NULL': 'Type: Null',
               'JANGMO_O': 'Jangmo-o', 'HAKAMO_O': 'Hakamo-o', 'KOMMO_O': 'Kommo-o', 'NIDORAN_F': 'Nidoran♀',
               'NIDORAN_M': 'Nidoran♂', 'FLABEBE': 'Flabébé'}
    head, _, tail = n.partition('-')
    t = special.get(head, head.replace('_', ' ').title())
    return t + ('-' + tail if tail else '')


def names(lst):
    return ', '.join(title(n) for n in lst) or '—'


def render(sp, fam, evo, where):
    o = ['# Milestone 4: Pokédex Placement Plan',
         '',
         'Generated by `scripts/rebalance_dex_plan.py --write` from the placement tables in that script.',
         'Do not edit by hand: change the script and regenerate. The design and the rules behind these',
         'tables are in `docs/GAME_REBALANCE_UPDATE.md` (§6–§7).',
         '',
         '**Starters** (decision D6) take a 1 % slot: grass slot 10 or 11, or surf slot 5. A starter appears as',
         'the stage its slot level reaches (a Mudkip slot on Route 44 at Lv. 40 holds Swampert).',
         '',
         '**Radio:** the Hoenn / Sinnoh Sound slots stay as vanilla (user, 2026-10-27) and are not listed here.',
         '',
         '**Slot conventions** (vanilla HGSS, unchanged): grass and caves have 12 slots at',
         '20/20/10/10/10/10/5/5/4/4/1/1 %. *Core* fills the first eight slots (morning and day), *Night*',
         'replaces core slots at night, *Rare* takes the 4 % and 1 % slots. Surfing has 5 slots',
         '(60/30/5/4/1 %), each rod 5, Rock Smash 2, Headbutt trees 12 + 6. The level column is the slot',
         'range; Kanto adds 2 levels per Kanto badge on top (§3.3). Areas that span several vanilla tables',
         '(Routes 16 to 18, 19 to 21, Mt. Moon, Cerulean Cave, Mt. Silver) spread their lists over them.',
         '']
    for act in sorted(ACT_TITLES):
        rows = [a for a in A if a[1] == act]
        if not rows:
            continue
        o += [f'## {ACT_TITLES[act]}', '',
              '| Area | Lv. | Core | Night | Rare | Surf | Fishing | Rock Smash / Headbutt / other |',
              '| :--- | :--- | :--- | :--- | :--- | :--- | :--- | :--- |']
        for name, _, lv, m in rows:
            other = []
            for k in ('starter', 'smash', 'headbutt', 'static', 'gift', 'inside'):
                if m.get(k):
                    other.append(f"*{'starter (1 %)' if k == 'starter' else k}:* {names(m[k])}")
            o.append(f"| {name} | {lv} | {names(m.get('day', []))} | {names(m.get('night', []))} | "
                     f"{names(m.get('rare', []))} | {names(m.get('surf', []))} | {names(m.get('fish', []))} | "
                     f"{'<br>'.join(other) or '—'} |")
        o.append('')
    o += ['## Safari Zone: alternate forms only (Lv. 28–38, no block timers)', '',
          '| Area | Theme | Core | Night | Rare | Water | Vivillon / Minior |',
          '| :--- | :--- | :--- | :--- | :--- | :--- | :--- |']
    for biome, theme, m in SAFARI:
        water = names(m.get('surf', []) + [n for n in m.get('fish', []) if n not in m.get('surf', [])])
        o.append(f"| {biome} | {theme} | {names(m.get('land', []))} | {names(m.get('night', []))} | "
                 f"{names(m.get('rare', []))} | {water} | {names(m.get('vivillon', []) + m.get('minior', []))} |")
    o += ['', '## Gifts', '', '| Source | Pokémon |', '| :--- | :--- |']
    for g, lst in GIFTS.items():
        o.append(f'| {g} | {names(lst)} |')
    o += ['', '## Deferred: legendary, mythical and Ultra Beast encounters', '',
          'Not placed in this milestone (user, 2026-10-27). This is the proposal for the C legend manager',
          '(decision D5), kept for when it is scheduled. Also deferred: the Kubfu, Meltan, Cosmog, Poipole and',
          'Type: Null gifts.', '',
          '| Pokémon | Proposed place | How | Lv. |', '| :--- | :--- | :--- | :--- |']
    for disp, loc, meth, lv in LEGENDS:
        o.append(f"| {disp} | {loc} | {meth} | {lv or '—'} |")
    o += ['', '## Every family, by National Dex number', '',
          'First listed source per family (other members come by evolution or breeding).', '',
          '| # | Family | First source |', '| :--- | :--- | :--- |']
    for k in sorted(fam, key=sp.get):
        best = None
        for n in fam[k]:
            for act, loc, meth in where.get(n, []):
                cand = (act if act else 0.5, f'{loc} ({meth}, as {title(n)})')
                if best is None or cand < best:
                    best = cand
        if best is None:
            # Only reachable by evolving another family's member or form (Yamask-Galar, Meltan).
            edges = [(a, t) for a, lst in evo.items() for _, t in lst] + list(EVO_FIXES)
            for a, t in edges:
                if t in fam[k] and a in where:
                    best = (3, f'{where[a][0][1]} (as {title(a)}, then evolves)')
        if best is None and fam[k][0] in DEFERRED:
            best = (9, 'deferred (legendary / mythical / Ultra Beast)')
        o.append(f"| {sp[k]} | {' / '.join(title(n) for n in fam[k])} | {best[1] if best else '**none**'} |")
    o.append('')
    return '\n'.join(o)


def built_placements():
    """What the built tables hold (--built): grass, water, rods, Rock Smash (no radio or swarm
    fields: those only repeat), Headbutt trees, the Safari; plus the plan's gifts and statics."""
    formname = {}
    cur = None
    n = 0
    for line in open(HG / 'data/PokeFormDataTbl.c'):
        m = re.match(r'\s*\[SPECIES_(\w+)\] = \{', line)
        if m:
            cur, n = m.group(1), 0
            continue
        m = re.match(r'\s*(NEEDS_REVERSION \| )?SPECIES_(\w+),', line)
        if m and cur:
            n += 1
            formname.setdefault((cur, n), m.group(2))

    def names(text):
        out = set()
        for m in re.finditer(r'MON_WITH_FORM\(SPECIES_(\w+), (\d+)\)|SPECIES_(\w+)', text):
            if m.group(1):
                out.add(formname.get((m.group(1), int(m.group(2))), m.group(1)))
            elif m.group(3) != 'NONE':
                out.add(m.group(3))
        return out

    enc = open(HG / 'data/Encounters.c').read()
    enc = re.sub(r'\.(hoennSoundSpecies|sinnohSoundSpecies) = \{[^}]*\}', '', enc)
    enc = re.sub(r'\.(landSwarm|surfSwarm|nightFish|fishSwarm) = SPECIES_\w+', '', enc)
    head = open(HG / 'data/Headbutt.c').read()
    head = head[head.index('= {', head.index('HeadbuttFile'))] if False else head
    where = {}
    for n_ in names(enc) | names(head):
        where.setdefault(n_, []).append((1, 'wild tables', 'built'))
    for n_ in names(open(HG / 'data/SafariEncounters.c').read()):
        where.setdefault(n_, []).append((3, 'Safari Zone', 'built'))
    for g, lst in GIFTS.items():
        for n_ in lst:
            where.setdefault(n_, []).append((0, g, 'gift'))
    for name, act, lv, m in A:
        for n_ in m.get('static', []) + m.get('gift', []):
            where.setdefault(n_, []).append((act, name, 'static'))
    return where


def main():
    sp, forms, base, evo = load_engine()
    fam = families(sp, base, evo)
    FAMILY.update({n: k for k, v in fam.items() for n in v})
    where = built_placements() if '--built' in sys.argv else placements()
    DEFERRED.update(deferred_species(evo))
    errors = check(sp, forms, base, evo, where)
    for e in errors:
        print(e)
    deferred = deferred_species(evo) & set(sp)
    print(f'{len(sp)} species, {len(fam)} families, {len(where)} placed directly, '
          f'{len(deferred)} deferred (legendary / mythical / Ultra Beast), {len(errors)} problems')
    if '--write' in sys.argv:
        OUT.write_text(render(sp, fam, evo, where))
        print(f'wrote {OUT.relative_to(ROOT)}')
    return 1 if errors else 0


if __name__ == '__main__':
    sys.exit(main())
