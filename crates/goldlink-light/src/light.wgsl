// GoldLink's lighting on the GPU (UC14): the overworld lit per DS pixel, as lib.rs
// `overworld_scene` did on the CPU (the same steps, constants and arithmetic): normals and
// ambient occlusion from the pixels' places, each card's light, then the key light, the point
// lights, the cast shadows, the grade and the vignette.

const EDGE: f32 = 12.0;
const AO_RANGE: f32 = 20.0;
const AMBIENT_FLOOR: f32 = 0.55;
const CONTRAST: f32 = 0.10;
const NORMAL_OFFSET: f32 = 0.4;
const BIAS: f32 = 0.4;
// Nothing nearer a point light than this casts (shadow.rs `NEAR`).
const NEAR: f32 = 0.5;
// How much wider than 90 degrees a point light's face sees (shadow.rs `WIDEN`).
const WIDEN: f32 = 1.10;
const TREE_HEIGHT: f32 = 44.0;
// Cards whose feet are nearer than this (view units; a tile is 16) don't shadow each other's light.
const CARD_NEAR: f32 = 12.0;
// Words per pixel out of the light pass (gpu.rs `WORDS`).
const WORDS: u32 = 7u;

struct Frame {
    // The picture's width and height, its column 0's column in the scene's picture (the card
    // map's), the card map's width.
    dims: vec4<u32>,
    // Lights, cards, point light slots, flags (1: the shadows are ready, 2: the sun's map).
    counts: vec4<u32>,
    // The sun's strength, the lighting's normalisation, the occlusion's factor, the vignette.
    params: vec4<f32>,
    key_dir: vec4<f32>,
    ambient: vec4<f32>,
    key: vec4<f32>,
    // The grade: tint (w: saturation), the shadows' tint (w: exposure), the highlights'.
    tint: vec4<f32>,
    shadow: vec4<f32>,
    high: vec4<f32>,
    // The ground's normal (the shadows').
    up: vec4<f32>,
    // The projection (vectors on the left: column k is the CPU's row k) and its inverse.
    proj: mat4x4<f32>,
    rows: mat4x4<f32>,
    // Where the polygons truly land (shadow.rs `fit`).
    fit: vec4<f32>,
    // The sun's map: toward the sun, its axes, (origin u, origin v, texels per unit), its size.
    sun_dir: vec4<f32>,
    sun_u: vec4<f32>,
    sun_v: vec4<f32>,
    sun_view: vec4<f32>,
    sun_side: vec4<u32>,
    // The occlusion's taps (x, y).
    taps: array<vec4<i32>, 8>,
    // The bloom: its strength, the brightness (linear luminance) it starts from; the light
    // sources' own glow's strength, how much brighter than their colour they shine.
    bloom: vec4<f32>,
}

// One pixel: its place (view space; w: 1 the 3D scene), its colour (0x00RRGGBB, unfaded), its
// glow (bits of a float), 1 where the player shows, how much it blooms as a light source (bits of
// a float; -1: not one; -2: a card's, a sprite's, which doesn't bloom).
struct Pixel {
    pos: vec4<f32>,
    info: vec4<u32>,
}

// A light: where (w: reach), its colour (w: its shadow maps' slot, -1 none).
struct Light {
    pos: vec4<f32>,
    colour: vec4<f32>,
}

// A point light's face: the light's place as the polygons put it (w: texels a side, 0 not
// drawn), its axes (w: where its tile starts in the atlas, x then y).
struct Face {
    pos: vec4<f32>,
    forward: vec4<f32>,
    right: vec4<f32>,
    upward: vec4<f32>,
}

// A card (shadow.rs `Card`): its plane's point (w: 1 attached), normal, across, up, its extent
// (a, b), its foot.
struct Card {
    origin: vec4<f32>,
    normal: vec4<f32>,
    across: vec4<f32>,
    up: vec4<f32>,
    ab: vec4<f32>,
    foot: vec4<f32>,
}

