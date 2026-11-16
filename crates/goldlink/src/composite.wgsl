// The player's picture put together with the world's on the window's GPU (UC14): melonDS's
// `SoftRenderer::Composite` (patch 0026), as the DS would draw the two in one picture, at each
// screen pixel round the player. Each colour it takes is lit as what it is: the world's by the
// world's light there, the player's by the player's (the lighting's light maps), then blended:
// lit after blending, the world round a moving player took the player's light (a fringe in the
// shadows, the user's look).

struct U {
    // xf = map.x + map.y * pixel x, yf = map.z + map.w * pixel y: the world's point (DS pixels).
    map: vec4<f32>,
    // The attached picture's point is the world's less `off` (x, y); 1 when lit (z), 1 when the
    // target is sRGB (w).
    off: vec4<f32>,
    // The shown picture's width, the plain mask's columns' offset, the arrays' (shown column +
    // from), the picture's left and right columns (edges outside aren't marked).
    dims: vec4<i32>,
    // The crop the arrays hold (array columns, rows: x0, y0, width, height).
    crop: vec4<i32>,
    // DISP3DCNT, the arrays' width (RenderWidth), 0, 0.
    misc: vec4<u32>,
    // The edge colours, two a word.
    edges: vec4<u32>,
    // Each plane's first word in `planes`.
    offsets: array<vec4<u32>, 5>,
    // The grade: tint (w: saturation), shadows' tint (w: exposure), highlights' (w: vignette).
    tint: vec4<f32>,
    shadow: vec4<f32>,
    high: vec4<f32>,
}

@group(0) @binding(0) var<uniform> u: U;
@group(0) @binding(1) var<storage, read> planes: array<u32>;

const WORLD_TOP: u32 = 0u;
const WORLD_TOP_DEPTH: u32 = 1u;
const WORLD_TOP_ATTR: u32 = 2u;
const WORLD_BOTTOM: u32 = 3u;
const WORLD_BOTTOM_DEPTH: u32 = 4u;
const WORLD_OPAQUE: u32 = 5u;
const ATT_TOP: u32 = 6u;
const ATT_TOP_DEPTH: u32 = 7u;
const ATT_TRANS_DEPTH: u32 = 8u;
const ATT_ATTR: u32 = 9u;
const ATT_BOTTOM: u32 = 10u;
const ATT_K: u32 = 11u;
const ATT_B: u32 = 12u;
const LATE_EARLY: u32 = 13u;
const LATE_DEPTH: u32 = 14u;
const LATE_K: u32 = 15u;
const LATE_B: u32 = 16u;
const LIGHT_WORLD: u32 = 17u;
const LIGHT_PLAYER: u32 = 18u;
const PLAIN: u32 = 19u;

fn plane_start(p: u32) -> u32 {
    return u.offsets[p / 4u][p % 4u];
}

// The cell of the crop at (x, y) (array columns, rows), clamped into it.
fn cell(x: i32, y: i32) -> u32 {
    let cx = clamp(x - u.crop.x, 0, u.crop.z - 1);
    let cy = clamp(y - u.crop.y, 0, u.crop.w - 1);
    return u32(cy * u.crop.z + cx);
}

fn word(p: u32, x: i32, y: i32) -> u32 {
    return planes[plane_start(p) + cell(x, y)];
}

fn fword(p: u32, x: i32, y: i32) -> f32 {
    return bitcast<f32>(word(p, x, y));
}

fn bword(p: u32, x: i32, y: i32, ch: u32) -> f32 {
    return bitcast<f32>(planes[plane_start(p) + cell(x, y) * 3u + ch]);
}

// ---- The lighting (goldlink-light's light.wgsl, the same arithmetic) ----

const CONTRAST: f32 = 0.10;

fn lum(c: vec3<f32>) -> f32 {
    return 0.2126 * c.x + 0.7152 * c.y + 0.0722 * c.z;
}

fn to_linear1(c: f32) -> f32 {
    if c <= 0.04045 {
        return c / 12.92;
    }
    return pow((c + 0.055) / 1.055, 2.4);
}

fn to_byte(v: f32) -> f32 {
    let l = floor(clamp(v, 0.0, 1.0) * 4095.0 + 0.5) / 4095.0;
    var s: f32;
    if l <= 0.0031308 {
        s = l * 12.92;
    } else {
        s = 1.055 * pow(l, 1.0 / 2.4) - 0.055;
    }
    let curve = s * s * (3.0 - 2.0 * s);
    s = s + CONTRAST * (curve - s);
    return floor(clamp(s, 0.0, 1.0) * 255.0 + 0.5);
}

