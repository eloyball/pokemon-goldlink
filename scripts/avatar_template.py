#!/usr/bin/env python3
"""Builds the custom avatar drawing template (one PNG per avatar).

Writes to hg-engine/data/graphics/avatars/template/:
  avatar_template.png        the canvas to draw on: transparent cells, grey gutters and labels
  avatar_template_guide.png  the same, with Ethan's frames faintly in every cell (a tracing layer)
  avatar_layout.json         every frame's rectangle: the contract the cutter reads
  README.md                  the rules for drawing

Layout: one column per state. The overworld columns are 32 px wide, with 32x32 frames stacked
from the top in the vanilla sheet's order. The battle columns are 80 px wide, with 80x80 frames.
Everything outside the cells (the header, the gutters) is ignored by the cutter.

Usage: python3 scripts/avatar_template.py
"""
import json
import os

from PIL import Image, ImageDraw, ImageFont

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
GFX = os.path.join(ROOT, "hg-engine", "data", "graphics")
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
# (key, source, label, priority, kind): both battle pictures are cut into OAM pieces in the
# game and are reassembled here from their cell data.
BATTLE = [
    ("front", "trainer_gfx/000", "Battle front", "must", "front"),
    ("throw", "trainer_back_gfx/00", "Throw", "must", "back"),
    ("throw_link", "trainer_back_gfx/15", "Throw (link)", "optional", "back"),
]

OW, BW = 32, 80       # frame sizes
GUTTER = 16           # between columns: frame numbers go here
GAP = 2               # between frames of a column: the cell borders go here
HEADER = 162          # labels (written vertically)
MARGIN = 4
BG = (216, 216, 216, 255)
LINE = (150, 150, 150, 255)
TEXT = (40, 40, 40, 255)
MUST = (176, 32, 32, 255)
GUIDE_ALPHA = 80      # Ethan's frames in the guide, out of 255

OAM_SIZES = {(0, 0): (8, 8), (0, 1): (16, 16), (0, 2): (32, 32), (0, 3): (64, 64),
             (1, 0): (16, 8), (1, 1): (32, 8), (1, 2): (32, 16), (1, 3): (64, 32),
             (2, 0): (8, 16), (2, 1): (8, 32), (2, 2): (16, 32), (2, 3): (32, 64)}