@group(0) @binding(0) var<uniform> F: Frame;
@group(0) @binding(1) var<storage, read> pix: array<Pixel>;
// The raw normals (xyz) and the raw occlusion (w).
@group(0) @binding(2) var<storage, read_write> work0: array<vec4<f32>>;
// The smoothed normals (xyz) and occlusion (w).
@group(0) @binding(3) var<storage, read_write> work1: array<vec4<f32>>;
@group(0) @binding(4) var<storage, read_write> outp: array<u32>;
@group(0) @binding(5) var<storage, read> lights: array<Light>;
@group(0) @binding(6) var<storage, read> faces: array<Face>;
@group(0) @binding(7) var<storage, read> cards: array<Card>;
// Per card: the sun's light, then each slot's.
@group(0) @binding(8) var<storage, read_write> card_light: array<f32>;
@group(0) @binding(9) var sun_raw: texture_2d<f32>;
@group(0) @binding(10) var sun_card: texture_2d<u32>;
@group(0) @binding(11) var atlas_raw: texture_2d<f32>;
@group(0) @binding(12) var atlas_card: texture_2d<u32>;
@group(0) @binding(13) var card_px: texture_2d<u32>;
// The bloom at half size (its own bind group's: the light pass's has the most storage buffers a
// stage may have), blurred from one to the other and back.
@group(0) @binding(14) var<storage, read_write> bloom_a: array<vec4<f32>>;
@group(0) @binding(15) var<storage, read_write> bloom_b: array<vec4<f32>>;
// The light sources' own glow at full size, blurred from one to the other and back.
@group(0) @binding(16) var<storage, read_write> glow_a: array<vec4<f32>>;
@group(0) @binding(17) var<storage, read_write> glow_b: array<vec4<f32>>;

fn is3d(i: u32) -> bool {
    return pix[i].pos.w > 0.5;
}

fn safe_normalize(a: vec3<f32>) -> vec3<f32> {
    let l = sqrt(dot(a, a));
    if l > 1e-9 {
        return a * (1.0 / l);
    }
    return vec3<f32>(0.0, 0.0, 1.0);
}

fn lum(c: vec3<f32>) -> f32 {
    return 0.2126 * c.x + 0.7152 * c.y + 0.0722 * c.z;
}

// The nearer of two one-sided differences (lib.rs `normals`' pick).
fn pick(hf: bool, f: vec3<f32>, hb: bool, b: vec3<f32>, out: ptr<function, vec3<f32>>) -> bool {
    var best: vec3<f32>;
    if hf && hb {
        best = select(b, f, abs(f.z) <= abs(b.z));
    } else if hf {
        best = f;
    } else if hb {
        best = b;
    } else {
        return false;
    }
    if abs(best.z) < EDGE {
        *out = best;
        return true;
    }
    return false;
}

@compute @workgroup_size(8, 8)
fn normals_raw(@builtin(global_invocation_id) g: vec3<u32>) {
    let w = F.dims.x;
    let h = F.dims.y;
    if g.x >= w || g.y >= h {
        return;
    }
    let x = g.x;
    let y = g.y;
    let i = y * w + x;
    var out = vec3<f32>(0.0, 0.0, 1.0);
    if is3d(i) {
        let c = pix[i].pos.xyz;
        let hr = x + 1u < w && is3d(i + 1u);
        let hl = x > 0u && is3d(i - 1u);
        let hu = y > 0u && is3d(i - w);
        let hd = y + 1u < h && is3d(i + w);
        var right = vec3<f32>(0.0);
        var left = vec3<f32>(0.0);
        var upv = vec3<f32>(0.0);
        var down = vec3<f32>(0.0);
        if hr { right = pix[i + 1u].pos.xyz - c; }
        if hl { left = c - pix[i - 1u].pos.xyz; }
        if hu { upv = pix[i - w].pos.xyz - c; }
        if hd { down = c - pix[i + w].pos.xyz; }
        var a: vec3<f32>;
        var b: vec3<f32>;
        if pick(hr, right, hl, left, &a) && pick(hu, upv, hd, down, &b) {
            let n = safe_normalize(cross(a, b));
            out = select(n, -n, dot(n, c) > 0.0);
        }
    }
    work0[i] = vec4<f32>(out, 1.0);
}

@compute @workgroup_size(8, 8)
fn normals_smooth(@builtin(global_invocation_id) g: vec3<u32>) {
    let w = F.dims.x;
    let h = F.dims.y;
    if g.x >= w || g.y >= h {
        return;
    }
    let i = g.y * w + g.x;
    if !is3d(i) {
        work1[i] = vec4<f32>(0.0, 0.0, 1.0, 1.0);
        return;
    }
    let z = pix[i].pos.z;
    let y0 = select(g.y - 1u, 0u, g.y == 0u);
    let y1 = min(g.y + 1u, h - 1u);
    let x0 = select(g.x - 1u, 0u, g.x == 0u);
    let x1 = min(g.x + 1u, w - 1u);
    var sum = vec3<f32>(0.0);
    for (var yy = y0; yy <= y1; yy++) {
        for (var xx = x0; xx <= x1; xx++) {
            let j = yy * w + xx;
            if is3d(j) && abs(pix[j].pos.z - z) < 3.0 {
                sum += work0[j].xyz;
            }
        }
    }
    work1[i] = vec4<f32>(safe_normalize(sum), 1.0);
}