fn grade(c0: vec3<f32>, v: f32) -> vec3<f32> {
    var c = c0 * u.tint.xyz * u.shadow.w;
    let l = lum(c);
    c = vec3<f32>(l) + (c - vec3<f32>(l)) * u.tint.w;
    let lift = min(l * 4.0, 1.0);
    c = c * mix(u.shadow.xyz, vec3<f32>(1.0), lift);
    let high = clamp((l - 0.35) / 0.65, 0.0, 1.0);
    c = c * mix(vec3<f32>(1.0), u.high.xyz, high);
    return max(c * v, vec3<f32>(0.0));
}

// The vignette at the world's pixel (the shown picture's column and row).
var<private> vig: f32 = 1.0;

fn vignette(x: f32, y: f32) -> f32 {
    let strength = u.high.w;
    if strength <= 0.0 {
        return 1.0;
    }
    let w = f32(u.dims.x);
    let h = 192.0;
    let dx = (x + 0.5 - w / 2.0) / (h * 0.75);
    let dy = (y + 0.5 - h / 2.0) / (h / 2.0);
    let t = clamp((dx * dx + dy * dy - 0.45) / 1.3, 0.0, 1.0);
    return 1.0 - strength * t * sqrt(t);
}

// Words a cell in a light plane: the world's light, the bloom and the glow there (four), the
// player's light (two).
fn light_words(p: u32) -> u32 {
    return select(2u, 4u, p == LIGHT_WORLD);
}

// A bloom or glow word (light.wgsl `pack_bloom`): linear light.
fn unpack_bloom(q: u32) -> vec3<f32> {
    let r = vec3<f32>(f32(q & 1023u), f32((q >> 10u) & 1023u), f32((q >> 20u) & 1023u)) / 1023.0;
    return r * r;
}

// A colour (0..255 a channel, as shown unlit) lit by the light at (x, y) of `p` (the world's or
// the player's map).
fn lit(c: vec3<f32>, p: u32, x: i32, y: i32) -> vec3<f32> {
    if u.off.z < 0.5 {
        return c;
    }
    let i = plane_start(p) + cell(x, y) * light_words(p);
    let rg = unpack2x16float(planes[i]);
    let bg = unpack2x16float(planes[i + 1u]);
    var light = vec3<f32>(rg.x, rg.y, bg.x);
    var glow = bg.y;
    let world = plane_start(LIGHT_WORLD) + cell(x, y) * 4u;
    if p == LIGHT_PLAYER && all(light == vec3<f32>(0.0)) {
        // Where the player wasn't at rest: the world's light there.
        let j = world;
        let rg2 = unpack2x16float(planes[j]);
        let bg2 = unpack2x16float(planes[j + 1u]);
        light = vec3<f32>(rg2.x, rg2.y, bg2.x);
        glow = 0.0;
    }
    let base = vec3<f32>(to_linear1(c.x / 255.0), to_linear1(c.y / 255.0), to_linear1(c.z / 255.0));
    var out = base;
    if glow < 1.0 {
        out = base * light * (1.0 - glow) + base * glow;
    }
    // The light sources' glow there; the bloom, as much as this colour takes (light.wgsl
    // `takes`).
    out += unpack_bloom(planes[world + 3u]);
    out += unpack_bloom(planes[world + 2u]) * min(lum(base) / 0.04, 1.0);
    let g = grade(out, vig);
    return vec3<f32>(to_byte(g.x), to_byte(g.y), to_byte(g.z));
}

// ---- The composite (GPU3D_Soft.cpp `Composite`) ----

// A colour: its channels (0..255, lit) and its alpha (0..31).
struct Col {
    rgb: vec3<f32>,
    a: u32,
}

// A DS colour (6 bits a channel, red lowest; alpha in bits 24..28) as shown: 8 bits a channel.
fn ds(c: u32) -> vec3<f32> {
    let r = c & 0x3Fu;
    let g = (c >> 8u) & 0x3Fu;
    let b = (c >> 16u) & 0x3Fu;
    return vec3<f32>(f32((r << 2u) | (r >> 4u)), f32((g << 2u) | (g >> 4u)), f32((b << 2u) | (b >> 4u)));
}

// A world's colour at (x, y), lit as the world; an attached one at (x, y), lit as the player.
fn world_col(c: u32, x: i32, y: i32) -> Col {
    return Col(lit(ds(c), LIGHT_WORLD, x, y), (c >> 24u) & 0x1Fu);
}

fn att_col(c: u32, x: i32, y: i32) -> Col {
    return Col(lit(ds(c), LIGHT_PLAYER, x, y), (c >> 24u) & 0x1Fu);
}

fn mixc(top: Col, bottom: Col, a: u32) -> vec3<f32> {
    let t = f32(a) / 32.0;
    return top.rgb * t + bottom.rgb * (1.0 - t);
}

