#!/usr/bin/env python3
"""Reads a custom avatar sheet and normalizes it, column by column, for the game.

A sheet is one PNG per avatar drawn on the template (hg-engine/data/graphics/avatars/template/,
see its README). For every column of avatar_layout.json this:
  1. cuts out the frames (anything outside the cells is ignored; drawing that spills into a
     gap or gutter is reported, since it is cut off);
  2. makes every pixel fully opaque or fully transparent (alpha threshold 50%);
  3. skips a column still identical to Ethan's (the template guide): it falls back to vanilla;
  4. snaps colours to the DS's 15-bit palette (5 bits per channel), which merges colours that
     look the same on the DS anyway;
  5. reduces the column to at most 15 colours (+ transparency): each step merges the pair of
     colours whose merge changes the picture least (the rarer colour's pixel count times the
     colour distance), so stray near-duplicates go first and the main colours stay.
Each overworld state and each battle picture is its own file with its own palette in the game,
so the limit is per column.

Usage:
  python3 scripts/avatar_sheet.py <sheet.png>                 report what the cutter would do
  python3 scripts/avatar_sheet.py <sheet.png> --write out.png  also save the normalized sheet
scripts/avatar_build.py imports normalize_sheet() from here.
"""
import argparse
import json
import os

from PIL import Image

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
TEMPLATE = os.path.join(ROOT, "hg-engine", "data", "graphics", "avatars", "template")
MAX_COLOURS = 15     # palette index 0 is transparency
ALPHA_CUT = 128


