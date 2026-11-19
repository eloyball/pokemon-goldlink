#!/usr/bin/env python3
"""GoldLink (v0.8, UC16's option C): NitroSystem texture sets (BTX0 / TEX0) read and written, and
the dictionaries' name trees built, for scripts/coop_area_neighbours.py.

A dictionary (NNSG3dResDict, ccd lib/include/nnsys/g3d/binres/res_struct.h): revision, count,
size; the tree (a header of 8 bytes, then one node of 4 bytes per entry and the root: refBit,
idxLeft, idxRight, idxEntry); the entries (unit size, names' offset, the data); the 16-byte names.
The game finds a name by walking the tree (NNS_G3dGetResDictIdxByName): from the root, while the
refBit falls, the name's bit refBit (its 4 little-endian words) picks the right or left link;
the entry reached is compared with the name. `build_tree` makes Nintendo's tree from the names in
order (checked against every set in the game: `--check`).
"""
import struct
import sys


def u16(b, o):
    return struct.unpack_from("<H", b, o)[0]


def u32(b, o):
    return struct.unpack_from("<I", b, o)[0]


def name16(name):
    raw = name.encode("latin-1")[:16]
    return raw + b"\0" * (16 - len(raw))


def _bit(key, n):
    """Bit n (0..127) of a 16-byte name read as 4 little-endian words."""
    return (key[n >> 3] >> (n & 7)) & 1


def build_tree(names):
    """The dictionary tree's nodes for these names: [(refBit, left, right, entry)], node 0 the root
    (refBit 127). Built as a Patricia tree by inserting the names in order, the root standing for
    an all-zero name; the nodes then numbered in preorder (as Nintendo's tools did: every set in
    the game, --check)."""
    keys = [name16(n) for n in names]
    zero = bytes(16)
    nodes = [[127, 0, 0, 0]]

    def key_of(x):
        return zero if x == 0 else keys[nodes[x][3]]

    for i, k in enumerate(keys):
        p, x = 0, nodes[0][1]
        while nodes[p][0] > nodes[x][0]:
            p = x
            x = nodes[x][2] if _bit(k, nodes[x][0]) else nodes[x][1]
        other = key_of(x)
        d = max(n for n in range(128) if _bit(k, n) != _bit(other, n))
        p, x = 0, nodes[0][1]
        while nodes[p][0] > nodes[x][0] and nodes[x][0] > d:
            p = x
            x = nodes[x][2] if _bit(k, nodes[x][0]) else nodes[x][1]
        t = len(nodes)
        nodes.append([d, t, x, i] if not _bit(k, d) else [d, x, t, i])
        if p == 0:
            nodes[0][1] = t
        elif _bit(k, nodes[p][0]):
            nodes[p][2] = t
        else:
            nodes[p][1] = t
    # Preorder: a node, its left subtree, its right one (a link down: to a lower refBit).
    order = [0]

    def visit(n):
        order.append(n)
        for c in (nodes[n][1], nodes[n][2]):
            if c != 0 and nodes[c][0] < nodes[n][0]:
                visit(c)

    if len(nodes) > 1:
        visit(nodes[0][1])
    new = {old: i for i, old in enumerate(order)}
    return [(nodes[o][0], new[nodes[o][1]], new[nodes[o][2]], nodes[o][3]) for o in order]


def read_dict(b, o):
    """A dictionary at o: (names, entries (bytes each), tree nodes)."""
    count = b[o + 1]
    e = o + u16(b, o + 6)
    unit = u16(b, e)
    names_at = e + u16(b, e + 2)
    entries = [bytes(b[e + 4 + i * unit:e + 4 + (i + 1) * unit]) for i in range(count)]
    names = [bytes(b[names_at + 16 * i:names_at + 16 * (i + 1)]).split(b"\0")[0].decode("latin-1") for i in range(count)]
    nodes = [tuple(b[o + 8 + 4 * n:o + 12 + 4 * n]) for n in range(count + 1)]
    return names, entries, nodes