// k * c + b (a stack of translucent pixels over what's behind them), `b` lit as its source.
fn apply(kp: u32, bp: u32, x: i32, y: i32, c: Col, light: u32) -> Col {
    let k = fword(kp, x, y);
    let b6 = vec3<f32>(bword(bp, x, y, 0u), bword(bp, x, y, 1u), bword(bp, x, y, 2u));
    var b = b6 * (255.0 / 63.0);
    let cover = 1.0 - k;
    if u.off.z > 0.5 && cover > 0.01 {
        b = lit(min(b / cover, vec3<f32>(255.0)), light, x, y) * cover;
    }
    return Col(clamp(c.rgb * k + b, vec3<f32>(0.0), vec3<f32>(255.0)), c.a);
}

fn in_att(x: i32, y: i32) -> bool {
    return x >= 0 && y >= 0 && x < i32(u.misc.y) && y < 192;
}

var<private> wx: i32;
var<private> wy: i32;
var<private> ax: i32;
var<private> ay: i32;
var<private> ddx: f32;
var<private> ddy: f32;

// The opaque pixel on top at the world's point (x, y), the attached picture's (qx, qy): its
// polygon ID and depth.
fn opaque_at(x: i32, y: i32, qx: i32, qy: i32) -> vec2<u32> {
    var id = (word(WORLD_TOP_ATTR, x, y) >> 24u) & 0x3Fu;
    var depth = word(WORLD_TOP_DEPTH, x, y) & 0xFFFFFFu;
    if in_att(qx, qy) && fword(ATT_K, qx, qy) == 0.0
        && (word(ATT_TOP_DEPTH, qx, qy) & 0xFFFFFFu) < depth {
        id = (word(ATT_ATTR, qx, qy) >> 24u) & 0x3Fu;
        depth = word(ATT_TOP_DEPTH, qx, qy) & 0xFFFFFFu;
    }
    return vec2<u32>(id, depth);
}

fn edge_colour(id: u32) -> vec3<f32> {
    let e = (u.edges[(id >> 3u) / 2u] >> (16u * ((id >> 3u) % 2u))) & 0xFFFFu;
    var r = (e << 1u) & 0x3Eu;
    if r != 0u { r += 1u; }
    var g = (e >> 4u) & 0x3Eu;
    if g != 0u { g += 1u; }
    var b = (e >> 9u) & 0x3Eu;
    if b != 0u { b += 1u; }
    return ds(r | (g << 8u) | (b << 16u));
}

// The DS's final pass on the top pixel: its edge marked, its edge blended with what's behind.
fn final_pass(colour0: Col, attr0: u32, depth: u32, bottom: Col, from_att: bool) -> Col {
    var colour = colour0;
    var attr = attr0;
    if (u.misc.x & (1u << 5u)) != 0u && (attr & 0xFu) != 0u {
        let id = (attr >> 24u) & 0x3Fu;
        var marked = false;
        for (var d = 0; d < 4 && !marked; d++) {
            var ox = 0;
            var oy = 0;
            if d == 0 { ox = -1; } else if d == 1 { ox = 1; } else if d == 2 { oy = -1; } else { oy = 1; }
            var nx: i32;
            var ny: i32;
            var qx: i32;
            var qy: i32;
            if from_att {
                qx = ax + ox;
                qy = ay + oy;
                nx = i32(floor(f32(qx) + 0.5 + ddx));
                ny = i32(floor(f32(qy) + 0.5 + ddy));
            } else {
                nx = wx + ox;
                ny = wy + oy;
                qx = i32(floor(f32(nx) + 0.5 - ddx));
                qy = i32(floor(f32(ny) + 0.5 - ddy));
            }
            if nx < u.dims.w >> 16 || nx >= (u.dims.w & 0xFFFF) || ny < 0 || ny > 191 {
                continue;
            }
            let n = opaque_at(nx, ny, qx, qy);
            marked = n.x != id && depth < n.y;
        }
        if marked {
            let e = edge_colour(id);
            if from_att {
                colour = Col(lit(e, LIGHT_PLAYER, ax, ay), colour.a);
            } else {
                colour = Col(lit(e, LIGHT_WORLD, wx, wy), colour.a);
            }
            attr = (attr & 0xFFFFE0FFu) | 0x00001000u;
        }
    }
    if (u.misc.x & (1u << 4u)) != 0u && (attr & 0xFu) != 0u {
        let coverage = (attr >> 8u) & 0x1Fu;
        if coverage == 0u {
            colour = bottom;
        } else if coverage != 0x1Fu {
            let top_a = colour.a;
            let bot_a = bottom.a;
            let a = (top_a * (coverage + 1u) + bot_a * (31u - coverage)) >> 5u;
            if bot_a != 0u {
                colour = Col(mixc(colour, bottom, coverage + 1u), a);
            } else {
                colour = Col(colour.rgb, a);
            }
        }
    }
    return colour;
}

