#!/usr/bin/env python3
"""Turns the avatar sheets into game files (docs/CUSTOM_SPRITES_PLAN.md, Phase 1).

Reads every hg-engine/data/graphics/avatars/<name>.png (sorted by name; custom avatar k has
avatar ID 3 + k) and writes:
  hg-engine/data/graphics/overworlds/custom/avKK_SS.png / .json / -<palette>.pal
      one overworld state per file (KK = avatar, SS = state in the layout's order). hg-engine's
      btx rule builds them into NARC a/0/8/1 members 2_avKK_SS. The json is the vanilla state's,
      byte for byte: the billboard animation looks the texture and palette names up.
  hg-engine/include/coop_avatars_gen.h
      the overworld table entries, the follower offset, and the lookup tables for the ghosts
  coop_avatars.json, build/avatars/*.png
      the bridge's avatar list and previews
Only files whose bytes change are written, so a build with no new art rebuilds nothing. Files
of avatars that are gone are removed (also their built btx0: a stale NARC member would shift
every follower).

Usage: python3 scripts/avatar_build.py            (build_all.sh runs it before make)
       python3 scripts/avatar_build.py --selftest  cut Ethan's guide sheet and compare with vanilla
"""
import argparse
import glob
import io
import json
import os
import re
import sys

from PIL import Image

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import avatar_sheet  # noqa: E402
import avatar_template  # noqa: E402

ROOT = avatar_sheet.ROOT
ENGINE = os.path.join(ROOT, "hg-engine")
GFX = os.path.join(ENGINE, "data", "graphics")
AVATARS = os.path.join(GFX, "avatars")
CUSTOM = os.path.join(GFX, "overworlds", "custom")
BUILT = os.path.join(ENGINE, "build", "pokemonow")
HEADER = os.path.join(ENGINE, "include", "coop_avatars_gen.h")
TABLE = os.path.join(ENGINE, "src", "field", "overworld_table.c")
LIST = os.path.join(ROOT, "coop_avatars.json")
PREVIEWS = os.path.join(ROOT, "build", "avatars")

CUSTOM_GFX_START = 297   # overworld_table.c NEW_NPC_GFX_START
CUSTOM_TAG_START = 7000  # overworld_table.c NEW_NPC_TAG_START
FIRST_CUSTOM_ID = 3      # 0 = the save's own, 1 = Ethan, 2 = Lyra
MAX_CUSTOM = 32
NAME = re.compile(r"^[a-z0-9_]{1,16}$")

# Each overworld column's {hero, heroine} sprite (PlayerAvatar_GetSpriteByStateAndGender and the
# ghost table in coop_ghosts.c), keyed as in the layout.
STATE_SPRITES = {
    "walk": (0, 97), "cycle": (21, 98), "surf": (178, 179), "hm": (176, 177), "fish": (188, 189),
    "save": (198, 199), "heal": (200, 201), "gear": (196, 197), "ladder": (248, 249),
    "water": (180, 181), "shake": (423, 424), "athlon": (407, 408), "rocket": (258, 259),
    "rheal": (260, 261), "rsave": (297, 298),
}


def write_if_changed(path, data):
    if os.path.exists(path):
        with open(path, "rb") as f:
            if f.read() == data:
                return False
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, "wb") as f:
        f.write(data)
    return True


def table_params():
    """{tag: callback_params} from overworld_table.c."""
    out = {}
    pattern = re.compile(r"\.tag\s*=\s*(\d+),\s*\.gfx\s*=\s*\d+,\s*\.callback_params\s*=\s*(0x[0-9A-Fa-f]+|\d+)")
    with open(TABLE) as f:
        for m in pattern.finditer(f.read()):
            out.setdefault(int(m.group(1)), int(m.group(2), 0))
    return out


def indexed(frames, pal0):
    """Stacks RGBA frames into a 4-bit indexed strip: 0 transparent (pal0), 1..n the colours."""
    colours = sorted({p[:3] for f in frames for p in f.get_flattened_data() if p[3]})
    if len(colours) > 15:
        raise SystemExit("internal: %d colours after the reduction" % len(colours))
    palette = [pal0] + colours + [(0, 0, 0)] * (15 - len(colours))
    index = {c: i + 1 for i, c in enumerate(colours)}
    w, h = frames[0].size
    strip = Image.new("P", (w, h * len(frames)), 0)
    strip.putpalette([v for c in palette for v in c])
    px = strip.load()
    for n, f in enumerate(frames):
        src = f.load()
        for y in range(h):
            for x in range(w):
                p = src[x, y]
                if p[3]:
                    px[x, n * h + y] = index[p[:3]]
    return strip, palette


def png_bytes(image, **kw):
    buf = io.BytesIO()
    image.save(buf, "PNG", **kw)
    return buf.getvalue()


def jasc(palette):
    lines = ["JASC-PAL", "0100", "16"] + ["%d %d %d" % c for c in palette]
    return ("\r\n".join(lines) + "\r\n").encode()


