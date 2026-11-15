// GoldLink's cast shadows on the GPU (UC14): depth maps of the casters as a light sees them, and
// which card each pixel of the picture shows. The CPU places each triangle on its map (the sun's
// texels, a point light's face, the picture's pixels: shadow.rs `View::map`, `Tri`); this draws
// them, nearest the light winning, sprites through their texels' alpha.

struct In {
    // Where on the map: x, y in [-1, 1], the depth test's value in [0, 1], 1.
    @location(0) pos: vec4<f32>,
    // How near the light (the CPU's value, kept for the test).
    @location(1) raw: f32,
    // Texture coordinates (texels).
    @location(2) st: vec2<f32>,
    // The texture: where its alpha starts, width, height (0: none), the DS's parameters.
    @location(3) tex: vec4<u32>,
    // The card it belongs to (index + 1; 0 none).
    @location(4) card: u32,
}

struct Out {
    @builtin(position) pos: vec4<f32>,
    @location(0) @interpolate(linear) raw: f32,
    @location(1) @interpolate(linear) st: vec2<f32>,
    @location(2) @interpolate(flat) tex: vec4<u32>,
    @location(3) @interpolate(flat) card: u32,
}

// The textures' alpha (0..31 a texel), 4096 a row.
@group(0) @binding(0) var alpha: texture_2d<u32>;

@vertex
fn vs(v: In) -> Out {
    return Out(v.pos, v.raw, v.st, v.tex, v.card);
}

struct Target {
    @location(0) raw: f32,
    @location(1) card: u32,
}

// A texel coordinate wrapped as the DS does: repeat (bit `rep`), mirrored (bit `flip`), else
// clamped (shadow.rs `wrap`).
fn wrap(v: i32, size: i32, param: u32, rep: u32, flip: u32) -> i32 {
    if (param & (1u << rep)) != 0u {
        if (param & (1u << flip)) != 0u && (v & size) != 0 {
            return size - 1 - (v & (size - 1));
        }
        return v & (size - 1);
    }
    return clamp(v, 0, size - 1);
}

@fragment
fn fs(i: Out) -> Target {
    if i.tex.y != 0u {
        let w = i32(i.tex.y);
        let h = i32(i.tex.z);
        let s = wrap(i32(floor(i.st.x)), w, i.tex.w, 16u, 18u);
        let t = wrap(i32(floor(i.st.y)), h, i.tex.w, 17u, 19u);
        let k = i.tex.x + u32(t) * u32(w) + u32(s);
        let dims = textureDimensions(alpha);
        var a = 31u;
        if k / 4096u < dims.y {
            a = textureLoad(alpha, vec2<i32>(i32(k % 4096u), i32(k / 4096u)), 0).r;
        }
        if a < 16u {
            discard;
        }
    }
    return Target(i.raw, i.card);
}
