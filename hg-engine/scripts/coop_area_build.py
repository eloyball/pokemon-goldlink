#!/usr/bin/env python3
"""GoldLink (v0.8, UC16's option C): each area's map textures and props gain those its
neighbouring areas' border blocks use, so the wider picture shows a neighbour's block as it is
(the game loads one area's graphics at a time; Overlay 153's stand-ins, src/coop_ov1/
coop_neighbour.c, stay for anything still missing).

The list (data/coop/area_neighbours.json, from the dev tool) names, per map texture set (a/0/4/4
member) the textures and palettes to copy from another member, and per prop list (a/0/4/3 member,
whose prop textures are a/0/7/0's same member) the props to load and their textures. This copies
them from the game's own files (nothing of the game's is in the repository), appending: a member's
own data keeps its offsets, names and ids already there are skipped, so running it again on its
output changes nothing (the build's file system keeps the last build's archives).

    python3 scripts/coop_area_build.py <plan.json> <file system root> <out dir>
"""
import json
import os
import struct
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import coop_nitro_tex as nt  # noqa: E402

MAX_PROPS = 549  # pret AREA_DATA_MAX_MAP_PROPS (550): the count stays below it
TEX_BUDGET = 96 * 1024  # a map or prop texture set after the additions (an area loads one of each
# into the 256 KB of texture memory, with the people's sprites); a set over it gains nothing


def narc_parts(path):
    """A NARC's members, and its file name block (kept as it is)."""
    b = open(path, "rb").read()
    o = nt.u16(b, 0x0C)
    n = nt.u16(b, o + 8)
    spans = [(nt.u32(b, o + 12 + 8 * i), nt.u32(b, o + 16 + 8 * i)) for i in range(n)]
    o += nt.u32(b, o + 4)
    btnf = b[o:o + nt.u32(b, o + 4)]
    o += nt.u32(b, o + 4)
    base = o + 8
    return [bytes(b[base + s:base + e]) for s, e in spans], bytes(btnf)


def narc_write(members, btnf):
    data, spans = b"", []
    for m in members:
        spans.append((len(data), len(data) + len(m)))
        data += m + b"\xff" * (-len(m) % 4)
    btaf = b"BTAF" + struct.pack("<IHH", 12 + 8 * len(members), len(members), 0)
    btaf += b"".join(struct.pack("<II", s, e) for s, e in spans)
    gmif = b"GMIF" + struct.pack("<I", 8 + len(data)) + data
    body = btaf + btnf + gmif
    return b"NARC" + struct.pack("<HHIHH", 0xFFFE, 0x0100, 16 + len(body), 16, 3) + body


def palette_block(b):
    """A BTX0's palette data and its dictionary: (data, {name: (offset, flag)})."""
    t = next(nt.u32(b, 0x10 + 4 * i) for i in range(nt.u16(b, 0x0E))
             if b[nt.u32(b, 0x10 + 4 * i):nt.u32(b, 0x10 + 4 * i) + 4] == b"TEX0")
    size = nt.u16(b, t + 0x30) << 3
    data = b[t + nt.u32(b, t + 0x38):t + nt.u32(b, t + 0x38) + size]
    names, entries, _ = nt.read_dict(b, t + nt.u16(b, t + 0x34))
    return data, {n: (nt.u16(e, 0) << 3, nt.u16(e, 2)) for n, e in zip(names, entries)}


def additions(members, src_member, wanted):
    """From member src_member, the textures and palettes named in wanted ([texture, palette]):
    each palette as long as its texture's format reads (a 4x4 texture's: up to the next palette)."""
    b = members[src_member]
    textures = {t.name: t for t in nt.read_tex0(b)[0]}
    pdata, pdict = palette_block(b)
    offsets = sorted({o for o, _ in pdict.values()})
    tex_out, pal_out = [], []
    for tname, pname in wanted:
        tex = textures.get(tname)
        if tex is None:
            continue
        tex_out.append(tex)
        if pname and pname in pdict:
            start, flag = pdict[pname]
            n = nt.PALETTE_BYTES.get(tex.fmt)
            if n is None:
                later = [o for o in offsets if o > start]
                n = (later[0] if later else len(pdata)) - start
            pal_out.append(nt.Palette(pname, flag, bytes(pdata[start:min(start + n, len(pdata))])))
    return tex_out, pal_out


def tex_bytes(b):
    textures, palettes = nt.read_tex0(b)
    return sum(t.size() for t in textures) + sum(len(p.data) for p in palettes)


def main():
    plan_path, root, out = sys.argv[1:4]
    plan = json.load(open(plan_path))
    os.makedirs(out, exist_ok=True)
    maptex, maptex_btnf = narc_parts(os.path.join(root, "a/0/4/4"))
    proptex, proptex_btnf = narc_parts(os.path.join(root, "a/0/7/0"))
    lists, lists_btnf = narc_parts(os.path.join(root, "a/0/4/3"))
    added = skipped = 0
    for member, wanted in plan["map"].items():
        m = int(member)
        by_src = {}
        for tname, pname, src in wanted:
            by_src.setdefault(src, []).append((tname, pname))
        b = maptex[m]
        for src, w in sorted(by_src.items()):
            texs, pals = additions(maptex, src, w)
            b = nt.merge_btx0(b, texs, pals)
        if tex_bytes(b) > TEX_BUDGET:
            skipped += 1
            print(f"map textures {m}: over the budget, unchanged")
            continue
        added += len(nt.read_tex0(b)[0]) - len(nt.read_tex0(maptex[m])[0])
        maptex[m] = b
    props_added = 0
    for member, entry in plan["props"].items():
        m = int(member)
        lst = lists[m]
        count = nt.u16(lst, 0)
        ids = [nt.u16(lst, 2 * (i + 1)) for i in range(count)]
        new = [p for p in entry["ids"] if p not in ids]
        if count + len(new) > MAX_PROPS:
            print(f"props {m}: {count} + {len(new)} over the limit, unchanged")
            continue
        by_src = {}
        for tname, pname, src in entry["tex"]:
            by_src.setdefault(src, []).append((tname, pname))
        b = proptex[m]
        for src, w in sorted(by_src.items()):
            texs, pals = additions(proptex, src, w)
            b = nt.merge_btx0(b, texs, pals)
        if tex_bytes(b) > TEX_BUDGET:
            print(f"prop textures {m}: over the budget, unchanged")
            continue
        proptex[m] = b
        ids += new
        lists[m] = struct.pack("<H", len(ids)) + b"".join(struct.pack("<H", p) for p in ids) + lst[2 + 2 * count:]
        props_added += len(new)
    open(os.path.join(out, "a044"), "wb").write(narc_write(maptex, maptex_btnf))
    open(os.path.join(out, "a070"), "wb").write(narc_write(proptex, proptex_btnf))
    open(os.path.join(out, "a043"), "wb").write(narc_write(lists, lists_btnf))
    print(f"area neighbours: {added} map textures, {props_added} props added; {skipped} sets over the budget")


if __name__ == "__main__":
    main()
