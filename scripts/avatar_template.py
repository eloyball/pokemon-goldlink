#!/usr/bin/env python3
"""Builds the custom avatar drawing template (one PNG per avatar).

Writes to hg-engine/data/graphics/avatars/template/:
  avatar_template.png        the canvas to draw on: transparent cells, grey gutters and labels
  avatar_template_guide.png  the same, with Ethan's frames in every cell (full opacity): a base to
                             trace over or to edit directly
  avatar_layout.json         every frame's rectangle, and each battle frame's in-game offset: the
                             contract the cutter reads
(README.md beside them holds the drawing rules.)

Layout: one column per state, frames stacked from the top in the vanilla order.
- Overworld columns: 32x32 frames.
- Battle front and throws: 80x80 frames, each stored unshifted. The game draws some throw frames
  shifted (the arm reaching forward); that shift is in the layout (offset).
- VS face: one 96x64 picture (the PvP battle intro, built on the Elite Four / Champion one, NARC a/1/0/9).
The header, the gutters and the gaps between frames are ignored by the cutter.

Usage: python3 scripts/avatar_template.py   (run from anywhere; uses hg-engine's own tools)
"""
import json
import os
import shutil
import subprocess
import tempfile

from PIL import Image, ImageDraw, ImageFont

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
ENGINE = os.path.join(ROOT, "hg-engine")
GFX = os.path.join(ENGINE, "data", "graphics")
OUT = os.path.join(GFX, "avatars", "template")

# (key, vanilla source, label, priority). Ethan's layouts: every avatar follows them.
OVERWORLD = [
    ("walk", "overworlds/0069.png", "Walk / run", "must"),
    ("cycle", "overworlds/0071.png", "Cycling", "must"),
    ("surf", "overworlds/0073.png", "Surfing", "must"),
    ("hm", "overworlds/0075.png", "HM use", "should"),
    ("fish", "overworlds/0079.png", "Fishing", "should"),
    ("save", "overworlds/0087.png", "Saving", "should"),
    ("heal", "overworlds/0089.png", "Healing", "should"),
    ("gear", "overworlds/0085.png", "Pokegear", "should"),
    ("ladder", "overworlds/0081.png", "Ladder", "should"),
    ("water", "overworlds/0077.png", "Watering", "optional"),
    ("shake", "overworlds/0207.png", "Apricorn shake", "optional"),
    ("athlon", "overworlds/0099.png", "Pokeathlon", "optional"),
    ("rocket", "overworlds/0095.png", "Rocket disguise", "optional"),
    ("rheal", "overworlds/0097.png", "Rocket healing", "optional"),
    ("rsave", "overworlds/0248.png", "Rocket saving", "optional"),
]
# (key, cell-data base, label, priority): battle pictures are cut into OAM pieces in the game
# and are reassembled here from their cell data.
BATTLE = [
    ("front", "trainer_gfx/000", "Battle front", "must"),
    ("throw", "trainer_back_gfx/00", "Throw", "must"),
    ("throw_link", "trainer_back_gfx/15", "Throw (link)", "optional"),
]
# The player's face in the Elite Four / Champion intro (ccd encounter_effect_leaders.c: Ethan's
# palette, pixels, cells at members 207-209 of NARC a/1/0/9).
VS_FACE = ("vs_face", "a/1/0/9", 207, "VS face", "should")

OW, BW = 32, 80       # overworld and battle frame sizes
GUTTER = 16           # between columns: frame numbers go here
GAP = 2               # between frames of a column: the cell borders go here
HEADER = 162          # labels (written vertically)
MARGIN = 4
BG = (216, 216, 216, 255)
LINE = (150, 150, 150, 255)
TEXT = (40, 40, 40, 255)
MUST = (176, 32, 32, 255)

OAM_SIZES = {(0, 0): (8, 8), (0, 1): (16, 16), (0, 2): (32, 32), (0, 3): (64, 64),
             (1, 0): (16, 8), (1, 1): (32, 8), (1, 2): (32, 16), (1, 3): (64, 32),
             (2, 0): (8, 16), (2, 1): (8, 32), (2, 2): (16, 32), (2, 3): (32, 64)}