def ds_colour(rgb):
    """The colour the DS shows (5 bits per channel), back in 8-bit form."""
    return tuple((c >> 3) * 255 // 31 for c in rgb)


def distance(a, b):
    """'Redmean' colour distance: cheap and closer to perception than plain RGB."""
    r = (a[0] + b[0]) / 2
    dr, dg, db = a[0] - b[0], a[1] - b[1], a[2] - b[2]
    return ((2 + r / 256) * dr * dr + 4 * dg * dg + (2 + (255 - r) / 256) * db * db) ** 0.5


def reduce_colours(counts, limit=MAX_COLOURS):
    """Merges colours until at most `limit` remain. `counts`: {rgb: pixels}. Returns
    ({old rgb: new rgb} for every colour, [(merged rgb, into rgb, pixels)] in merge order)."""
    mapping = {c: c for c in counts}
    live = dict(counts)
    merges = []
    while len(live) > limit:
        best = None
        colours = list(live)
        for i, a in enumerate(colours):
            for b in colours[i + 1:]:
                small, big = (a, b) if live[a] <= live[b] else (b, a)
                cost = live[small] * distance(a, b)
                if best is None or cost < best[0]:
                    best = (cost, small, big)
        _, small, big = best
        merges.append((small, big, live[small]))
        live[big] += live.pop(small)
        for old, new in mapping.items():
            if new == small:
                mapping[old] = big
    return mapping, merges


def load_layout():
    with open(os.path.join(TEMPLATE, "avatar_layout.json")) as f:
        return json.load(f)


def opaque_or_clear(frame):
    """Every pixel fully opaque or fully transparent (transparent ones as (0, 0, 0, 0))."""
    px = frame.load()
    for yy in range(frame.size[1]):
        for xx in range(frame.size[0]):
            p = px[xx, yy]
            px[xx, yy] = (p[0], p[1], p[2], 255) if p[3] >= ALPHA_CUT else (0, 0, 0, 0)
    return frame


def normalize_sheet(path, fallback=True):
    """Returns (columns, notes): columns[key] = {"frames": [RGBA images], "colours": n,
    "vanilla": bool, "changed": pixels changed by the colour steps}; notes are human-readable.
    fallback=False keeps a column identical to Ethan's (the cutter's round-trip check)."""
    layout = load_layout()
    sheet = Image.open(path).convert("RGBA")
    guide = Image.open(os.path.join(TEMPLATE, "avatar_template_guide.png")).convert("RGBA")
    template = Image.open(os.path.join(TEMPLATE, "avatar_template.png")).convert("RGBA")
    if list(sheet.size) != layout["image_size"]:
        # A sheet drawn on an older template: columns only ever get added on the right (the map
        # head, Milestone 6), so lay it over the blank template; the new columns are empty.
        if sheet.size[0] > layout["image_size"][0] or sheet.size[1] > layout["image_size"][1]:
            raise SystemExit("%s is %dx%d; the template layout is %dx%d" % (path, *sheet.size, *layout["image_size"]))
        # Up to the right edge of its last whole column: its margin would reach into the new ones.
        edge = max(x + w for col in layout["columns"] for x, _, w, _ in col["frames"] if x + w <= sheet.size[0])
        padded = template.copy()
        padded.paste(sheet.crop((0, 0, edge, sheet.size[1])), (0, 0))
        sheet = padded
    notes = []

    # Drawing outside the cells: compare with the blank template, column by column.
    inside = Image.new("L", sheet.size, 0)
    for col in layout["columns"]:
        for x, y, w, h in col["frames"]:
            inside.paste(255, (x, y, x + w, y + h))
    spill = {}
    sp, tp, ip = sheet.load(), template.load(), inside.load()
    starts = sorted((c["frames"][0][0], c["key"]) for c in layout["columns"])
    for yy in range(layout["columns"][0]["frames"][0][1] - 2, sheet.size[1]):
        for xx in range(sheet.size[0]):
            if not ip[xx, yy] and sp[xx, yy] != tp[xx, yy]:
                near = [k for x0, k in starts if x0 - 16 <= xx]
                key = near[-1] if near else "the left margin"
                spill[key] = spill.get(key, 0) + 1
    for key, n in spill.items():
        notes.append("%s: %d px drawn outside its cells (cut off)" % (key, n))

    columns = {}
    for col in layout["columns"]:
        frames, same = [], True
        for x, y, w, h in col["frames"]:
            frame = opaque_or_clear(sheet.crop((x, y, x + w, y + h)))
            ethan = opaque_or_clear(guide.crop((x, y, x + w, y + h)))
            if frame.getchannel("A").getbbox() is not None and frame.tobytes() != ethan.tobytes():
                same = False
            frames.append(frame)
        drawn = any(f.getchannel("A").getbbox() for f in frames)
        if not drawn or (same and fallback):
            columns[col["key"]] = {"frames": frames, "colours": 0, "vanilla": True, "changed": 0}
            notes.append("%s: %s, uses the vanilla sprite" % (col["key"], "empty" if not drawn else "still Ethan's"))
            continue
        counts = {}
        for f in frames:
            for p in f.get_flattened_data():
                if p[3]:
                    counts[p[:3]] = counts.get(p[:3], 0) + 1
        snapped = {}
        for rgb, n in counts.items():
            snapped[ds_colour(rgb)] = snapped.get(ds_colour(rgb), 0) + n
        merged, merges = reduce_colours(snapped)
        changed = 0
        for f in frames:
            px = f.load()
            for yy in range(f.size[1]):
                for xx in range(f.size[0]):
                    p = px[xx, yy]
                    if p[3]:
                        new = merged[ds_colour(p[:3])]
                        if new != ds_colour(p[:3]):
                            changed += 1
                        px[xx, yy] = new + (255,)
        final = len(set(merged.values()))
        columns[col["key"]] = {"frames": frames, "colours": final, "vanilla": False, "changed": changed}
        if len(snapped) > MAX_COLOURS:
            notes.append("%s: %d colours (%d on the DS) reduced to %d, %d px changed: %s"
                         % (col["key"], len(counts), len(snapped), final, changed,
                            ", ".join("%s -> %s (%d px)" % (a, b, n) for a, b, n in merges)))
    return columns, notes


def main():
    parser = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    parser.add_argument("sheet")
    parser.add_argument("--write", help="save the normalized sheet here")
    args = parser.parse_args()
    columns, notes = normalize_sheet(args.sheet)
    layout = load_layout()
    for col in layout["columns"]:
        c = columns[col["key"]]
        print("%-11s %s" % (col["key"], "vanilla" if c["vanilla"] else "%2d colours" % c["colours"]))
    print()
    print("\n".join(notes) if notes else "nothing to fix")
    if args.write:
        out = Image.open(os.path.join(TEMPLATE, "avatar_template.png")).convert("RGBA")
        for col in layout["columns"]:
            for (x, y, w, h), frame in zip(col["frames"], columns[col["key"]]["frames"]):
                out.paste(frame, (x, y))
        out.save(args.write)
        print("normalized sheet -> %s" % args.write)


if __name__ == "__main__":
    main()