@compute @workgroup_size(8, 8)
fn occlusion_raw(@builtin(global_invocation_id) g: vec3<u32>) {
    let w = F.dims.x;
    let h = F.dims.y;
    if g.x >= w || g.y >= h {
        return;
    }
    let i = g.y * w + g.x;
    var out = 1.0;
    if is3d(i) {
        let p = pix[i].pos.xyz;
        let n = work1[i].xyz;
        var occ = 0.0;
        for (var k = 0; k < 8; k++) {
            let xx = i32(g.x) + F.taps[k].x;
            let yy = i32(g.y) + F.taps[k].y;
            if xx < 0 || yy < 0 || xx >= i32(w) || yy >= i32(h) {
                continue;
            }
            let j = u32(yy) * w + u32(xx);
            if !is3d(j) {
                continue;
            }
            let v = pix[j].pos.xyz - p;
            let d2 = dot(v, v);
            let along = dot(n, v);
            if along <= 0.0 || d2 >= AO_RANGE * AO_RANGE || along * along <= 0.0225 * d2 {
                continue;
            }
            let d = sqrt(d2);
            occ += (along / d - 0.15) * (1.0 - d / AO_RANGE);
        }
        out = clamp(1.0 - F.params.z * occ, 0.5, 1.0);
    }
    work0[i] = vec4<f32>(work0[i].xyz, out);
}

@compute @workgroup_size(8, 8)
fn occlusion_smooth(@builtin(global_invocation_id) g: vec3<u32>) {
    let w = F.dims.x;
    let h = F.dims.y;
    if g.x >= w || g.y >= h {
        return;
    }
    let i = g.y * w + g.x;
    if !is3d(i) {
        work1[i] = vec4<f32>(work1[i].xyz, 1.0);
        return;
    }
    let z = pix[i].pos.z;
    let y0 = select(g.y - 1u, 0u, g.y == 0u);
    let y1 = min(g.y + 1u, h - 1u);
    let x0 = select(g.x - 1u, 0u, g.x == 0u);
    let x1 = min(g.x + 1u, w - 1u);
    var sum = 0.0;
    var cnt = 0.0;
    for (var yy = y0; yy <= y1; yy++) {
        for (var xx = x0; xx <= x1; xx++) {
            let j = yy * w + xx;
            if is3d(j) && abs(pix[j].pos.z - z) < 3.0 {
                sum += work0[j].w;
                cnt += 1.0;
            }
        }
    }
    work1[i] = vec4<f32>(work1[i].xyz, sum / cnt);
}

// ---- The cast shadows (shadow.rs) ----

// A pixel's place as the lighting found it, moved to where the polygons put it (`true_place`);
// w 0 when there's none.
fn true_place(p: vec3<f32>) -> vec4<f32> {
    let c = p.x * F.proj[0] + p.y * F.proj[1] + p.z * F.proj[2] + F.proj[3];
    if abs(c.w) < 1e-9 || abs(F.fit.y) < 1e-3 || abs(F.fit.w) < 1e-3 {
        return vec4<f32>(0.0);
    }
    let xn = c.x / c.w;
    let yn = c.y / c.w;
    let zn = c.z / c.w;
    let col = 128.0 * xn + 191.5;
    let row = 95.5 - 96.0 * yn;
    let x2 = (col - F.fit.x) / F.fit.y;
    let y2 = (row - F.fit.z) / F.fit.w;
    let r = x2 * F.rows[0] + y2 * F.rows[1] + zn * F.rows[2] + F.rows[3];
    if abs(r.w) < 1e-9 {
        return vec4<f32>(0.0);
    }
    let q = r.xyz / r.w;
    // Finite (NaN fails every comparison) and in front of the camera.
    if !(q.z < 0.0) || !(abs(q.x) < 3.0e38) || !(abs(q.y) < 3.0e38) || !(q.z > -3.0e38) {
        return vec4<f32>(0.0);
    }
    return vec4<f32>(q, 1.0);
}