def vanilla_source(col):
    """(png path, json path, pal path, palette file name) of a column's vanilla state."""
    png = os.path.join(ENGINE, col["source"])
    base = png[:-4]
    with open(base + ".json") as f:
        pal_name = list(json.load(f)["palettes"].values())[0]["fileName"]
    return png, base + ".json", "%s-%s" % (base, pal_name), pal_name


def read_pal0(path):
    with open(path) as f:
        return tuple(int(v) for v in f.read().split()[3:6])


def cut_overworld(columns, layout, stem):
    """{file stem: bytes} for one avatar's drawn overworld columns; stem e.g. 'av00'."""
    files = {}
    for s, col in enumerate(c for c in layout["columns"] if c["kind"] == "overworld"):
        cut = columns[col["key"]]
        if cut["vanilla"]:
            continue
        png, js, pal, pal_name = vanilla_source(col)
        if Image.open(png).size[1] != col["frame_size"][1] * len(cut["frames"]):
            raise SystemExit("%s: the layout's frame count no longer matches %s" % (col["key"], png))
        strip, palette = indexed(cut["frames"], read_pal0(pal))
        name = "%s_%02d" % (stem, s)
        files[name + ".png"] = png_bytes(strip, bits=4)
        with open(js, "rb") as f:
            files[name + ".json"] = f.read()
        files["%s-%s" % (name, pal_name)] = jasc(palette)
    return files


def front_preview(columns):
    """The bridge's preview: the battle front picture, else the first walking frame."""
    for key in ("front", "walk"):
        c = columns.get(key)
        if c and not c["vanilla"]:
            return c["frames"][0]
    return None


def selftest():
    """Ethan's guide sheet through the cutter: every state must draw exactly like vanilla."""
    layout = avatar_sheet.load_layout()
    columns, _ = avatar_sheet.normalize_sheet(os.path.join(avatar_sheet.TEMPLATE, "avatar_template_guide.png"),
                                              fallback=False)
    files = cut_overworld(columns, layout, "av00")
    bad = 0
    for s, col in enumerate(c for c in layout["columns"] if c["kind"] == "overworld"):
        png, js, pal, pal_name = vanilla_source(col)
        name = "av00_%02d" % s
        mine = Image.open(io.BytesIO(files[name + ".png"]))
        theirs = Image.open(png)
        ok = mine.size == theirs.size and files[name + ".json"] == open(js, "rb").read()
        if ok:
            vp = [tuple(int(v) for v in l.split()) for l in open(pal).read().split("\n")[3:19] if l.strip()]
            mp = [tuple(int(v) for v in l.split()) for l in files["%s-%s" % (name, pal_name)].decode().split("\r\n")[3:19]]
            a, b = mine.load(), theirs.load()
            for y in range(mine.size[1]):
                for x in range(mine.size[0]):
                    ca = None if a[x, y] == 0 else avatar_sheet.ds_colour(mp[a[x, y]])
                    cb = None if b[x, y] == 0 else avatar_sheet.ds_colour(vp[b[x, y]])
                    if ca != cb:
                        ok = False
        print("%-7s %s" % (col["key"], "same as vanilla" if ok else "DIFFERS"))
        bad += not ok
    print("selftest: %s" % ("pass" if not bad else "%d states differ" % bad))
    return bad == 0


