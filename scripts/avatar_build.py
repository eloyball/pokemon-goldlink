#!/usr/bin/env python3
"""Turns the avatar sheets into game files (docs/CUSTOM_SPRITES_PLAN.md, Phase 1).

Reads every hg-engine/data/graphics/avatars/<name>.png (sorted by name; custom avatar k has
avatar ID 3 + k) and writes:
  hg-engine/data/graphics/overworlds/custom/avKK_SS.png / .json / -<palette>.pal
      one overworld state per file (KK = avatar, SS = state in the layout's order). hg-engine's
      btx rule builds them into NARC a/0/8/1 members 2_avKK_SS. The json is the vanilla state's,
      byte for byte: the billboard animation looks the texture and palette names up.
  hg-engine/data/graphics/cutin/<NN>.png
      the HM cut-in's trainer frames of each avatar whose HM column is drawn (KNOWN_RISKS #82):
      narcs.mk adds them to NARC a/0/9/3 as members NN (the frames) and NN + 1 (the palette).
  hg-engine/data/graphics/vsface/<NNN>.png
      the VS face of each avatar that has one (Phase 6, the PvP battle intro), in the game's tile
      order: narcs.mk adds it to NARC a/1/0/9 as members NNN (palette), NNN + 1 (characters) and
      Ethan's cell and animation (NNN + 2, NNN + 3).
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
CUTIN = os.path.join(GFX, "cutin")
CUTIN_NARC = os.path.join(ENGINE, "build", "narc", "a093.narc")
VSFACE = os.path.join(GFX, "vsface")
VSFACE_NARC = os.path.join(ENGINE, "build", "narc", "a109.narc")
PREVIEWS = os.path.join(ROOT, "build", "avatars")

CUSTOM_GFX_START = 297   # overworld_table.c NEW_NPC_GFX_START
CUSTOM_TAG_START = 7000  # overworld_table.c NEW_NPC_TAG_START
FIRST_CUSTOM_ID = 3      # 0 = the save's own, 1 = Ethan, 2 = Lyra
MAX_CUSTOM = 32
CUTIN_FIRST_MEMBER = 26  # NARC a/0/9/3 (choose_starter_sub_res) has 26 members; narcpy names them 3_00..
# The HM cut-in's 10 trainer frames (vanilla's Ethan set, member 17), from the overworld columns
# (zero-based frames; checked shape by shape against the vanilla char).
# NARC a/1/0/9 (the encounter effects) has 242 members; narcpy names them 9_000..
VSFACE_FIRST_MEMBER = 242
# Ethan's VS face cell (a/1/0/9 member 209): (x, y, width, height, first tile) of its two OAM
# pieces in the 96x64 face, tiles in 1D order (one per character name). --selftest checks it.
VSFACE_OAMS = [(0, 0, 64, 64, 0), (64, 0, 32, 64, 64)]
VSFACE_PAL0 = (197, 189, 156)  # Ethan's face's colour 0 (transparent, 0x4EF8), as nitrogfx decodes it
CUTIN_FRAMES = [("walk", 0), ("walk", 21), ("surf", 0), ("surf", 1), ("walk", 7), ("walk", 11),
                ("hm", 0), ("hm", 1), ("hm", 2), ("hm", 3)]
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


# Battle pictures (Phase 5): (column, vanilla base, output folder, suffix). Ethan's pictures are the
# models: their cell / anim json and encryption key are copied, the pixels are the avatar's.
BATTLE_PICS = [
    ("front", "trainer_gfx/000", "trainer_gfx", ""),
    ("throw", "trainer_back_gfx/00", "trainer_back_gfx", ""),
    ("throw_link", "trainer_back_gfx/15", "trainer_back_gfx", "l"),
]


def pack_picture(frames, base):
    """The inverse of avatar_template.battle_frames: each 80x80 frame's OAM pieces back into the
    picture's tile order (frame i in its own 100-tile block). Returns (indexed image, palette)."""
    raw = Image.open(os.path.join(GFX, base + ".png"))
    with open(os.path.join(GFX, base + "_cell.json")) as f:
        cells = json.load(f)["cells"]
    if len(cells) != len(frames):
        raise SystemExit("%s: %d cells, %d frames" % (base, len(cells), len(frames)))
    pal0 = tuple(raw.getpalette()[:3])
    strip, palette = indexed(frames, pal0)  # one palette for the picture, frames stacked
    w = avatar_template.BW
    src = strip.load()
    out = Image.new("P", raw.size, 0)
    out.putpalette(strip.getpalette())
    dst = out.load()
    per_row = raw.size[0] // 8
    for index, cell in enumerate(cells):
        for oam in cell["OAM"]:
            ow, oh = avatar_template.OAM_SIZES[(oam["Attr0"]["Shape"], oam["Attr1"]["Size"])]
            first = index * (w * w // 64) + oam["Attr2"]["CharName"] * 2
            x0 = oam["Attr1"]["XCoordinate"] - cell["minX"]
            y0 = oam["Attr0"]["YCoordinate"] - cell["minY"]
            k = 0
            for ty in range(oh // 8):
                for tx in range(ow // 8):
                    t = first + k
                    k += 1
                    tx0, ty0 = (t % per_row) * 8, (t // per_row) * 8
                    if ty0 + 8 > raw.size[1]:
                        continue
                    for yy in range(8):
                        for xx in range(8):
                            fx, fy = x0 + tx * 8 + xx, y0 + ty * 8 + yy
                            v = 0
                            if 0 <= fx < w and 0 <= fy < w:
                                v = src[fx, index * w + fy]
                            dst[tx0 + xx, ty0 + yy] = v
    return out, palette


def encounter_image(frames, palette):
    """The _enc image: frames 0 and 1 side by side (a single picture: the second half empty)."""
    w = avatar_template.BW
    strip, _ = indexed(frames[:2], palette[0])
    # Re-index with the picture's palette (the same colours, maybe another order).
    index = {c: i for i, c in enumerate(palette) if i}
    enc = Image.new("P", (2 * w, w), 0)
    enc.putpalette([v for c in palette for v in c])
    px = enc.load()
    for n, f in enumerate(frames[:2]):
        fp = f.load()
        for y in range(w):
            for x in range(w):
                p = fp[x, y]
                if p[3]:
                    px[n * w + x, y] = index[p[:3]]
    return enc


def cut_battle(columns, stem):
    """{relative path under data/graphics: bytes} for one avatar's drawn battle pictures."""
    files = {}
    for key, base, folder, suffix in BATTLE_PICS:
        cut = columns.get(key)
        if cut is None or cut["vanilla"]:
            continue
        pic, palette = pack_picture(cut["frames"], base)
        name = "%s/%s%s" % (folder, stem, suffix)
        files[name + ".png"] = png_bytes(pic, bits=4)
        files[name + "_enc.png"] = png_bytes(encounter_image(cut["frames"], palette), bits=4)
        for ext in ("_cell.json", "_anim.json", "_enc.png.key"):
            with open(os.path.join(GFX, base + ext), "rb") as f:
                files[name + ext] = f.read()
    return files


def cut_cutin(columns, layout):
    """The HM cut-in's frames (KNOWN_RISKS #82) as a 4-bit 32x320 strip with its own palette
    (the cut-in loads one palette for all 10), or None when the HM column isn't drawn (the
    save's own cut-in stays). An undrawn walking or surfing column lends Ethan's frames."""
    if columns["hm"]["vanilla"]:
        return None, []
    cols = {c["key"]: c for c in layout["columns"]}
    guide = None
    frames = []
    for key, j in CUTIN_FRAMES:
        if columns[key]["vanilla"]:
            if guide is None:
                guide = Image.open(os.path.join(avatar_sheet.TEMPLATE, "avatar_template_guide.png")).convert("RGBA")
            x, y, w, h = cols[key]["frames"][j]
            frames.append(avatar_sheet.opaque_or_clear(guide.crop((x, y, x + w, y + h))))
        else:
            frames.append(columns[key]["frames"][j].copy())
    counts = {}
    for f in frames:
        for p in f.get_flattened_data():
            if p[3]:
                c = avatar_sheet.ds_colour(p[:3])
                counts[c] = counts.get(c, 0) + 1
    merged, merges = avatar_sheet.reduce_colours(counts)
    for f in frames:
        px = f.load()
        for yy in range(f.size[1]):
            for xx in range(f.size[0]):
                p = px[xx, yy]
                if p[3]:
                    px[xx, yy] = merged[avatar_sheet.ds_colour(p[:3])] + (255,)
    notes = []
    if merges:
        notes.append("HM cut-in: walking, surfing and HM share one palette: %d colours reduced to %d, %d px changed"
                     % (len(counts), len(set(merged.values())), sum(n for _, _, n in merges)))
    strip, _ = indexed(frames, (0, 0, 0))
    return png_bytes(strip, bits=4), notes


def pack_vsface(face, pal0):
    """A 96x64 RGBA face in the game's tile order: an indexed 8x768 strip, tile t at y = 8t."""
    pic, palette = indexed([face], pal0)
    src = pic.load()
    out = Image.new("P", (8, 8 * sum(w * h // 64 for _, _, w, h, _ in VSFACE_OAMS)), 0)
    out.putpalette(pic.getpalette())
    dst = out.load()
    for x0, y0, w, h, first in VSFACE_OAMS:
        for t in range(w * h // 64):
            tx, ty = x0 + (t % (w // 8)) * 8, y0 + (t // (w // 8)) * 8
            for yy in range(8):
                for xx in range(8):
                    dst[xx, (first + t) * 8 + yy] = src[tx + xx, ty + yy]
    return out, palette


def cut_vsface(columns):
    """The VS face's PNG for narcs.mk, or None when the column isn't drawn (the save's own face)."""
    col = columns.get("vs_face")
    if col is None or col["vanilla"]:
        return None
    pic, _ = pack_vsface(col["frames"][0], VSFACE_PAL0)
    return png_bytes(pic, bits=4)


_MAP_HEAD_PALETTE = []


def cut_map_head(columns):
    """The Pokegear map head (Milestone 6 §6.2) as 128 bytes of 4bpp characters in the game's order
    (16x16, 1D: top left, top right, bottom left, bottom right), in the map's own head palette;
    None when the column isn't drawn (Ethan's or Lyra's head). Also returns notes."""
    col = columns.get("map_head")
    if col is None or col["vanilla"]:
        return None, []
    if not _MAP_HEAD_PALETTE:
        _MAP_HEAD_PALETTE.extend(avatar_template.map_head_palette())
    pal = [avatar_sheet.ds_colour(c) for c in _MAP_HEAD_PALETTE]
    head = col["frames"][0]
    px = head.load()
    index = [[0] * 16 for _ in range(16)]
    off = 0
    for y in range(16):
        for x in range(16):
            p = px[x, y]
            if not p[3]:
                continue
            rgb = avatar_sheet.ds_colour(p[:3])
            best = min(range(15), key=lambda i: avatar_sheet.distance(rgb, pal[i]))
            if pal[best] != rgb:
                off += 1
            index[y][x] = best + 1
    data = bytearray()
    for t in range(4):
        tx, ty = (t % 2) * 8, (t // 2) * 8
        for y in range(8):
            for x in range(0, 8, 2):
                data.append(index[ty + y][tx + x] | (index[ty + y][tx + x + 1] << 4))
    notes = ["map head: %d px not in the map's palette, moved to the nearest colour" % off] if off else []
    return bytes(data), notes


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
    # The battle pictures: Ethan's frames packed back must draw like the vanilla files.
    battle = cut_battle(columns, "av00")
    for key, base, folder, suffix in BATTLE_PICS:
        name = "%s/av00%s" % (folder, suffix)
        for mine_name, theirs_path in ((name + ".png", base + ".png"), (name + "_enc.png", base + "_enc.png")):
            mine = Image.open(io.BytesIO(battle[mine_name]))
            theirs = Image.open(os.path.join(GFX, theirs_path))
            ok = mine.size == theirs.size
            if ok:
                mp, tp = mine.getpalette(), theirs.getpalette()
                a, b = mine.load(), theirs.load()
                diff = 0
                for y in range(mine.size[1]):
                    for x in range(mine.size[0]):
                        ca = None if a[x, y] == 0 else avatar_sheet.ds_colour(tuple(mp[a[x, y] * 3:a[x, y] * 3 + 3]))
                        cb = None if b[x, y] == 0 else avatar_sheet.ds_colour(tuple(tp[b[x, y] * 3:b[x, y] * 3 + 3]))
                        diff += ca != cb
                ok = diff == 0
                detail = "" if ok else " (%d px differ)" % diff
            else:
                detail = " (size %s vs %s)" % (mine.size, theirs.size)
            print("%-28s %s%s" % (mine_name, "same as vanilla" if ok else "DIFFERS", detail))
            bad += not ok
    # The VS face: Ethan's packed back must be his characters, tile for tile, colour for colour.
    raw, cell = avatar_template.vs_face_raw()
    oams = [(o["Attr1"]["XCoordinate"] - cell["minX"], o["Attr0"]["YCoordinate"] - cell["minY"],
             avatar_template.OAM_SIZES[(o["Attr0"]["Shape"], o["Attr1"]["Size"])][0],
             avatar_template.OAM_SIZES[(o["Attr0"]["Shape"], o["Attr1"]["Size"])][1],
             o["Attr2"]["CharName"]) for o in cell["OAM"]]
    mine, _ = pack_vsface(columns["vs_face"]["frames"][0], tuple(raw.getpalette()[:3]))
    ok = oams == VSFACE_OAMS and mine.size == raw.size
    diff = 0
    if ok:
        mp, tp = mine.getpalette(), raw.getpalette()
        a, b = mine.load(), raw.load()
        for y in range(mine.size[1]):
            for x in range(mine.size[0]):
                ca = None if a[x, y] == 0 else avatar_sheet.ds_colour(tuple(mp[a[x, y] * 3:a[x, y] * 3 + 3]))
                cb = None if b[x, y] == 0 else avatar_sheet.ds_colour(tuple(tp[b[x, y] * 3:b[x, y] * 3 + 3]))
                diff += ca != cb
        ok = diff == 0
    print("%-28s %s%s" % ("vsface", "same as vanilla" if ok else "DIFFERS",
                          "" if ok else " (cell %s, %d px differ)" % (oams, diff)))
    bad += not ok
    if tuple(raw.getpalette()[:3]) != VSFACE_PAL0:
        print("vsface colour 0 %s, VSFACE_PAL0 %s" % (tuple(raw.getpalette()[:3]), VSFACE_PAL0))
        bad += 1
    print("selftest: %s" % ("pass" if not bad else "%d differ" % bad))
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
    battle_wanted = {}  # relative to data/graphics
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
        battle_wanted.update(cut_battle(columns, "av%02d" % k))
        drawn = [s for s, col in enumerate(states) if not columns[col["key"]]["vanilla"]]
        cutin, cutin_notes = cut_cutin(columns, layout)
        for note in cutin_notes:
            print("avatar %s: %s" % (name, note))
        head, head_notes = cut_map_head(columns)
        for note in head_notes:
            print("avatar %s: %s" % (name, note))
        avatars.append({"name": name, "id": FIRST_CUSTOM_ID + k, "stem": "av%02d" % k, "drawn": drawn,
                        "preview": front_preview(columns), "cutin": cutin, "vsface": cut_vsface(columns),
                        "map_head": head})

    # The custom folder: ours are av*; anything else there is another custom NPC and keeps its place.
    changed = 0
    for fname, data in wanted.items():
        changed += write_if_changed(os.path.join(CUSTOM, fname), data)
    for old in glob.glob(os.path.join(CUSTOM, "av*")):
        if os.path.basename(old) not in wanted:
            os.remove(old)
            changed += 1
    pngs = sorted(os.path.basename(p)[:-4] for p in glob.glob(os.path.join(CUSTOM, "*.png")))
    repack = set()  # packed NARCs that lost a member: make only repacks for newer inputs, never for removed ones

    # Battle pictures: NARC a/0/5/8 (fronts) and a/0/0/6 (backs) take one member set per *_enc.png,
    # in name order, so av* come after the numbered vanilla ones.
    for rel, data in battle_wanted.items():
        changed += write_if_changed(os.path.join(GFX, rel), data)
    pic_index = {}
    for folder, built, prefix in (("trainer_gfx", "trainer_gfx", "8_"), ("trainer_back_gfx", "trainer_back_gfx", "6_")):
        for old in glob.glob(os.path.join(GFX, folder, "av*")):
            if os.path.relpath(old, GFX) not in battle_wanted:
                os.remove(old)
                changed += 1
        stems = sorted(os.path.basename(p)[:-8] for p in glob.glob(os.path.join(GFX, folder, "*_enc.png")))
        for i, stem in enumerate(stems):
            pic_index[(folder, stem)] = i
        for old in glob.glob(os.path.join(ENGINE, "build", built, prefix + "av*")):
            stem = os.path.basename(old)[len(prefix):].split("-")[0]
            if stem not in stems:
                os.remove(old)  # a stale member would shift nothing (av* sort last) but would still be packed
                repack.add(os.path.join(ENGINE, "build", "narc", built + ".narc"))
                changed += 1
    for old in glob.glob(os.path.join(BUILT, "2_*.btx0")):
        if os.path.basename(old)[2:-5] not in pngs:
            os.remove(old)  # a stale member would shift every follower
            repack.add(os.path.join(ENGINE, "build", "narc", "pokemonow.narc"))
            changed += 1
    # Removing an input doesn't make make repack a NARC (its rule only sees newer inputs): a NARC
    # still holding removed overworld members put every follower on the wrong file (a two-frame
    # avatar texture for Geodude: the follower's animation crashed the game; KNOWN_RISKS #77).
    for narc in repack:
        if os.path.exists(narc):
            os.remove(narc)

    # The HM cut-in (KNOWN_RISKS #82): one PNG per avatar that has one, named by its NARC member.
    # narcs.mk rebuilds a/0/9/3 from vanilla's 26 members every time, so a removed PNG only needs
    # the archive rebuilt.
    cutin_wanted = {}
    for a in avatars:
        a["cutin_member"] = 0xFFFF
        if a["cutin"] is not None:
            a["cutin_member"] = CUTIN_FIRST_MEMBER + 2 * len(cutin_wanted)
            cutin_wanted["%d.png" % a["cutin_member"]] = a["cutin"]
    if CUTIN_FIRST_MEMBER + 2 * len(cutin_wanted) > 100:
        raise SystemExit("internal: a/0/9/3 member names would need three digits")
    for fname, data in cutin_wanted.items():
        changed += write_if_changed(os.path.join(CUTIN, fname), data)
    for old in glob.glob(os.path.join(CUTIN, "*.png")):
        if os.path.basename(old) not in cutin_wanted:
            os.remove(old)
            changed += 1
            if os.path.exists(CUTIN_NARC):
                os.remove(CUTIN_NARC)

    # The VS face (Phase 6): one PNG per avatar that has one, named by its first NARC member.
    # narcs.mk rebuilds a/1/0/9 from vanilla's 242 members every time, as the cut-in's.
    vsface_wanted = {}
    for a in avatars:
        a["vsface_member"] = 0xFFFF
        if a["vsface"] is not None:
            a["vsface_member"] = VSFACE_FIRST_MEMBER + 4 * len(vsface_wanted)
            vsface_wanted["%d.png" % a["vsface_member"]] = a["vsface"]
    if VSFACE_FIRST_MEMBER + 4 * len(vsface_wanted) > 1000:
        raise SystemExit("internal: a/1/0/9 member names would need four digits")
    for fname, data in vsface_wanted.items():
        changed += write_if_changed(os.path.join(VSFACE, fname), data)
    for old in glob.glob(os.path.join(VSFACE, "*.png")):
        if os.path.basename(old) not in vsface_wanted:
            os.remove(old)
            changed += 1
            if os.path.exists(VSFACE_NARC):
                os.remove(VSFACE_NARC)

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
        a["pics"] = [pic_index.get(("trainer_gfx", a["stem"]), 0xFFFF),
                     pic_index.get(("trainer_back_gfx", a["stem"]), 0xFFFF),
                     pic_index.get(("trainer_back_gfx", a["stem"] + "l"), 0xFFFF)]
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
    if avatars:
        h += ["// Battle pictures per custom avatar (Phase 5): { front (NARC a/0/5/8 set), throw, link throw",
              "// (a/0/0/6 sets) }, 0xFFFF where the column wasn't drawn (Ethan's picture is used).",
              "#define COOP_AVATAR_PICS { \\"]
        h += ["    { %d, %d, %d }, /* %d %s */ \\" % (a["pics"][0], a["pics"][1], a["pics"][2], a["id"], a["name"])
              for a in avatars] + ["}", ""]
        h += ["// The HM cut-in's trainer frames per custom avatar (KNOWN_RISKS #82): NARC a/0/9/3 member of",
              "// the frames (the palette is the next one), 0xFFFF where the HM column wasn't drawn.",
              "#define COOP_AVATAR_CUTIN { %s }" % ", ".join("%d" % a["cutin_member"] for a in avatars), ""]
        h += ["// The VS face per custom avatar (Phase 6, the PvP intro): NARC a/1/0/9 member of its palette",
              "// (characters, cell, animation follow), 0xFFFF where the VS face wasn't drawn.",
              "#define COOP_AVATAR_VSFACE { %s }" % ", ".join("%d" % a["vsface_member"] for a in avatars), ""]
        heads = [a for a in avatars if a["map_head"] is not None]
        h += ["// The Pokegear map head per custom avatar (Milestone 6 §6.2): its row in",
              "// COOP_AVATAR_MAP_HEAD_TILES, 0xFF where the column wasn't drawn (Ethan's or Lyra's head).",
              "#define COOP_AVATAR_MAP_HEAD { %s }" % ", ".join(
                  "%d" % heads.index(a) if a["map_head"] is not None else "0xFF" for a in avatars), ""]
        if heads:
            h += ["// 16x16 4bpp characters (top left, top right, bottom left, bottom right) in the map's head",
                  "// palette (pret pgmap_gra, NARC a/1/4/4, palette 0).",
                  "#define COOP_AVATAR_MAP_HEAD_TILES { \\"]
            for a in heads:
                hexes = ", ".join("0x%02X" % b for b in a["map_head"])
                h.append("    { %s }, /* %d %s */ \\" % (hexes, a["id"], a["name"]))
            h += ["}", ""]
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