fn holds(k: u32, p: vec3<f32>) -> bool {
    let c = cards[k];
    let d = p - c.origin.xyz;
    let a = dot(d, c.across.xyz);
    let b = dot(d, c.up.xyz);
    return abs(dot(d, c.normal.xyz)) < 2.5 && a > c.ab.x - 1.5 && a < c.ab.y + 1.5
        && b > c.ab.z - 1.5 && b < c.ab.w + 1.5;
}

// What lies over `q` on the sun's map: -1 nothing, else the card (index + 1; 0 a polygon).
fn over_sun(q: vec3<f32>, slope: f32) -> i32 {
    let texels = F.sun_view.z;
    let x = (dot(q, F.sun_u.xyz) - F.sun_view.x) * texels;
    let y = (dot(q, F.sun_v.xyz) - F.sun_view.y) * texels;
    let d = dot(q, F.sun_dir.xyz);
    if !(x >= 0.0) || !(y >= 0.0) {
        return -1;
    }
    let ix = u32(x);
    let iy = u32(y);
    if ix >= F.sun_side.x || iy >= F.sun_side.y {
        return -1;
    }
    let depth = textureLoad(sun_raw, vec2<u32>(ix, iy), 0).r;
    if depth > d + BIAS * (1.0 + slope / texels) {
        return i32(textureLoad(sun_card, vec2<u32>(ix, iy), 0).r);
    }
    return -1;
}

fn over_face(f: Face, q: vec3<f32>, slope: f32) -> i32 {
    let size = f.pos.w;
    if size <= 0.0 {
        return -1;
    }
    let d = q - f.pos.xyz;
    let z = dot(d, f.forward.xyz);
    if !(z >= NEAR) {
        return -1;
    }
    let x = (dot(d, f.right.xyz) / (z * WIDEN) * 0.5 + 0.5) * size;
    let y = (dot(d, f.upward.xyz) / (z * WIDEN) * 0.5 + 0.5) * size;
    if !(x >= 0.0) || !(y >= 0.0) {
        return -1;
    }
    let ix = u32(x);
    let iy = u32(y);
    if ix >= u32(size) || iy >= u32(size) {
        return -1;
    }
    let at = vec2<u32>(u32(f.forward.w) + ix, u32(f.right.w) + iy);
    let depth = textureLoad(atlas_raw, at, 0).r;
    if depth > 1.0 / max(z - BIAS * (1.0 + slope), NEAR) {
        return i32(textureLoad(atlas_card, at, 0).r);
    }
    return -1;
}

// How much of a light (the sun with `point` -1, else that slot) reaches `p` (shadow.rs
// `visible`): 0 or 1, `card` (an index, -1 none) left out.
fn visible(point: i32, p: vec3<f32>, n: vec3<f32>, attached: bool, card: i32) -> f32 {
    var to: vec3<f32>;
    var f: Face;
    if point < 0 {
        if (F.counts.w & 2u) == 0u {
            return 1.0;
        }
        to = F.sun_dir.xyz;
    } else {
        let base = u32(point) * 6u;
        let d = p - faces[base].pos.xyz;
        // The face whose axis is nearest the direction from the light (the last of equals, as
        // the CPU's max_by).
        var k = 0u;
        if abs(d.y) >= abs(d.x) { k = 1u; }
        if abs(d.z) >= abs(d[k]) { k = 2u; }
        let face = 2u * k + select(0u, 1u, d[k] < 0.0);
        f = faces[base + face];
        to = safe_normalize(-d);
    }
    let facing = clamp(dot(n, to), 0.05, 1.0);
    let slope = min(sqrt(1.0 - facing * facing) / facing, 3.0);
    let q = p + n * (NORMAL_OFFSET * (1.0 + slope));
    var o: i32;
    if point < 0 {
        o = over_sun(q, slope);
    } else {
        o = over_face(f, q, slope);
    }
    if o < 0 {
        return 1.0;
    }
    if o == 0 {
        return 0.0;
    }
    let k = u32(o - 1);
    if i32(k) == card || holds(k, p) || (!attached && cards[k].origin.w > 0.5) {
        return 1.0;
    }
    // A card lit as one isn't shadowed by a card standing where it stands (the player passing
    // their follower: lit at its feet in the follower's shadow, the player dimmed for a moment;
    // the user, 2026-11-18).
    if card >= 0 && length(cards[k].foot.xyz - cards[u32(card)].foot.xyz) < CARD_NEAR {
        return 1.0;
    }
    return 0.0;
}