fn composite() -> vec3<f32> {
    let att_in = in_att(ax, ay);
    let wd = word(WORLD_TOP_DEPTH, wx, wy) & 0xFFFFFFu;
    var k = 1.0;
    if att_in {
        k = fword(ATT_K, ax, ay);
    }
    let opaque = att_in && k == 0.0;
    var ad = 0xFFFFFFFFu;
    if opaque {
        ad = word(ATT_TOP_DEPTH, ax, ay) & 0xFFFFFFu;
    }
    var at = 0xFFFFFFFFu;
    if att_in && k < 1.0 {
        at = min(ad, word(ATT_TRANS_DEPTH, ax, ay) & 0xFFFFFFu);
    }
    let late = fword(LATE_K, wx, wy) < 1.0 || bword(LATE_B, wx, wy, 0u) != 0.0
        || bword(LATE_B, wx, wy, 1u) != 0.0 || bword(LATE_B, wx, wy, 2u) != 0.0;
    var out: Col;
    if opaque && ad < wd {
        let ab = word(ATT_BOTTOM, ax, ay);
        var bottom: Col;
        if (ab >> 24u) != 0u {
            bottom = att_col(ab, ax, ay);
        } else if late {
            bottom = world_col(word(LATE_EARLY, wx, wy), wx, wy);
        } else {
            bottom = world_col(word(WORLD_TOP, wx, wy), wx, wy);
        }
        out = final_pass(att_col(word(ATT_TOP, ax, ay), ax, ay), word(ATT_ATTR, ax, ay), ad, bottom, true);
        if late && (word(LATE_DEPTH, wx, wy) & 0xFFFFFFu) < ad {
            out = apply(LATE_K, LATE_B, wx, wy, out, LIGHT_WORLD);
        }
    } else {
        let front = att_in && k < 1.0 && at < wd;
        let late_over = front && late && (word(LATE_DEPTH, wx, wy) & 0xFFFFFFu) < at;
        var top: Col;
        if late_over {
            top = world_col(word(LATE_EARLY, wx, wy), wx, wy);
        } else {
            top = world_col(word(WORLD_TOP, wx, wy), wx, wy);
        }
        var bottom = world_col(word(WORLD_BOTTOM, wx, wy), wx, wy);
        if att_in && k < 1.0 && at > wd && at < (word(WORLD_BOTTOM_DEPTH, wx, wy) & 0xFFFFFFu) {
            if opaque {
                bottom = att_col(word(ATT_TOP, ax, ay), ax, ay);
            } else {
                bottom = apply(ATT_K, ATT_B, ax, ay, bottom, LIGHT_PLAYER);
            }
        }
        let attr = word(WORLD_TOP_ATTR, wx, wy);
        out = final_pass(top, attr, wd, bottom, false);
        if front && (attr & (1u << 22u)) != 0u {
            // Over the world's own translucent pixels (another shadow): the darker of the two, a
            // channel each (the user's choice).
            let opaque_world = world_col(word(WORLD_OPAQUE, wx, wy), wx, wy);
            let under = apply(ATT_K, ATT_B, ax, ay, final_pass(opaque_world, attr, wd, bottom, false), LIGHT_PLAYER);
            out = Col(min(under.rgb, out.rgb), out.a);
        } else if front {
            out = apply(ATT_K, ATT_B, ax, ay, out, LIGHT_PLAYER);
            if late_over {
                out = apply(LATE_K, LATE_B, wx, wy, out, LIGHT_WORLD);
            }
        }
    }
    return out.rgb;
}

@vertex
fn vs(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
    let x = f32((i << 1u) & 2u) * 2.0 - 1.0;
    let y = f32(i & 2u) * 2.0 - 1.0;
    return vec4<f32>(x, y, 0.0, 1.0);
}

@fragment
fn fs(@builtin(position) p: vec4<f32>) -> @location(0) vec4<f32> {
    let xf = u.map.x + u.map.y * p.x;
    let yf = u.map.z + u.map.w * p.y;
    if !(xf >= 0.0) || !(yf >= 0.0) || yf >= 192.0 || xf >= f32(u.dims.x) {
        discard;
    }
    let sx = i32(xf);
    let sy = i32(yf);
    // The shown column into the arrays' columns.
    wx = sx + u.dims.y + u.dims.z;
    wy = sy;
    if word(PLAIN, wx, wy) == 0u {
        discard;
    }
    ax = i32(floor(xf - u.off.x)) + u.dims.y + u.dims.z;
    ay = i32(floor(yf - u.off.y));
    ddx = u.off.x;
    ddy = u.off.y;
    vig = vignette(f32(sx), f32(sy));
    let c = composite() / 255.0;
    if u.off.w > 0.5 {
        return vec4<f32>(to_linear1(c.x), to_linear1(c.y), to_linear1(c.z), 1.0);
    }
    return vec4<f32>(c, 1.0);
}
