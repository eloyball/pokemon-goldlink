#!/usr/bin/env python3
"""Draws the launcher's pixel art (dev/docs/unified-client/UC8.md §2.4): golden hour over a Johto town
(a sunset sky, a pagoda, a snowy peak, roofs, autumn trees, falling leaves), the menu icons and the app icon. Everything is drawn
here from shapes and a fixed seed, so the files are reproducible:

    python3 scripts/launcher_art.py      (writes crates/goldlink/assets/*.png)

Needs Pillow.
"""
import math
import random
from pathlib import Path

from PIL import Image

OUT = Path(__file__).resolve().parent.parent / "crates" / "goldlink" / "assets"
W, H = 480, 270  # drawn at 1x; the launcher scales it by whole pixels

BAYER = [[0, 8, 2, 10], [12, 4, 14, 6], [3, 11, 1, 9], [15, 7, 13, 5]]


def hexc(s):
    s = s.lstrip("#")
    return tuple(int(s[i : i + 2], 16) for i in (0, 2, 4))


def lerp(a, b, t):
    return tuple(round(a[i] + (b[i] - a[i]) * t) for i in range(3))


def dithered_band(img, y0, y1, top, bottom, steps):
    """A vertical gradient quantized to `steps` colours with ordered dithering."""
    px = img.load()
    palette = [lerp(top, bottom, i / (steps - 1)) for i in range(steps)]
    for y in range(y0, y1):
        t = (y - y0) / max(1, y1 - y0 - 1) * (steps - 1)
        base, frac = int(t), t - int(t)
        for x in range(W):
            hi = frac * 16 > BAYER[y % 4][x % 4]
            px[x, y] = palette[min(steps - 1, base + (1 if hi else 0))]


def ridge(rng, y_base, rough, n=W, start=None):
    """A ridge line by midpoint displacement: heights for x in 0..n."""
    size = 1
    while size < n:
        size *= 2
    h = [0.0] * (size + 1)
    h[0] = start if start is not None else rng.uniform(-1, 1) * rough
    h[size] = rng.uniform(-1, 1) * rough
    step, scale = size, rough
    while step > 1:
        half = step // 2
        for i in range(half, size, step):
            h[i] = (h[i - half] + h[i + half]) / 2 + rng.uniform(-1, 1) * scale
        step, scale = half, scale * 0.55
    return [y_base + h[x] for x in range(n)]


def fill_below(img, line, colour, until=H):
    px = img.load()
    for x, top in enumerate(line):
        for y in range(max(0, int(top)), until):
            px[x, y] = colour