// Each card's light, as one (shadow.rs `card_light`): the sun's, then each slot's.
@compute @workgroup_size(64)
fn cards_lit(@builtin(global_invocation_id) g: vec3<u32>) {
    let per = 1u + F.counts.z;
    if g.x >= F.counts.y * per {
        return;
    }
    let k = g.x / per;
    let j = g.x % per;
    let c = cards[k];
    let up = F.up.xyz;
    var flat: vec3<f32>;
    var point = -1;
    if j == 0u {
        if (F.counts.w & 2u) == 0u {
            card_light[g.x] = 1.0;
            return;
        }
        let dir = F.sun_dir.xyz;
        flat = safe_normalize(dir - up * dot(dir, up));
    } else {
        point = i32(j - 1u);
        let d = faces[(j - 1u) * 6u].pos.xyz - c.foot.xyz;
        flat = safe_normalize(d - up * dot(d, up));
    }
    let height = c.ab.w - c.ab.z;
    var v: f32;
    if height > TREE_HEIGHT {
        v = (visible(point, c.foot.xyz + up * (height * 0.3), flat, true, i32(k))
            + visible(point, c.foot.xyz + up * (height * 0.5), flat, true, i32(k))
            + visible(point, c.foot.xyz + up * (height * 0.7), flat, true, i32(k))) / 3.0;
    } else {
        v = visible(point, c.foot.xyz + up * 0.5, up, true, i32(k));
    }
    card_light[g.x] = v;
}

// A pixel's light (shadow.rs `light`): a card's pixels lit as the card.
fn light_at(point: i32, p: vec3<f32>, n: vec3<f32>, attached: bool, col: u32, row: u32) -> f32 {
    if (F.counts.w & 1u) == 0u {
        return 1.0;
    }
    if col < F.dims.w {
        let k = textureLoad(card_px, vec2<u32>(col, row), 0).r;
        if k > 0u {
            let c = k - 1u;
            if (!attached && cards[c].origin.w > 0.5) || holds(c, p) {
                return card_light[c * (1u + F.counts.z) + u32(point + 1)];
            }
        }
    }
    return visible(point, p, n, attached, -1);
}

// ---- Light, grade, out ----

fn to_linear(b: u32) -> f32 {
    let c = f32(b) / 255.0;
    if c <= 0.04045 {
        return c / 12.92;
    }
    return pow((c + 0.055) / 1.055, 2.4);
}

fn to_byte(v: f32) -> u32 {
    // The CPU's table: 4096 steps of linear light, sRGB, the contrast curve.
    let l = floor(clamp(v, 0.0, 1.0) * 4095.0 + 0.5) / 4095.0;
    var s: f32;
    if l <= 0.0031308 {
        s = l * 12.92;
    } else {
        s = 1.055 * pow(l, 1.0 / 2.4) - 0.055;
    }
    let curve = s * s * (3.0 - 2.0 * s);
    s = s + CONTRAST * (curve - s);
    return u32(clamp(s, 0.0, 1.0) * 255.0 + 0.5);
}

fn vignette(x: u32, y: u32) -> f32 {
    let strength = F.params.w;
    if strength <= 0.0 {
        return 1.0;
    }
    let w = f32(F.dims.x);
    let h = f32(F.dims.y);
    let dx = (f32(x) + 0.5 - w / 2.0) / (h * 0.75);
    let dy = (f32(y) + 0.5 - h / 2.0) / (h / 2.0);
    let t = clamp((dx * dx + dy * dy - 0.45) / 1.3, 0.0, 1.0);
    return 1.0 - strength * t * sqrt(t);
}

fn grade(c0: vec3<f32>, v: f32) -> vec3<f32> {
    var c = c0 * F.tint.xyz * F.shadow.w;
    let l = lum(c);
    c = vec3<f32>(l) + (c - vec3<f32>(l)) * F.tint.w;
    let lift = min(l * 4.0, 1.0);
    c = c * mix(F.shadow.xyz, vec3<f32>(1.0), lift);
    let high = clamp((l - 0.35) / 0.65, 0.0, 1.0);
    c = c * mix(vec3<f32>(1.0), F.high.xyz, high);
    return max(c * v, vec3<f32>(0.0));
}