def write_dict(names, entries):
    """A dictionary's bytes (its tree built): revision, count, size, 8, the entries' offset, the
    root and one node per name, the entries (unit size, the names' offset, the data), the names."""
    count = len(names)
    unit = len(entries[0]) if entries else 4
    nodes = build_tree(names)
    ofs_entry = 8 + 4 * len(nodes)
    total = ofs_entry + 4 + unit * count + 16 * count
    head = struct.pack("<BBHHH", 0, count, total, 8, ofs_entry)
    tree = b"".join(struct.pack("<BBBB", *n) for n in nodes)
    body = struct.pack("<HH", unit, 4 + unit * count) + b"".join(entries)
    return head + tree + body + b"".join(name16(n) for n in names)


def narc(path):
    """A NARC's members (bytes)."""
    b = open(path, "rb").read()
    assert b[:4] == b"NARC"
    o = u16(b, 0x0C)
    assert b[o:o + 4] == b"BTAF"
    n = u16(b, o + 8)
    spans = [(u32(b, o + 12 + 8 * i), u32(b, o + 16 + 8 * i)) for i in range(n)]
    o += u32(b, o + 4)
    o += u32(b, o + 4)  # BTNF
    base = o + 8  # GMIF
    return [b[base + s:base + e] for s, e in spans]


class Texture:
    def __init__(self, name, param, extra, data, index=b""):
        self.name, self.param, self.extra, self.data, self.index = name, param, extra, data, index

    @property
    def fmt(self):
        return (self.param >> 26) & 7

    def size(self):
        w = 8 << ((self.param >> 20) & 7)
        h = 8 << ((self.param >> 23) & 7)
        bpp = {1: 8, 2: 2, 3: 4, 4: 8, 5: 2, 6: 8, 7: 16}.get(self.fmt, 8)
        return w * h * bpp // 8


class Palette:
    def __init__(self, name, flag, data):
        self.name, self.flag, self.data = name, flag, data