def main():
    parser = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    parser.add_argument("--selftest", action="store_true")
    args = parser.parse_args()
    if args.selftest:
        sys.exit(0 if selftest() else 1)

    layout = avatar_sheet.load_layout()
    sheets = sorted(glob.glob(os.path.join(AVATARS, "*.png")))
    if len(sheets) > MAX_CUSTOM:
        raise SystemExit("at most %d custom avatars" % MAX_CUSTOM)
    params = table_params()
    states = [c for c in layout["columns"] if c["kind"] == "overworld"]
    for col in states:
        if col["key"] not in STATE_SPRITES:
            raise SystemExit("no vanilla sprite known for the layout's column %s" % col["key"])

    wanted = {}
    avatars = []
    for k, path in enumerate(sheets):
        name = os.path.basename(path)[:-4]
        if not NAME.match(name):
            raise SystemExit("%s: name avatar sheets in lowercase letters, digits and _ (at most 16)" % path)
        columns, notes = avatar_sheet.normalize_sheet(path)
        for note in notes:
            print("avatar %s: %s" % (name, note))
        files = cut_overworld(columns, layout, "av%02d" % k)
        wanted.update(files)
        drawn = [s for s, col in enumerate(states) if not columns[col["key"]]["vanilla"]]
        avatars.append({"name": name, "id": FIRST_CUSTOM_ID + k, "stem": "av%02d" % k, "drawn": drawn,
                        "preview": front_preview(columns)})

    # The custom folder: ours are av*; anything else there is another custom NPC and keeps its place.
    changed = 0
    for fname, data in wanted.items():
        changed += write_if_changed(os.path.join(CUSTOM, fname), data)
    for old in glob.glob(os.path.join(CUSTOM, "av*")):
        if os.path.basename(old) not in wanted:
            os.remove(old)
            changed += 1
    pngs = sorted(os.path.basename(p)[:-4] for p in glob.glob(os.path.join(CUSTOM, "*.png")))
    for old in glob.glob(os.path.join(BUILT, "2_*.btx0")):
        if os.path.basename(old)[2:-5] not in pngs:
            os.remove(old)  # a stale member would shift every follower
            changed += 1

    # The header: table entries for every av* file, by its place among all the custom PNGs.
    entries, tags, base = [], [], []
    for a in avatars:
        row = []
        for s, col in enumerate(states):
            if s not in a["drawn"]:
                row.append(0)
                continue
            n = pngs.index("%s_%02d" % (a["stem"], s))
            hero = STATE_SPRITES[col["key"]][0]
            entries.append("    { .tag = %d, .gfx = %d, .callback_params = 0x%04X }, /* %s %s */"
                           % (CUSTOM_TAG_START + n, CUSTOM_GFX_START + n, params[hero], a["name"], col["key"]))
            row.append(CUSTOM_TAG_START + n)
            base.append((CUSTOM_TAG_START + n, hero))
        tags.append("    { %s }, /* %d %s */" % (", ".join(str(t) for t in row), a["id"], a["name"]))
    first = min((t for t, _ in base), default=CUSTOM_TAG_START)
    span = (max(t for t, _ in base) - first + 1) if base else 0
    base_row = [0xFFFF] * span  # 0xFFFF: not an avatar tag (0 is Ethan's walking sprite)
    for t, hero in base:
        base_row[t - first] = hero
    h = ["// Generated by scripts/avatar_build.py from hg-engine/data/graphics/avatars/*.png: do not edit.",
         "// docs/CUSTOM_SPRITES_PLAN.md. Avatar IDs: 0 the save's own, 1 Ethan, 2 Lyra, 3 + k custom.",
         "#ifndef COOP_AVATARS_GEN_H", "#define COOP_AVATARS_GEN_H", "",
         "// Every PNG in data/graphics/overworlds/custom (NARC a/0/8/1 from 297): the followers come after.",
         "#define COOP_CUSTOM_OW_FILES %d" % len(pngs),
         "#define COOP_AVATAR_STATES %d" % len(states),
         "#define COOP_AVATAR_CUSTOM %d" % len(avatars),
         "#define COOP_AVATAR_FIRST_CUSTOM_ID %d" % FIRST_CUSTOM_ID, "",
         "// gOWTagToFileNum entries: the vanilla state's billboard (callback_params), the avatar's texture.",
         "#define COOP_AVATAR_OW_ENTRIES \\"]
    h += [e + " \\" for e in entries] + [""]
    h += ["// {hero, heroine} sprite of each state, in the tables' order.",
          "#define COOP_AVATAR_STATE_SPRITES { %s }" % ", ".join(
              "{ %d, %d }" % STATE_SPRITES[c["key"]] for c in states), ""]
    if avatars:
        h += ["// Per custom avatar, its tag in each state (0: the vanilla sprite).",
              "#define COOP_AVATAR_TAGS { \\"] + [t + " \\" for t in tags] + ["}", ""]
    if base:
        h += ["// Per custom tag from COOP_AVATAR_FIRST_TAG, the hero sprite it stands in for (0xFFFF: none).",
              "#define COOP_AVATAR_FIRST_TAG %d" % first,
              "#define COOP_AVATAR_TAG_SPAN %d" % span,
              "#define COOP_AVATAR_TAG_BASE { %s }" % ", ".join(str(v) for v in base_row), ""]
    h += ["#endif // COOP_AVATARS_GEN_H", ""]
    changed += write_if_changed(HEADER, "\n".join(h).encode())

    # The bridge's list and previews.
    os.makedirs(PREVIEWS, exist_ok=True)
    listing = [{"id": 0, "name": "Default", "note": "Ethan or Lyra, by the save"}]
    for vid, vname, pic in ((1, "Ethan", "trainer_gfx/000"), (2, "Lyra", "trainer_gfx/001")):
        frames, _ = avatar_template.battle_frames(pic)
        write_if_changed(os.path.join(PREVIEWS, vname.lower() + ".png"), png_bytes(frames[0]))
        listing.append({"id": vid, "name": vname, "preview": "build/avatars/%s.png" % vname.lower()})
    for a in avatars:
        entry = {"id": a["id"], "name": a["name"].replace("_", " ").title(), "file": a["name"]}
        if a["preview"] is not None:
            write_if_changed(os.path.join(PREVIEWS, a["name"] + ".png"), png_bytes(a["preview"]))
            entry["preview"] = "build/avatars/%s.png" % a["name"]
        listing.append(entry)
    changed += write_if_changed(LIST, (json.dumps({"version": 1, "avatars": listing}, indent=1) + "\n").encode())
    print("avatars: %d custom (%s), %d overworld files, %d changed"
          % (len(avatars), ", ".join(a["name"] for a in avatars) or "none", len(pngs), changed))


if __name__ == "__main__":
    main()