// A pixel's light (times its colour, over the normalisation): as the world there (`player`
// false: its normal and occlusion, what the camera follows casting) or as the player.
fn light_of(i: u32, player: bool, col: u32, row: u32) -> vec3<f32> {
    let px = pix[i];
    var n = work1[i].xyz;
    var ao = work1[i].w;
    if player {
        n = vec3<f32>(0.0, 0.0, 1.0);
        ao = 1.0;
    }
    let raw = work0[i].xyz;
    let tp = true_place(px.pos.xyz);
    let wrap = clamp((dot(n, F.key_dir.xyz) + 0.35) / 1.35, 0.0, 1.0);
    let sun_strength = F.params.x;
    var sun_vis = 1.0;
    if sun_strength > 0.0 && tp.w > 0.5 {
        sun_vis = light_at(-1, tp.xyz, raw, !player, col, row);
    }
    let shade_k = 1.0 - sun_strength * (1.0 - sun_vis);
    var light = F.ambient.xyz * (AMBIENT_FLOOR + (1.0 - AMBIENT_FLOOR) * ao)
        + F.key.xyz * (wrap * ao * shade_k);
    for (var l = 0u; l < F.counts.x; l++) {
        let li = lights[l];
        let v = li.pos.xyz - px.pos.xyz;
        let d2 = dot(v, v);
        let range = li.pos.w;
        if d2 >= range * range {
            continue;
        }
        let d = max(sqrt(d2), 1e-3);
        let fall = 1.0 - d / range;
        let facing = max(dot(n, v) / d, 0.0);
        let add = li.colour.xyz * (fall * fall * (0.3 + 0.7 * facing));
        if lum(add) < 0.004 {
            continue;
        }
        var reach = 1.0;
        let slot = i32(li.colour.w);
        if slot >= 0 && tp.w > 0.5 {
            reach = light_at(slot, tp.xyz, raw, !player, col, row);
        }
        light += add * reach;
    }
    return light * (1.0 / F.params.y);
}

// Per pixel, seven words: its colour (0x00RRGGBB, `finish`'s), then the light as the world
// (half floats: red, green; blue, 0: a glowing colour's own light in it) and as the player (the same; the world's where the player
// isn't), for the player's picture put together with the world's on the window's GPU (each lit
// as itself), then the bloom there and the light sources' glow there (`finish`). Until `finish` the first and last hold the lit
// colour before the grade (half floats: red, green; blue, how much bloom it takes).
@compute @workgroup_size(8, 8)
fn shade(@builtin(global_invocation_id) g: vec3<u32>) {
    let w = F.dims.x;
    let h = F.dims.y;
    if g.x >= w || g.y >= h {
        return;
    }
    let i = g.y * w + g.x;
    let px = pix[i];
    let packed = px.info.x;
    let o = i * WORDS;
    if px.pos.w < 0.5 {
        outp[o] = packed;
        let none = pack2x16float(vec2<f32>(0.0, 0.0));
        outp[o + 1u] = none;
        outp[o + 2u] = none;
        outp[o + 3u] = none;
        outp[o + 4u] = none;
        outp[o + 5u] = 0u;
        outp[o + 6u] = 0u;
        return;
    }
    let base = vec3<f32>(
        to_linear((packed >> 16u) & 255u),
        to_linear((packed >> 8u) & 255u),
        to_linear(packed & 255u),
    );
    let glow = bitcast<f32>(px.info.y);
    let player = px.info.z == 1u;
    // A light source shines brighter than its colour, as much as its light shows (it fades in).
    let boost = 1.0 + (F.bloom.w - 1.0) * max(bitcast<f32>(px.info.w), 0.0);
    let col = F.dims.z + g.x;
    let world = light_of(i, false, col, g.y);
    var lit = world;
    var as_player = world;
    // The world's light with its glow in it (the colour's own light where it glows), for the
    // player's picture: base * light.
    var world_out = world * (1.0 - glow) + vec3<f32>(glow * boost);
    if player {
        as_player = light_of(i, true, col, g.y);
        lit = as_player;
        // Under the player the picture's colour is the player's: the world there doesn't glow.
        world_out = world;
    }
    var out = base;
    if glow < 1.0 || boost != 1.0 {
        out = base * lit * (1.0 - glow) + base * (glow * boost);
    }
    outp[o] = pack2x16float(out.xy);
    outp[o + 1u] = pack2x16float(world_out.xy);
    outp[o + 2u] = pack2x16float(vec2<f32>(world_out.z, 0.0));
    outp[o + 3u] = pack2x16float(as_player.xy);
    outp[o + 4u] = pack2x16float(vec2<f32>(as_player.z, 0.0));
    outp[o + 5u] = pack2x16float(vec2<f32>(out.z, takes(base)));
}