def read_tex0(b):
    """A BTX0's (or BMD0's) TEX0: (textures, palettes), or ([], []) without one."""
    nblocks = u16(b, 0x0E)
    t = None
    for i in range(nblocks):
        o = u32(b, 0x10 + 4 * i)
        if b[o:o + 4] == b"TEX0":
            t = o
    if t is None:
        return [], []
    tex_data = t + u32(b, t + 0x14)
    c4_data = t + u32(b, t + 0x24)
    c4_index = t + u32(b, t + 0x28)
    plt_data = t + u32(b, t + 0x38)
    plt_size = u16(b, t + 0x30) << 3
    names, entries, _ = read_dict(b, t + u16(b, t + 0x0E))
    textures = []
    for name, e in zip(names, entries):
        param, extra = u32(e, 0), u32(e, 4)
        tex = Texture(name, param, extra, b"")
        off = (param & 0xFFFF) << 3
        n = tex.size()
        if tex.fmt == 5:
            tex.data = bytes(b[c4_data + off:c4_data + off + n])
            tex.index = bytes(b[c4_index + off // 2:c4_index + off // 2 + n // 2])
        else:
            tex.data = bytes(b[tex_data + off:tex_data + off + n])
        textures.append(tex)
    pnames, pentries, _ = read_dict(b, t + u16(b, t + 0x34))
    offs = sorted({u16(e, 0) << 3 for e in pentries})
    palettes = []
    for name, e in zip(pnames, pentries):
        start = u16(e, 0) << 3
        later = [x for x in offs if x > start]
        end = later[0] if later else plt_size
        palettes.append(Palette(name, u16(e, 2), bytes(b[plt_data + start:plt_data + end])))
    return textures, palettes


def _align(b, n=8):
    return b + b"\0" * (-len(b) % n)


def write_btx0(textures, palettes):
    """A BTX0 with one TEX0 of these textures and palettes (data in their order, 8-aligned)."""
    tex_data, c4_data, c4_index, plt_data = b"", b"", b"", b""
    tex_entries, plt_entries = [], []
    for t in textures:
        if t.fmt == 5:
            off = len(c4_data)
            c4_data = _align(c4_data + t.data)
            c4_index = c4_index + t.index + b"\0" * (len(c4_data) // 2 - len(c4_index) - len(t.index))
        else:
            off = len(tex_data)
            tex_data = _align(tex_data + t.data)
        param = (t.param & ~0xFFFF) | (off >> 3)
        tex_entries.append(struct.pack("<II", param, t.extra))
    for p in palettes:
        off = len(plt_data)
        plt_data = _align(plt_data + p.data)
        plt_entries.append(struct.pack("<HH", off >> 3, p.flag))
    tex_dict = write_dict([t.name for t in textures], tex_entries)
    plt_dict = write_dict([p.name for p in palettes], plt_entries)
    header_size = 0x3C
    o_texdict = header_size
    o_pltdict = o_texdict + len(_align(tex_dict, 4))
    o_tex = _align(b"\0" * (o_pltdict + len(plt_dict)), 8).__len__()
    o_c4 = o_tex + len(tex_data)
    o_c4i = o_c4 + len(c4_data)
    o_plt = o_c4i + len(_align(c4_index))
    block_size = o_plt + len(plt_data)
    head = b"TEX0" + struct.pack("<I", block_size)
    head += struct.pack("<I", 0)
    head += struct.pack("<HH", len(tex_data) >> 3, o_texdict)
    head += struct.pack("<I", 0)
    head += struct.pack("<I", o_tex)
    head += struct.pack("<I", 0)
    head += struct.pack("<HH", len(c4_data) >> 3, o_texdict)
    head += struct.pack("<I", 0)
    head += struct.pack("<I", o_c4)
    head += struct.pack("<I", o_c4i)
    head += struct.pack("<I", 0)
    head += struct.pack("<HH", len(plt_data) >> 3, 0)
    head += struct.pack("<HH", o_pltdict, 0)
    head += struct.pack("<I", o_plt)
    assert len(head) == header_size
    block = head + _align(tex_dict, 4) + plt_dict
    block = _align(block, 8)
    assert len(block) == o_tex, (len(block), o_tex)
    block += tex_data + c4_data + _align(c4_index) + plt_data
    btx = b"BTX0" + struct.pack("<HHIHHI", 0xFEFF, 1, 0x14 + len(block), 0x10, 1, 0x14) + block
    return btx


def merge_btx0(b, textures, palettes):
    """The BTX0 b with these textures and palettes added: its data kept as it is (textures sharing
    texels, palettes overlapping: their offsets don't move), the new ones appended after each
    block, both dictionaries rebuilt (old entries first, unchanged). Names already there are
    skipped (so merging twice changes nothing)."""
    nblocks = u16(b, 0x0E)
    t = next(u32(b, 0x10 + 4 * i) for i in range(nblocks) if b[u32(b, 0x10 + 4 * i):u32(b, 0x10 + 4 * i) + 4] == b"TEX0")
    tex_size = u16(b, t + 0x0C) << 3
    c4_size = u16(b, t + 0x1C) << 3
    plt_size = u16(b, t + 0x30) << 3
    tex_data = bytes(b[t + u32(b, t + 0x14):t + u32(b, t + 0x14) + tex_size])
    c4_data = bytes(b[t + u32(b, t + 0x24):t + u32(b, t + 0x24) + c4_size])
    c4_index = bytes(b[t + u32(b, t + 0x28):t + u32(b, t + 0x28) + c4_size // 2])
    plt_data = bytes(b[t + u32(b, t + 0x38):t + u32(b, t + 0x38) + plt_size])
    tnames, tentries, _ = read_dict(b, t + u16(b, t + 0x0E))
    pnames, pentries, _ = read_dict(b, t + u16(b, t + 0x34))
    tnames, tentries, pnames, pentries = list(tnames), list(tentries), list(pnames), list(pentries)
    for tex in textures:
        if tex.name in tnames:
            continue
        if tex.fmt == 5:
            off = len(c4_data)
            c4_data = _align(c4_data + tex.data)
            c4_index = c4_index + b"\0" * (off // 2 - len(c4_index)) + tex.index
            c4_index = c4_index + b"\0" * (len(c4_data) // 2 - len(c4_index))
        else:
            off = len(tex_data)
            tex_data = _align(tex_data + tex.data)
        tnames.append(tex.name)
        tentries.append(struct.pack("<II", (tex.param & ~0xFFFF) | (off >> 3), tex.extra))
    for plt in palettes:
        if plt.name in pnames:
            continue
        off = len(plt_data)
        plt_data = _align(plt_data + plt.data)
        pnames.append(plt.name)
        pentries.append(struct.pack("<HH", off >> 3, plt.flag))
    if len(tnames) > 255 or len(pnames) > 255:
        raise ValueError("a dictionary holds at most 255 names")
    tex_dict = write_dict(tnames, tentries)
    plt_dict = write_dict(pnames, pentries)
    header_size = 0x3C
    o_texdict = header_size
    o_pltdict = o_texdict + len(_align(tex_dict, 4))
    o_tex = len(_align(b"\0" * (o_pltdict + len(plt_dict)), 8))
    o_c4 = o_tex + len(tex_data)
    o_c4i = o_c4 + len(c4_data)
    o_plt = o_c4i + len(_align(c4_index))
    block_size = o_plt + len(plt_data)
    # NNSG3dResTex: the texture, 4x4 and palette infos (vramKey, size, dictionary, flag, data);
    # their flags kept from the original (the palettes' 0x8000: its 4-colour palettes go in the
    # first 64 KB of palette memory; lost, Route 8's area never finished loading).
    tex_flag, c4_flag, plt_flag = u16(b, t + 0x10), u16(b, t + 0x20), u16(b, t + 0x32)
    head = b"TEX0" + struct.pack("<I", block_size) + struct.pack("<I", 0)
    head += struct.pack("<HH", len(tex_data) >> 3, o_texdict) + struct.pack("<HH", tex_flag, 0)
    head += struct.pack("<I", o_tex) + struct.pack("<I", 0)
    head += struct.pack("<HH", len(c4_data) >> 3, o_texdict) + struct.pack("<HH", c4_flag, 0)
    head += struct.pack("<I", o_c4) + struct.pack("<I", o_c4i) + struct.pack("<I", 0)
    head += struct.pack("<HH", len(plt_data) >> 3, plt_flag) + struct.pack("<HH", o_pltdict, 0)
    head += struct.pack("<I", o_plt)
    block = _align(head + _align(tex_dict, 4) + plt_dict, 8)
    block += tex_data + c4_data + _align(c4_index) + plt_data
    return b"BTX0" + struct.pack("<HHIHHI", 0xFEFF, 1, 0x14 + len(block), 0x10, 1, 0x14) + block


PALETTE_BYTES = {1: 64, 2: 8, 3: 32, 4: 512, 6: 16}  # by the texture format using it (4x4: as found)


def _check(paths):
    """Every dictionary of these BTX0 / BMD0 files (or NARCs of them) rebuilt from its names: the
    same tree."""
    total = bad = 0
    for path in paths:
        members = narc(path)
        for k, b in enumerate(members):
            if b[:4] not in (b"BTX0", b"BMD0"):
                continue
            nblocks = u16(b, 0x0E)
            for i in range(nblocks):
                o = u32(b, 0x10 + 4 * i)
                if b[o:o + 4] != b"TEX0":
                    continue
                for d in (o + u16(b, o + 0x0E), o + u16(b, o + 0x34)):
                    names, _, nodes = read_dict(b, d)
                    if not names:
                        continue
                    total += 1
                    if build_tree(names) != nodes:
                        bad += 1
                        if bad <= 3:
                            print(f"{path} #{k}: {names[:4]}... tree differs")
    print(f"{total} dictionaries, {bad} differ")
    return bad == 0


if __name__ == "__main__":
    if sys.argv[1:2] == ["--check"]:
        sys.exit(0 if _check(sys.argv[2:]) else 1)