def to_rgba(indexed):
    """Palette index 0 is transparent."""
    rgba = indexed.convert("RGBA")
    px, src = rgba.load(), indexed.load()
    for y in range(indexed.size[1]):
        for x in range(indexed.size[0]):
            if src[x, y] == 0:
                px[x, y] = (0, 0, 0, 0)
    return rgba


def strip_frames(path, size):
    raw = Image.open(os.path.join(GFX, path))
    return [to_rgba(raw.crop((0, i * size, size, (i + 1) * size))) for i in range(raw.size[1] // size)]


def tiles_of(raw):
    per_row = raw.size[0] // 8
    return [raw.crop(((t % per_row) * 8, (t // per_row) * 8, (t % per_row) * 8 + 8, (t // per_row) * 8 + 8))
            for t in range(per_row * (raw.size[1] // 8))]


def place(frame, tiles, oam, first, x0, y0):
    w, h = OAM_SIZES[(oam["Attr0"]["Shape"], oam["Attr1"]["Size"])]
    k = 0
    for ty in range(h // 8):
        for tx in range(w // 8):
            if first + k < len(tiles):
                frame.paste(tiles[first + k], (x0 + tx * 8, y0 + ty * 8))
            k += 1


def battle_frames(base):
    """A battle picture's frames, each in its own 80x80 box, with the offset the game draws it
    at (relative to an 80x80 box centred on the picture's anchor). Each cell takes its own
    100-tile block (VRAM transfer)."""
    raw = Image.open(os.path.join(GFX, base + ".png"))
    cells = json.load(open(os.path.join(GFX, base + "_cell.json")))["cells"]
    tiles = tiles_of(raw)
    frames, offsets = [], []
    for index, cell in enumerate(cells):
        frame = Image.new("P", (BW, BW), 0)
        frame.putpalette(raw.getpalette())
        for oam in cell["OAM"]:
            place(frame, tiles, oam, index * (BW * BW // 64) + oam["Attr2"]["CharName"] * 2,
                  oam["Attr1"]["XCoordinate"] - cell["minX"], oam["Attr0"]["YCoordinate"] - cell["minY"])
        frames.append(to_rgba(frame))
        offsets.append([cell["minX"] + BW // 2, cell["minY"] + BW // 2])
    return frames, offsets


def vs_face():
    """Ethan's VS face, reassembled from its OAM pieces."""
    raw, cell = vs_face_raw()
    w, h = cell["maxX"] - cell["minX"] + 1, cell["maxY"] - cell["minY"] + 1
    face = Image.new("P", (w, h), 0)
    face.putpalette(raw.getpalette())
    tiles = tiles_of(raw)
    for oam in cell["OAM"]:  # 1D mapping, one tile per character name
        place(face, tiles, oam, oam["Attr2"]["CharName"],
              oam["Attr1"]["XCoordinate"] - cell["minX"], oam["Attr0"]["YCoordinate"] - cell["minY"])
    return to_rgba(face)


def vs_face_raw():
    """Ethan's VS face as the game stores it: (its characters as an indexed 8-pixel-wide tile
    strip, its cell), unpacked with hg-engine's narcpy and nitrogfx into a temporary folder."""
    _, narc, first, _, _ = VS_FACE
    tmp = tempfile.mkdtemp(prefix="avatar_vs_")
    try:
        subprocess.run(["python3", os.path.join(ENGINE, "tools", "narcpy.py"), "extract",
                        os.path.join(ENGINE, "base", "root", narc), "-o", tmp, "-nf"],
                       check=True, capture_output=True)
        member = lambda n: os.path.join(tmp, "9_%d" % n)
        for n, ext in ((first, "NCLR"), (first + 1, "NCGR"), (first + 2, "NCER")):
            shutil.copy(member(n), os.path.join(tmp, "face." + ext))
        gfx = os.path.join(ENGINE, "tools", "nitrogfx")
        subprocess.run([gfx, os.path.join(tmp, "face.NCGR"), os.path.join(tmp, "face.png"), "-palette",
                        os.path.join(tmp, "face.NCLR")], check=True, capture_output=True)
        subprocess.run([gfx, os.path.join(tmp, "face.NCER"), os.path.join(tmp, "face.json")],
                       check=True, capture_output=True)
        raw = Image.open(os.path.join(tmp, "face.png"))
        raw.load()
        cell = json.load(open(os.path.join(tmp, "face.json")))["cells"][0]
        return raw, cell
    finally:
        shutil.rmtree(tmp, ignore_errors=True)


def main():
    columns = []
    for key, src, label, prio in OVERWORLD:
        frames = strip_frames(src, OW)
        columns.append({"key": key, "label": label, "priority": prio, "kind": "overworld",
                        "source": "data/graphics/" + src, "w": OW, "h": OW, "frames": frames,
                        "offsets": None})
    for key, base, label, prio in BATTLE:
        frames, offsets = battle_frames(base)
        columns.append({"key": key, "label": label, "priority": prio,
                        "kind": "battle_front" if key == "front" else "battle_back",
                        "source": "data/graphics/" + base + ".png", "w": BW, "h": BW, "frames": frames,
                        "offsets": offsets})
    key, narc, first, label, prio = VS_FACE
    face = vs_face()
    columns.append({"key": key, "label": label, "priority": prio, "kind": "vs_face",
                    "source": "base/root/%s members %d-%d" % (narc, first, first + 3),
                    "w": face.size[0], "h": face.size[1], "frames": [face], "offsets": None})

    x = MARGIN + GUTTER
    for col in columns:
        col["x"] = x
        x += col["w"] + GUTTER
    width = x + MARGIN
    height = HEADER + max(len(c["frames"]) * (c["h"] + GAP) for c in columns) + MARGIN

    font = ImageFont.load_default()
    canvas = Image.new("RGBA", (width, height), BG)
    draw = ImageDraw.Draw(canvas)
    draw.text((MARGIN, 4), "AVATAR TEMPLATE", fill=TEXT, font=font)
    draw.text((MARGIN, 16), "Draw in the transparent cells only.", fill=TEXT, font=font)
    draw.text((MARGIN, 28), "Red label = must, black = should / optional.", fill=TEXT, font=font)
    draw.text((MARGIN, 40), "15 colours + transparent per column.", fill=TEXT, font=font)

    layout = {"version": 2, "frame_order": "top to bottom, as the vanilla source", "columns": []}
    for col in columns:
        w, h, n = col["w"], col["h"], len(col["frames"])
        tag = "%s (%d)" % (col["label"], n)
        label = Image.new("RGBA", (HEADER - 58, 12), (0, 0, 0, 0))
        ImageDraw.Draw(label).text((0, 0), tag, fill=MUST if col["priority"] == "must" else TEXT, font=font)
        label = label.rotate(90, expand=True)
        canvas.alpha_composite(label, (col["x"] + (w - label.size[0]) // 2, 56))
        cells = []
        for i in range(n):
            y = HEADER + i * (h + GAP)
            draw.rectangle((col["x"] - 1, y - 1, col["x"] + w, y + h), outline=LINE)
            draw.text((col["x"] - GUTTER + 2, y + 1), str(i), fill=TEXT, font=font)
            cells.append([col["x"], y, w, h])
        col["cells"] = cells
        entry = {"key": col["key"], "label": col["label"], "priority": col["priority"], "kind": col["kind"],
                 "source": col["source"], "frame_size": [w, h], "frames": cells}
        if col["offsets"] is not None:
            entry["offsets"] = col["offsets"]  # where the game draws each frame (vanilla's shift)
        layout["columns"].append(entry)

    # The cells are cleared last: no border or number may reach into them.
    for col in columns:
        for x0, y0, w, h in col["cells"]:
            canvas.paste((0, 0, 0, 0), (x0, y0, x0 + w, y0 + h))
    guide = canvas.copy()
    for col in columns:
        for (x0, y0, _, _), frame in zip(col["cells"], col["frames"]):
            guide.alpha_composite(frame, (x0, y0))

    os.makedirs(OUT, exist_ok=True)
    canvas.save(os.path.join(OUT, "avatar_template.png"))
    guide.save(os.path.join(OUT, "avatar_template_guide.png"))
    layout["image_size"] = [width, height]
    with open(os.path.join(OUT, "avatar_layout.json"), "w") as f:
        json.dump(layout, f, indent=1)
    print("template %dx%d, %d columns -> %s" % (width, height, len(columns), OUT))


if __name__ == "__main__":
    main()