// ---- The bloom: what's bright (and the light sources) at half size, blurred, added back ----

// How much bloom a pixel takes: what's (nearly) black takes none (a room's dark surround doesn't
// glow).
fn takes(base: vec3<f32>) -> f32 {
    return min(lum(base) / 0.04, 1.0);
}

// The half-size picture's width and height.
fn half_dims() -> vec2<u32> {
    return vec2<u32>((F.dims.x + 1u) / 2u, (F.dims.y + 1u) / 2u);
}

// What of a pixel blooms: a light source as much as its light shows, else what's brighter than
// the preset's threshold; nothing outside the 3D scene.
fn bright(x: u32, y: u32) -> vec3<f32> {
    let i = min(y, F.dims.y - 1u) * F.dims.x + min(x, F.dims.x - 1u);
    if pix[i].pos.w < 0.5 {
        return vec3<f32>(0.0);
    }
    let o = i * WORDS;
    let rg = unpack2x16float(outp[o]);
    let bt = unpack2x16float(outp[o + 5u]);
    let c = vec3<f32>(rg.x, rg.y, bt.x) + glow_at(i);
    let start = F.bloom.y;
    let over = clamp((lum(c) - start) / max(1.0 - start, 0.05), 0.0, 1.0);
    // A light source blooms as much as its light shows (it fades in and out with it); a card
    // (a sprite: -2) not at all.
    let source = bitcast<f32>(pix[i].info.w);
    if source < -1.5 {
        return vec3<f32>(0.0);
    }
    return c * select(over, source, source >= 0.0);
}

@compute @workgroup_size(8, 8)
fn bloom_down(@builtin(global_invocation_id) g: vec3<u32>) {
    let hd = half_dims();
    if g.x >= hd.x || g.y >= hd.y {
        return;
    }
    let x = g.x * 2u;
    let y = g.y * 2u;
    let sum = bright(x, y) + bright(x + 1u, y) + bright(x, y + 1u) + bright(x + 1u, y + 1u);
    bloom_a[g.y * hd.x + g.x] = vec4<f32>(sum * 0.25, 0.0);
}

// ---- The light sources' own glow: their colour (as much as their light shows) spread two or
// three pixels and added, dark surroundings included ----

@compute @workgroup_size(8, 8)
fn glow_seed(@builtin(global_invocation_id) g: vec3<u32>) {
    if g.x >= F.dims.x || g.y >= F.dims.y {
        return;
    }
    let i = g.y * F.dims.x + g.x;
    let px = pix[i];
    let source = bitcast<f32>(px.info.w);
    var c = vec3<f32>(0.0);
    if px.pos.w > 0.5 && source >= 0.0 {
        let packed = px.info.x;
        c = vec3<f32>(
            to_linear((packed >> 16u) & 255u),
            to_linear((packed >> 8u) & 255u),
            to_linear(packed & 255u),
        ) * source;
    }
    glow_a[i] = vec4<f32>(c, 0.0);
}

const GLOW_RADIUS: i32 = 2;

@compute @workgroup_size(8, 8)
fn glow_across(@builtin(global_invocation_id) g: vec3<u32>) {
    let w = F.dims.x;
    if g.x >= w || g.y >= F.dims.y {
        return;
    }
    var s = vec4<f32>(0.0);
    for (var d = -GLOW_RADIUS; d <= GLOW_RADIUS; d++) {
        let x = u32(clamp(i32(g.x) + d, 0, i32(w) - 1));
        s += glow_a[g.y * w + x];
    }
    glow_b[g.y * w + g.x] = s / f32(2 * GLOW_RADIUS + 1);
}

@compute @workgroup_size(8, 8)
fn glow_down(@builtin(global_invocation_id) g: vec3<u32>) {
    let w = F.dims.x;
    let h = F.dims.y;
    if g.x >= w || g.y >= h {
        return;
    }
    var s = vec4<f32>(0.0);
    for (var d = -GLOW_RADIUS; d <= GLOW_RADIUS; d++) {
        let y = u32(clamp(i32(g.y) + d, 0, i32(h) - 1));
        s += glow_b[y * w + g.x];
    }
    glow_a[g.y * w + g.x] = s / f32(2 * GLOW_RADIUS + 1);
}