def background():
    """Golden hour over a Johto town (the user's reference: HeartGold's own key art, its mood only):
    the sun on the horizon, a tall pagoda on the left, a snowy peak on the right, a town of
    tiled roofs below, golden autumn trees in front and leaves in the air."""
    rng = random.Random(1999)  # any fixed seed: the same picture every time
    img = Image.new("RGB", (W, H))
    px = img.load()
    sun = (250, 154)  # half under the far hills: a sunset

    # The sky: violet up high, through pink and orange, to gold at the horizon.
    stops = [(0, "#2e3478"), (34, "#6a4a9a"), (70, "#c4567e"), (102, "#ee6e4e"), (128, "#fb9a46"), (148, "#ffc35e"), (161, "#ffe28e")]
    for (y0, c0), (y1, c1) in zip(stops, stops[1:]):
        dithered_band(img, y0, y1, hexc(c0), hexc(c1), 4)

    # Streaks of cloud lit from below.
    for _ in range(16):
        cy = rng.randrange(14, 120)
        cx = rng.randrange(-40, W)
        length = rng.randrange(40, 140)
        thick = rng.randrange(2, 5)
        warm = cy / 120
        body = lerp(hexc("#6e4a9a"), hexc("#d8564e"), warm)
        lit = lerp(hexc("#f08ab0"), hexc("#ffc070"), warm)
        for i in range(length):
            x = cx + i
            taper = min(i, length - i) / (length / 2)
            t = max(1, int(thick * taper + 0.5))
            for dy in range(t):
                y = cy + dy + int(2 * math.sin((x + cy) / 23))
                if 0 <= x < W and 0 <= y < H:
                    px[x, y] = lit if dy == t - 1 else body

    HORIZON = 160
    HAZE = hexc("#ffd29a")

    def depth(y):
        """0 at the horizon, 1 at the bottom edge: how near the ground at row y is."""
        return max(0.0, min(1.0, (y - HORIZON) / (H - HORIZON)))

    def fogged(c, f, x, y, towards=HAZE):
        """Dithered haze: a share `f` (0..1) of the pixels move most of the way to `towards`."""
        if f <= 0:
            return c
        return lerp(c, towards, 0.6) if BAYER[y % 4][x % 4] < f * 16 else lerp(c, towards, f * 0.25)

    # A long blue ridge behind the plain.
    ridge_line = ridge(rng, 152, 4)
    for x, top in enumerate(ridge_line):
        for y in range(int(top), HORIZON):
            px[x, y] = lerp(hexc("#a088b4"), hexc("#efbd8c"), (y - top) / 10)

    # The sun and its glow, setting behind the ridge.
    sx, sy = sun
    for y in range(max(0, sy - 70), min(H, sy + 40)):
        for x in range(sx - 90, sx + 91):
            if not 0 <= x < W:
                continue
            d = math.hypot(x - sx, (y - sy) * 1.2)
            behind = y >= ridge_line[x]
            if d <= 12 and not behind:
                px[x, y] = hexc("#fffbe6")
            elif d <= 17 and not behind:
                px[x, y] = hexc("#ffe27a")
            elif d <= 90 and BAYER[y % 4][x % 4] < (90 - d) / 5.0:
                px[x, y] = lerp(px[x, y], hexc("#ffd890"), 0.5)

    # The snowy peak on the right. Far away: the air lightens it, and its foot melts into the
    # haze over the plain; its outline is softened into the sky with a dithered fringe.
    peak_x, peak_y, base = 372, 58, HORIZON
    noise = ridge(rng, 0, 7)
    gullies = ridge(rng, 0, 9)
    tops = []
    for x in range(W):
        top = peak_y + abs(x - peak_x) ** 1.15 * 0.62 + noise[x] * (abs(x - peak_x) / 40)
        if abs(x - peak_x) < 4:
            top = peak_y + 2 + noise[x] * 0.3  # a flat crater top
        tops.append(top)
        for y in range(int(top), base):
            d = y - top
            sunny = x < peak_x  # lit from the sun on the left
            snowline = 26 + 7 * math.sin(x / 4.0) + gullies[x] * 1.6 + (6 if (x // 3) % 3 == 0 else 0)
            if y - peak_y < snowline:
                c = hexc("#fff4ec") if sunny else hexc("#cfc9e8")
                if sunny and d < 2:
                    c = hexc("#ffd8b0")
            else:
                c = hexc("#a2708f") if sunny else hexc("#6a5688")
                if abs((x - peak_x) % 13 - 6) < 1:  # ridges down the slope
                    c = hexc("#b8849a") if sunny else hexc("#584878")
                elif (x * 13 + y * 7) % 17 == 0:
                    c = hexc("#8e6488") if sunny else hexc("#5e4c80")
            # Aerial perspective: lighter and warmer towards its foot.
            c = fogged(c, min(0.65, ((y - 118) / 42) ** 1.6) if y > 118 else 0, x, y)
            px[x, y] = c
    for x, top in enumerate(tops):
        if int(top) >= base - 2:
            continue
        edge = px[x, int(top) + 1]
        for k, share in ((1, 0.5), (2, 0.25)):
            y = int(top) - k
            if 0 <= y and BAYER[y % 4][x % 4] < share * 16:
                px[x, y] = lerp(px[x, y], edge, 0.5)

    def vnoise(x, y, scale, seed):
        """Smooth value noise in 0..1: random corners, blended."""
        gx, gy = x / scale, y / scale
        x0, y0 = int(math.floor(gx)), int(math.floor(gy))
        fx, fy = gx - x0, gy - y0
        def corner(i, j):
            return random.Random(hash((i, j, seed)) & 0xFFFFFFFF).random()
        fx, fy = fx * fx * (3 - 2 * fx), fy * fy * (3 - 2 * fy)
        top = corner(x0, y0) * (1 - fx) + corner(x0 + 1, y0) * fx
        bottom = corner(x0, y0 + 1) * (1 - fx) + corner(x0 + 1, y0 + 1) * fx
        return top * (1 - fy) + bottom * fy

    # The land: hazy gold at the horizon, rich and textured nearer, in patches like foliage.
    far, near = hexc("#efc47a"), hexc("#b07a30")
    patches = (hexc("#7c8636"), hexc("#94622a"), hexc("#a8a040"))
    for y in range(HORIZON, H):
        p_ = depth(y)
        for x in range(W):
            base_c = lerp(far, near, p_ ** 0.7)
            # Patches: bigger near, finer far (perspective in the noise too).
            scale = 3 + p_ * 14
            n = vnoise(x, y * (2.2 - p_), scale, 7) * 0.65 + vnoise(x, y, scale / 3, 8) * 0.35
            shade = n + (BAYER[y % 4][x % 4] / 16 - 0.5) * 0.35
            kind = vnoise(x, y * 1.5, 30, 9)  # which foliage this patch is
            if shade < 0.36:
                c = lerp(base_c, patches[int(kind * 2.99)], 0.45 + p_ * 0.3)
            elif shade < 0.44 and p_ > 0.3:
                c = lerp(base_c, patches[int(kind * 2.99)], 0.25)
            elif shade > 0.66:
                c = lerp(base_c, hexc("#ffe08a"), 0.35)
            else:
                c = base_c
            px[x, y] = c
    # Tufts of grass and leaf litter, larger nearer.
    for _ in range(900):
        y = rng.randrange(HORIZON + 4, H)
        x = rng.randrange(W)
        p_ = depth(y)
        size = 1 + int(p_ * 3)
        c = rng.choice([hexc("#ffe08a"), hexc("#7e8a34"), hexc("#9a5a22"), hexc("#d8a040")])
        for i in range(size):
            if 0 <= x + i - size // 2 < W and 0 <= y - i < H:
                px[x + i - size // 2, y - (i % 2)] = c

    # A lake glinting under the sun.
    for y in range(HORIZON + 1, HORIZON + 15):
        half = 30 - (y - HORIZON - 1) * 1.6
        for x in range(int(sx - half), int(sx + half)):
            if rng.random() < 0.55 and y % 2:
                px[x, y] = hexc("#fff3c0") if abs(x - sx) < half / 2 else hexc("#ffcf7a")

    # The town: rows of houses, smaller and hazier towards the horizon.
    def house(x, y, w):
        h = max(2, w * 2 // 3)
        wall = rng.choice([hexc("#f2dcae"), hexc("#e9c98e"), hexc("#d9b07a")])
        side = hexc("#b48250")
        roof = rng.choice([hexc("#3d4c78"), hexc("#4a3a5e"), hexc("#5a3b2a")])
        f = max(0.0, 1 - depth(y) * 3.2) * 0.8  # haze on the far rows
        rh = max(2, w // 3)
        for yy in range(y - h, y):
            for xx in range(x, x + w):
                if 0 <= xx < W and 0 <= yy < H:
                    px[xx, yy] = fogged(wall if xx > x else side, f, xx, yy)
        if w >= 8 and rng.random() < 0.6:  # a lit window
            wx = x + w // 2
            if 0 <= wx < W:
                px[wx, y - h // 2] = hexc("#ffcf4a")
        for i in range(rh):
            for xx in range(x - 1 - w // 8 + i // 2, x + w + 1 + w // 8 - i // 2):
                yy = y - h - i
                if 0 <= xx < W and 0 <= yy < H:
                    px[xx, yy] = fogged(hexc("#ffb36a") if i == rh - 1 else roof, f, xx, yy)

    y = HORIZON + 8
    while y < H + 4:
        p_ = depth(y)
        w_lo, w_hi = 4 + int(p_ * 11), 6 + int(p_ * 16)
        spread = 60 + p_ * 260  # the valley widens towards us
        x = int(sx + 20 - spread / 2 + rng.randrange(-6, 7))
        while x < sx + 20 + spread / 2:
            w = rng.randrange(w_lo, w_hi + 1)
            # Clusters: now and then a gap (gardens, lanes) where trees go.
            if vnoise(x, y, 26, 11) > 0.38 and not (x < 175 and y < 230):
                house(x, y, w)
            x += w + rng.randrange(1, 3 + int(p_ * 8))
        y += 4 + int(p_ * 14)

    # The pagoda on the left: nine roofs, warm wood, gold trim, lit from the sun.
    tx, ground = 92, 214
    for y in range(ground - 12, ground + 10):  # a grassy mound
        half = math.sqrt(max(0, y - (ground - 13))) * 9 + 8
        for x in range(int(tx - half), int(tx + half)):
            if 0 <= x < W:
                lit = x > tx + (y - ground) * 0.3
                c = hexc("#8e9a3e") if lit else hexc("#5e6a2c")
                if (x * 5 + y * 11) % 7 == 0:
                    c = hexc("#b0b04a") if lit else hexc("#4a5424")
                px[x, y] = c
    for y in range(ground - 6, ground):  # its stone plinth
        for x in range(tx - 19, tx + 20):
            c = hexc("#9c8c78") if y == ground - 6 else (hexc("#8a7a6a") if (x + y) % 5 else hexc("#6e604f"))
            px[x, y] = c
    tier_w, y = 34, ground - 6
    for tier in range(9):
        h = 9
        for yy in range(y - h, y):
            for xx in range(tx - tier_w // 2 + 4, tx + tier_w // 2 - 4):
                lit = xx > tx  # the sun is to the right of it
                px[xx, yy] = (hexc("#9a5a34") if lit else hexc("#5e3420")) if (xx - tx) % 5 else hexc("#3e2416")
            if tier % 2 == 0:
                px[tx + 3, y - 5] = hexc("#ffd060")
        for xx in range(tx - tier_w // 2 - 3, tx + tier_w // 2 + 3):
            px[xx, y - h] = hexc("#3a3048")
            px[xx, y - h + 1] = hexc("#2a2238")
            if xx > tx:
                px[xx, y - h - 1] = hexc("#ffb36a")  # sunlit roof edge
        for xx in (tx - tier_w // 2 - 3, tx + tier_w // 2 + 2):
            px[xx, y - h - 1] = hexc("#f4c84a")
            px[xx, y - h - 2] = hexc("#f4c84a")
        y -= h + 1
        tier_w = max(12, tier_w - 2)
    for yy in range(y - 12, y + 1):
        px[tx, yy] = hexc("#3a3048")
    for yy in (y - 13, y - 14):
        px[tx, yy] = hexc("#f4c84a")

    # Golden autumn trees, from small and hazy far away to big in front.
    def tree(cx, cy, r, colours):
        p_ = depth(cy)
        light, mid, dark = (lerp(c, HAZE, (1 - p_) * 0.45) for c in colours)
        trunk_w = 1 if r < 9 else 2
        for yy in range(cy, min(H, cy + max(2, r // 2) + 2)):
            for xx in range(cx - trunk_w // 2, cx - trunk_w // 2 + trunk_w):
                if 0 <= xx < W:
                    px[xx, yy] = lerp(hexc("#4a2a18"), HAZE, (1 - p_) * 0.4)
        blobs = [
            (cx + rng.randrange(-r, r + 1), cy + rng.randrange(-r // 2, r // 3 + 1), rng.randrange(max(2, r // 3), r // 2 + 3))
            for _ in range(4 + r // 3)
        ]
        blobs.sort(key=lambda b: b[1])
        for bx, by, br in blobs:
            for yy in range(by - br, by + br + 1):
                for xx in range(bx - br, bx + br + 1):
                    if not (0 <= xx < W and 0 <= yy < H):
                        continue
                    d = math.hypot(xx - bx, (yy - by) * 1.15)
                    if d > br:
                        continue
                    toward = (xx - bx) * (1 if bx < sx else -1) + (by - yy)
                    if d > br - 1.2 and toward < 0:
                        c = dark
                    elif toward > br * 0.5:
                        c = light
                    else:
                        c = mid if BAYER[yy % 4][xx % 4] < 11 else dark
                    # Leaves: speckles of light and shade, finer far away.
                    grain = rng.random()
                    if grain < 0.06 + p_ * 0.06:
                        c = hexc("#fff0a0") if toward > 0 else dark
                    elif grain > 0.94:
                        c = mid
                    px[xx, yy] = c

    golden = (hexc("#ffd75e"), hexc("#f0a830"), hexc("#a8561a"))
    orange = (hexc("#ffb85a"), hexc("#e8762a"), hexc("#963e16"))
    red = (hexc("#ff9a6a"), hexc("#d8503a"), hexc("#7e2a1a"))
    green = (hexc("#c6d870"), hexc("#7ea444"), hexc("#46682a"))
    spots = []
    for _ in range(120):
        side = rng.random()
        cy = int(HORIZON + 6 + (H + 6 - HORIZON - 6) * rng.random() ** 0.8)
        p_ = depth(cy)
        if side < 0.45:
            cx = rng.randrange(-20, int(150 + (1 - p_) * 60))
        elif side < 0.9:
            cx = rng.randrange(int(320 - (1 - p_) * 30), W + 20)
        else:
            cx = rng.randrange(170, 340)  # trees among the houses
        spots.append((cy, cx))
    for cy, cx in sorted(spots):  # back to front
        p_ = depth(cy)
        r = max(2, int(2 + p_ * 18 + rng.randrange(-2, 3)))
        tree(cx, cy, r, rng.choice([golden, golden, orange, orange, red, green]))

    # Sun rays: soft dithered beams fanning out from the sun, fading with distance.
    rays = [(-1.32, 0.05), (-0.95, 0.08), (-0.6, 0.05), (-0.28, 0.07), (0.1, 0.05), (0.42, 0.08),
            (0.78, 0.05), (1.12, 0.07), (1.45, 0.05), (-1.95, 0.10), (2.0, 0.10)]
    glow = hexc("#fff1c4")
    for y in range(H):
        for x in range(W):
            dx, dy = x - sx, sy - y
            dist = math.hypot(dx, dy)
            if dist < 18:
                continue
            a = math.atan2(dx, dy)  # 0 = straight up
            s_ = 0.0
            for ra, rw in rays:
                diff = abs((a - ra + math.pi) % (2 * math.pi) - math.pi)
                s_ = max(s_, 1 - diff / rw)
            if s_ <= 0:
                continue
            s_ *= max(0.0, 1 - dist / 420) * (0.55 if y > HORIZON else 1.0)
            if BAYER[y % 4][x % 4] < s_ * 9:
                px[x, y] = lerp(px[x, y], glow, 0.4)

    # Leaves in the air: small golden diamonds.
    for _ in range(26):
        x, y = rng.randrange(W), rng.randrange(10, H - 10)
        c = rng.choice([hexc("#ffd75e"), hexc("#f0a830"), hexc("#ffe9a0")])
        for dx, dy in ((0, 0), (1, 0), (-1, 0), (0, 1), (0, -1), (1, -1)):
            if 0 <= x + dx < W and 0 <= y + dy < H:
                px[x + dx, y + dy] = c if (dx, dy) != (1, -1) else hexc("#b8661c")
    return img


# 12x12 icons, drawn as text masks: one letter per colour.
ICONS = {
    "continue": (  # a play arrow
        {"g": "#4ade80", "d": "#22874a"},
        [
            "............",
            "..gg........",
            "..gggg......",
            "..gggggg....",
            "..gggggggg..",
            "..ggggggggg.",
            "..gggggggdd.",
            "..ggggggdd..",
            "..ggggdd....",
            "..ggdd......",
            "..dd........",
            "............",
        ],
    ),
    "play": (  # the play arrow in cream, for the green PLAY button
        {"g": "#fff4e0", "d": "#c9b48a"},
        [
            "............",
            "..gg........",
            "..gggg......",
            "..gggggg....",
            "..gggggggg..",
            "..ggggggggg.",
            "..gggggggdd.",
            "..ggggggdd..",
            "..ggggdd....",
            "..ggdd......",
            "..dd........",
            "............",
        ],
    ),
    "join": (  # a key
        {"y": "#f4c84a", "d": "#a8741b"},
        [
            "....yyyy....",
            "...yy..yy...",
            "...y....y...",
            "...yy..yy...",
            "....yyyy....",
            ".....yy.....",
            ".....yy.....",
            ".....yyyy...",
            ".....yy.....",
            ".....yyyy...",
            ".....yy.....",
            ".....dd.....",
        ],
    ),
    "create": (  # a house with a flag
        {"r": "#e2574c", "w": "#e8eaed", "b": "#5b6ea8", "d": "#3a4777", "f": "#f4c84a"},
        [
            ".....f......",
            ".....fff....",
            ".....f......",
            "....rrr.....",
            "...rrrrr....",
            "..rrrrrrr...",
            ".rrrrrrrrr..",
            "..wwwwwww...",
            "..wbbwddw...",
            "..wbbwddw...",
            "..wwwwddw...",
            "............",
        ],
    ),
    "restore": (  # a clock with a back arrow
        {"w": "#e8eaed", "b": "#7aa2ff", "d": "#3a4777"},
        [
            "....wwww....",
            "..ww....ww..",
            ".w...b....w.",
            ".w...b....w.",
            "w....b.....w",
            "w....bbb...w",
            "w..........w",
            ".w........w.",
            "bw........w.",
            "bbbw....ww..",
            "bb..wwww....",
            "............",
        ],
    ),
    "import": (  # a folder and an arrow into it
        {"y": "#d9a441", "l": "#f4c84a", "g": "#4ade80"},
        [
            ".....gg.....",
            ".....gg.....",
            "...gggggg...",
            "....gggg....",
            ".....gg.....",
            "yyy.........",
            "yllly.......",
            "yllllllllll.",
            "yllllllllll.",
            "yllllllllll.",
            "yyyyyyyyyyy.",
            "............",
        ],
    ),
    "settings": (  # a gear
        {"w": "#c9d4f2", "d": "#5b6ea8"},
        [
            ".....ww.....",
            "..w..ww..w..",
            "...wwwwww...",
            "...ww..ww...",
            "wwww....wwww",
            "wwww....wwww",
            "...ww..ww...",
            "...wwwwww...",
            "..w..ww..w..",
            ".....ww.....",
            "............",
            "............",
        ],
    ),
    "leave": (  # an arrow out of a door
        {"w": "#e8eaed", "r": "#f87171"},
        [
            "wwwwww......",
            "w....w......",
            "w....w...r..",
            "w....w...rr.",
            "w..rrrrrrrrr",
            "w..rrrrrrrrr",
            "w....w...rr.",
            "w....w...r..",
            "w....w......",
            "wwwwww......",
            "............",
            "............",
        ],
    ),
}


def icon(colours, rows):
    img = Image.new("RGBA", (12, 12), (0, 0, 0, 0))
    px = img.load()
    for y, row in enumerate(rows):
        for x, ch in enumerate(row):
            if ch != ".":
                px[x, y] = hexc(colours[ch]) + (255,)
    return img


def app_icon():
    """GoldLink's 32x32 app icon: two gold chain links on a sunset tile (saved at 1x and scaled to 256)."""
    n = 32
    img = Image.new("RGBA", (n, n), (0, 0, 0, 0))
    px = img.load()
    top, bottom = hexc("#f6a03c"), hexc("#5a2d6e")
    for y in range(n):
        for x in range(n):
            # a rounded tile: corners of radius 5 cut away
            cx = min(max(x, 5), n - 6)
            cy = min(max(y, 5), n - 6)
            if (x - cx) ** 2 + (y - cy) ** 2 > 25:
                continue
            t = y / (n - 1)
            steps = 6
            level = t * steps
            base = int(level)
            frac = level - base
            if BAYER[y % 4][x % 4] / 16 < frac:
                base += 1
            px[x, y] = lerp(top, bottom, min(base, steps) / steps) + (255,)
    edge = hexc("#3a1d0c")
    gold, light, shade = hexc("#f4c84a"), hexc("#fff1a8"), hexc("#b8801f")

    def link(cx, cy, rx, ry, w):
        """One chain link as {cell: colour}: an elliptical ring with a dark outline, lit from the top left."""
        ring = {}
        for y in range(n):
            for x in range(n):
                fx, fy = (x + 0.5 - cx), (y + 0.5 - cy)
                if (fx / rx) ** 2 + (fy / ry) ** 2 <= 1 and (fx / (rx - w)) ** 2 + (fy / (ry - w)) ** 2 >= 1:
                    lit = fy < 0 and fx < rx * 0.3 and (fx / (rx - w * 0.5)) ** 2 + (fy / (ry - w * 0.5)) ** 2 > 1
                    ring[(x, y)] = light if lit else shade if fy > ry * 0.45 else gold
        sprite = {}
        for (x, y) in ring:
            for dx, dy in ((1, 0), (-1, 0), (0, 1), (0, -1)):
                q = (x + dx, y + dy)
                if q not in ring and 0 <= q[0] < n and 0 <= q[1] < n:
                    sprite[q] = edge
        sprite.update(ring)
        return sprite

    left = link(11, 16, 8, 5.5, 2.2)
    right = link(21, 16, 8, 5.5, 2.2)
    # the right link passes over the left one at the top and under it at the bottom
    for sprite, keep in ((left, lambda q: True), (right, lambda q: True), (left, lambda q: q[1] >= 16 and q[0] >= 13)):
        for q, c in sprite.items():
            if keep(q):
                px[q] = c + (255,)
    return img


def main():
    OUT.mkdir(parents=True, exist_ok=True)
    background().save(OUT / "background.png", optimize=True)
    for name, (colours, rows) in ICONS.items():
        icon(colours, rows).save(OUT / f"icon-{name}.png", optimize=True)
    small = app_icon()
    big = small.resize((256, 256), Image.NEAREST)
    big.save(OUT / "goldlink.png", optimize=True)
    # the Windows program's icon (build.rs): whole-pixel scales where they exist, smoothed below 32
    # and at 48, the sizes Explorer and the taskbar ask for
    sizes = [16, 24, 32, 48, 64, 128, 256]
    frames = [
        small.resize((n, n), Image.NEAREST) if n % 32 == 0 else big.resize((n, n), Image.LANCZOS)
        for n in sizes
    ]
    frames[-1].save(OUT / "goldlink.ico", sizes=[(n, n) for n in sizes], append_images=frames[:-1])
    print(f"wrote {OUT}")


if __name__ == "__main__":
    main()