def strip_frames(path, size):
    """Frames of a vertical strip, palette index 0 transparent, as RGBA."""
    raw = Image.open(os.path.join(GFX, path))
    frames = []
    for i in range(raw.size[1] // size):
        frames.append(to_rgba(raw.crop((0, i * size, size, (i + 1) * size))))
    return frames


def to_rgba(indexed):
    rgba = indexed.convert("RGBA")
    px, src = rgba.load(), indexed.load()
    for y in range(indexed.size[1]):
        for x in range(indexed.size[0]):
            if src[x, y] == 0:
                px[x, y] = (0, 0, 0, 0)
    return rgba


def back_frames(base):
    """A battle picture's frames as shown in battle (front or back): each cell takes its own
    100-tile block (VRAM transfer) and places its OAM pieces around the centre."""
    raw = Image.open(os.path.join(GFX, base + ".png"))
    cells = json.load(open(os.path.join(GFX, base + "_cell.json")))["cells"]
    per_row = raw.size[0] // 8
    tiles = [raw.crop(((t % per_row) * 8, (t // per_row) * 8, (t % per_row) * 8 + 8, (t // per_row) * 8 + 8))
             for t in range((raw.size[0] // 8) * (raw.size[1] // 8))]
    frames = []
    for index, cell in enumerate(cells):
        frame = Image.new("P", (BW, BW), 0)
        frame.putpalette(raw.getpalette())
        for oam in cell["OAM"]:
            w, h = OAM_SIZES[(oam["Attr0"]["Shape"], oam["Attr1"]["Size"])]
            first = index * (BW * BW // 64) + oam["Attr2"]["CharName"] * 2
            x0, y0 = oam["Attr1"]["XCoordinate"] + BW // 2, oam["Attr0"]["YCoordinate"] + BW // 2
            k = 0
            for ty in range(h // 8):
                for tx in range(w // 8):
                    if first + k < len(tiles):
                        frame.paste(tiles[first + k], (x0 + tx * 8, y0 + ty * 8))
                    k += 1
        frames.append(to_rgba(frame))
    return frames


def main():
    columns = []
    for key, src, label, prio in OVERWORLD:
        columns.append({"key": key, "label": label, "priority": prio, "kind": "overworld",
                        "source": "data/graphics/" + src, "size": OW, "frames": strip_frames(src, OW)})
    for key, src, label, prio, kind in BATTLE:
        frames = back_frames(src)
        columns.append({"key": key, "label": label, "priority": prio, "kind": "battle_" + kind,
                        "source": "data/graphics/" + src + ".png",
                        "size": BW, "frames": frames})

    x = MARGIN + GUTTER
    for col in columns:
        col["x"] = x
        x += col["size"] + GUTTER
    width = x + MARGIN - GUTTER + GUTTER
    height = HEADER + max(len(c["frames"]) * (c["size"] + GAP) for c in columns) + MARGIN

    font = ImageFont.load_default()
    canvas = Image.new("RGBA", (width, height), BG)
    draw = ImageDraw.Draw(canvas)
    draw.text((MARGIN, 4), "AVATAR TEMPLATE", fill=TEXT, font=font)
    draw.text((MARGIN, 16), "Draw in the transparent cells only.", fill=TEXT, font=font)
    draw.text((MARGIN, 28), "Red label = must, black = should / optional.", fill=TEXT, font=font)
    draw.text((MARGIN, 40), "15 colours + transparent for the sheet.", fill=TEXT, font=font)

    layout = {"version": 1, "frame_order": "top to bottom, as the vanilla source", "columns": []}
    for col in columns:
        s, n = col["size"], len(col["frames"])
        # The label, written upwards in the header above the column.
        tag = "%s (%d)" % (col["label"], n)
        label = Image.new("RGBA", (HEADER - 58, 12), (0, 0, 0, 0))
        ImageDraw.Draw(label).text((0, 0), tag, fill=MUST if col["priority"] == "must" else TEXT, font=font)
        label = label.rotate(90, expand=True)
        canvas.alpha_composite(label, (col["x"] + (s - label.size[0]) // 2, 56))
        cells = []
        for i in range(n):
            y = HEADER + i * (s + GAP)
            draw.rectangle((col["x"] - 1, y - 1, col["x"] + s, y + s), outline=LINE)
            draw.text((col["x"] - GUTTER + 2, y + 1), str(i), fill=TEXT, font=font)
            cells.append([col["x"], y, s, s])
        col["cells"] = cells
        layout["columns"].append({"key": col["key"], "label": col["label"], "priority": col["priority"],
                                  "kind": col["kind"], "source": col["source"], "frame_size": s,
                                  "frames": cells})

    # The cells are cleared last: no border or number may reach into them.
    for col in columns:
        for x0, y0, w, h in col["cells"]:
            canvas.paste((0, 0, 0, 0), (x0, y0, x0 + w, y0 + h))
    guide = canvas.copy()
    for col in columns:
        for (x0, y0, _, _), frame in zip(col["cells"], col["frames"]):
            faint = frame.copy()
            alpha = faint.getchannel("A").point(lambda a: GUIDE_ALPHA if a else 0)
            faint.putalpha(alpha)
            guide.alpha_composite(faint, (x0, y0))

    os.makedirs(OUT, exist_ok=True)
    canvas.save(os.path.join(OUT, "avatar_template.png"))
    guide.save(os.path.join(OUT, "avatar_template_guide.png"))
    layout["image_size"] = [width, height]
    with open(os.path.join(OUT, "avatar_layout.json"), "w") as f:
        json.dump(layout, f, indent=1)
    print("template %dx%d, %d columns -> %s" % (width, height, len(columns), OUT))


if __name__ == "__main__":
    main()