// The glow (times its strength) at pixel `i`.
fn glow_at(i: u32) -> vec3<f32> {
    if F.bloom.z <= 0.0 {
        return vec3<f32>(0.0);
    }
    return glow_a[i].xyz * F.bloom.z;
}

const BLOOM_RADIUS: i32 = 3;

// A box blur across (a to b), then down (b to a); run twice.
@compute @workgroup_size(8, 8)
fn bloom_across(@builtin(global_invocation_id) g: vec3<u32>) {
    let hd = half_dims();
    if g.x >= hd.x || g.y >= hd.y {
        return;
    }
    var s = vec4<f32>(0.0);
    for (var d = -BLOOM_RADIUS; d <= BLOOM_RADIUS; d++) {
        let x = u32(clamp(i32(g.x) + d, 0, i32(hd.x) - 1));
        s += bloom_a[g.y * hd.x + x];
    }
    bloom_b[g.y * hd.x + g.x] = s / f32(2 * BLOOM_RADIUS + 1);
}

@compute @workgroup_size(8, 8)
fn bloom_down_blur(@builtin(global_invocation_id) g: vec3<u32>) {
    let hd = half_dims();
    if g.x >= hd.x || g.y >= hd.y {
        return;
    }
    var s = vec4<f32>(0.0);
    for (var d = -BLOOM_RADIUS; d <= BLOOM_RADIUS; d++) {
        let y = u32(clamp(i32(g.y) + d, 0, i32(hd.y) - 1));
        s += bloom_b[y * hd.x + g.x];
    }
    bloom_a[g.y * hd.x + g.x] = s / f32(2 * BLOOM_RADIUS + 1);
}

// The bloom (times its strength) at a full-size pixel: the half-size picture, bilinear.
fn bloom_at(x: u32, y: u32) -> vec3<f32> {
    let hd = half_dims();
    let fx = clamp((f32(x) + 0.5) / 2.0 - 0.5, 0.0, f32(hd.x - 1u));
    let fy = clamp((f32(y) + 0.5) / 2.0 - 0.5, 0.0, f32(hd.y - 1u));
    let x0 = u32(fx);
    let y0 = u32(fy);
    let x1 = min(x0 + 1u, hd.x - 1u);
    let y1 = min(y0 + 1u, hd.y - 1u);
    let tx = fract(fx);
    let ty = fract(fy);
    let top = mix(bloom_a[y0 * hd.x + x0].xyz, bloom_a[y0 * hd.x + x1].xyz, tx);
    let bottom = mix(bloom_a[y1 * hd.x + x0].xyz, bloom_a[y1 * hd.x + x1].xyz, tx);
    return mix(top, bottom, ty) * F.bloom.x;
}

// The bloom (or the glow) as one word: the square root of each channel (fine steps where it's
// faint), ten bits each (composite.wgsl reads it back).
fn pack_bloom(b: vec3<f32>) -> u32 {
    let q = vec3<u32>(sqrt(clamp(b, vec3<f32>(0.0), vec3<f32>(1.0))) * 1023.0 + 0.5);
    return q.x | (q.y << 10u) | (q.z << 20u);
}

// The lit colour, the glow and the bloom added, the grade and the vignette: the pixel's colour;
// the bloom and the glow there for the player's picture.
@compute @workgroup_size(8, 8)
fn finish(@builtin(global_invocation_id) g: vec3<u32>) {
    let w = F.dims.x;
    let h = F.dims.y;
    if g.x >= w || g.y >= h {
        return;
    }
    let i = g.y * w + g.x;
    if pix[i].pos.w < 0.5 {
        return;
    }
    let o = i * WORDS;
    let rg = unpack2x16float(outp[o]);
    let bt = unpack2x16float(outp[o + 5u]);
    let gl = glow_at(i);
    var c = vec3<f32>(rg.x, rg.y, bt.x) + gl;
    var b = vec3<f32>(0.0);
    if F.bloom.x > 0.0 {
        b = bloom_at(g.x, g.y);
        c += b * bt.y;
    }
    let graded = grade(c, vignette(g.x, g.y));
    outp[o] = (to_byte(graded.x) << 16u) | (to_byte(graded.y) << 8u) | to_byte(graded.z);
    outp[o + 5u] = pack_bloom(b);
    outp[o + 6u] = pack_bloom(gl);
}
