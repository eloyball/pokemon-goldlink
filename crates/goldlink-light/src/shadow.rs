//! Cast shadows (dev/docs/unified-client/UC13_SHADOWS.md): the sun's, from the polygons the DS
//! drew for the picture on screen.
//!
//! The polygons are placed in 3D with the projection (as the lighting places pixels) and drawn
//! into a depth map as the sun sees them (orthographic, along the sun's direction); a pixel is in
//! shadow when something lies between it and the sun there. Textured polygons keep their
//! transparent texels out, so a tree or a person casts their silhouette.
//!
//! People, Pokémon and trees are flat cards turned toward the camera: from the side the sun would
//! see a card edge-on. Each card casts as if it stood upright at its foot, turned toward the sun:
//! its sprite's silhouette laid on the ground, longer as the sun sinks. A card doesn't shadow
//! itself.

use super::{V3, add, cross, dot, normalize, scale, sub};

/// A polygon of the picture (goldlink-core `ShadowPoly`).
#[derive(Debug, Clone, Copy, Default)]
pub struct ShadowPoly {
    pub first_vert: u32,
    pub num_verts: u32,
    /// The DS's polygon attributes (alpha in bits 16-20).
    pub attr: u32,
    /// The DS's texture parameters (repeat and flip in bits 16-19).
    pub tex_param: u32,
    /// Index into the textures, -1: untextured or opaque.
    pub tex: i32,
    /// 1 attached, 2 translucent, 4 and 8 the DS's shadow volumes, 16 facing, 32 a line, 64 the
    /// game's own shadow.
    pub flags: u32,
}

/// A vertex: clip coordinates (20.12), texture coordinates (12.4 texels) and where it lands on
/// the picture (columns of the wide picture, rows).
#[derive(Debug, Clone, Copy, Default)]
pub struct ShadowVert {
    pub clip: [i32; 4],
    pub st: [i16; 2],
    pub screen: [i32; 2],
}

/// A texture's transparency: `width` x `height` bytes (0..31) from `offset` in the alpha.
#[derive(Debug, Clone, Copy, Default)]
pub struct ShadowTex {
    pub width: u32,
    pub height: u32,
    pub offset: u32,
}

/// The picture's polygons.
#[derive(Debug, Clone, Copy, Default)]
pub struct Geometry<'a> {
    pub polys: &'a [ShadowPoly],
    pub verts: &'a [ShadowVert],
    pub texs: &'a [ShadowTex],
    pub alpha: &'a [u8],
}

const FLAG_ATTACHED: u32 = 1;
/// Cast nothing: the DS's shadow volumes (4, 8), lines (32), the game's own shadows (64). See-through
/// polygons (alpha under 31) cast nothing either; ones drawn whole with an alpha texture (the
/// windmill's soft-edged blades, flag 2) cast through their texels that show.
const SKIP: u32 = 4 | 8 | 32 | 64;

/// The ground's normal when no polygons say (HeartGold's field camera).
const DEFAULT_UP: V3 = [0.0, 0.66, 0.75];

/// Shadow map texels per view unit (a DS pixel across is one unit).
const TEXELS: f32 = 2.0;
/// At most this many texels a side.
const MAX_SIDE: usize = 1536;
/// How far a pixel is pushed off its surface (along its normal, then toward the sun) before the
/// test, so a surface doesn't shadow itself.
const NORMAL_OFFSET: f32 = 1.0;
const BIAS: f32 = 1.0;
/// How far a standing card goes into the ground when it casts (view units): the test's margin.
const SINK: f32 = 2.5;

/// Where the sun is at `hours` (0..24, the DS clock): its azimuth (from north toward east), its
/// elevation, and how strong its shadows are (0 at night). It crosses the northern sky, so every
/// shadow leans toward the camera and shows below what casts it (the user, 2026-11-14): it rises
/// a little north of east at 5, stands in the north at 12:30 at 58 degrees, sets a little north
/// of west at 20. Shadows fade out as it nears the horizon, and it's never lower than 20 degrees
/// (longer shadows reach past what the DS draws, and pop in at the picture's edge).
pub fn sun(hours: f32) -> Option<(f32, f32, f32)> {
    let (rise, set) = (5.0f32, 20.0f32);
    let t = (hours.rem_euclid(24.0) - rise) / (set - rise);
    if !(0.0..=1.0).contains(&t) {
        return None;
    }
    let arc = std::f32::consts::PI * t;
    let elevation = 58f32.to_radians() * arc.sin();
    let strength = (elevation.to_degrees() / 10.0).clamp(0.0, 1.0);
    if strength <= 0.0 {
        return None;
    }
    // From north toward east: 80 degrees at sunrise, 0 at noon, -80 at sunset.
    let azimuth = 80f32.to_radians() * arc.cos();
    Some((azimuth, elevation.max(20f32.to_radians()), strength))
}

/// A card's frame: casting, it stands at `foot` turned toward the sun.
#[derive(Debug, Clone, Copy)]
struct Card {
    /// On the card's plane: a point, its normal, across and up the card (unit).
    origin: V3,
    normal: V3,
    across: V3,
    up: V3,
    /// Its extent along `across` and `up` from `origin`.
    a: [f32; 2],
    b: [f32; 2],
    /// Where it stands on the ground, and how much sun reaches there: the whole card is lit as
    /// its foot (another card's shadow across a flat sprite cut it in blocks: the user's look,
    /// 2026-11-14).
    foot: V3,
    sun: f32,
}

impl Card {
    /// Whether `p` lies on this card (within a unit or two).
    fn holds(&self, p: V3) -> bool {
        let d = sub(p, self.origin);
        let (a, b) = (dot(d, self.across), dot(d, self.up));
        dot(d, self.normal).abs() < 2.5
            && a > self.a[0] - 1.5
            && a < self.a[1] + 1.5
            && b > self.b[0] - 1.5
            && b < self.b[1] + 1.5
    }
}

/// The sun's depth map of the picture's polygons.
#[derive(Default)]
pub struct ShadowMap {
    /// Toward the sun, and the map's axes (view space).
    dir: V3,
    u: V3,
    v: V3,
    origin: [f32; 2],
    side: [usize; 2],
    texels: f32,
    /// Per texel: the nearest to the sun (largest along `dir`), and its card (index + 1; 0 none).
    depth: Vec<f32>,
    card: Vec<u32>,
    cards: Vec<Card>,
    /// Which cards are attached (drawn with what the camera follows).
    attached: Vec<bool>,
    /// Per pixel of the picture (`PICTURE_WIDTH` x 192): the card it shows (index + 1; 0 none).
    card_px: Vec<u32>,
    /// The ground's normal found in the last picture.
    pub up: V3,
    pub ready: bool,
    /// The projection and its inverse (rows, vectors on the left).
    proj: [[f32; 4]; 4],
    rows: [[f32; 4]; 4],
    /// Where the polygons truly land: column = fit[0] + fit[1] * x, row = fit[2] + fit[3] * y (x, y
    /// normalised device coordinates). The steady overworld's viewport isn't the screen's (it's
    /// 4/3 as wide), so the lighting's pixels ([`super::unproject`]) are placed on another scale
    /// across than the polygons: [`ShadowMap::light`] moves them onto the polygons'.
    fit: [f32; 4],
}

/// Least squares `y = a + b x` over `(x, y)` sums; None when the xs don't spread.
fn line_fit(n: f64, sx: f64, sy: f64, sxx: f64, sxy: f64) -> Option<[f32; 2]> {
    let det = n * sxx - sx * sx;
    if n < 2.0 || det.abs() < 1e-9 * n * n {
        return None;
    }
    let b = (n * sxy - sx * sy) / det;
    Some([((sy - b * sx) / n) as f32, b as f32])
}

/// A polygon's corners in view space with texture coordinates (texels).
struct Corner {
    p: V3,
    st: [f32; 2],
    /// Where it lands on the picture (its columns and rows).
    scr: [f32; 2],
}

impl ShadowMap {
    /// The direction toward the sun in view space (x east on screen, `up` the ground's normal;
    /// `azimuth` from north toward east).
    pub fn sun_dir(up: V3, azimuth: f32, elevation: f32) -> V3 {
        let east = [1.0, 0.0, 0.0];
        let north = normalize(cross(up, east));
        let flat = add(scale(east, azimuth.sin()), scale(north, azimuth.cos()));
        normalize(add(
            scale(flat, elevation.cos()),
            scale(up, elevation.sin()),
        ))
    }

    /// The ground's normal: the facing (area-weighted) most of the untextured polygons share
    /// whose normal leans up the screen; the default when there's none.
    pub fn find_up(g: &Geometry, rows: &[[f32; 4]; 4]) -> V3 {
        let mut bins: Vec<(V3, f32)> = Vec::new();
        let mut corners = Vec::new();
        for p in g.polys {
            if p.flags & SKIP != 0 || p.tex >= 0 {
                continue;
            }
            if !place(g, p, rows, &mut corners) {
                continue;
            }
            let n = area_normal(&corners);
            let area = dot(n, n).sqrt();
            if area < 1e-3 {
                continue;
            }
            let n = scale(n, 1.0 / area);
            if n[1] < 0.3 || n[0].abs() > 0.05 {
                continue;
            }
            match bins.iter_mut().find(|(m, _)| dot(*m, n) > 0.998) {
                Some(b) => b.1 += area,
                None => bins.push((n, area)),
            }
        }
        bins.iter()
            .max_by(|a, b| a.1.total_cmp(&b.1))
            .map_or(DEFAULT_UP, |b| normalize([0.0, b.0[1], b.0[2]]))
    }

    /// Draws the polygons into the map as the sun (toward `dir`) sees them.
    pub fn build(
        &mut self,
        g: &Geometry,
        proj: &[[f32; 4]; 4],
        rows: &[[f32; 4]; 4],
        up: V3,
        dir: V3,
        pool: Option<&rayon::ThreadPool>,
        seen: &dyn Fn(f32, f32) -> Option<V3>,
    ) {
        self.ready = false;
        self.up = up;
        self.dir = dir;
        self.proj = *proj;
        self.rows = *rows;
        // Where the vertices land against their device coordinates.
        let mut s = [[0f64; 5]; 2];
        for v in g.verts {
            if v.clip[3] == 0 {
                continue;
            }
            let w = v.clip[3] as f64;
            for (k, sum) in s.iter_mut().enumerate() {
                let (x, y) = (v.clip[k] as f64 / w, v.screen[k] as f64);
                sum[0] += 1.0;
                sum[1] += x;
                sum[2] += y;
                sum[3] += x * x;
                sum[4] += x * y;
            }
        }
        let fit = |k: usize, default: [f32; 2]| {
            line_fit(s[k][0], s[k][1], s[k][2], s[k][3], s[k][4]).unwrap_or(default)
        };
        let (fx, fy) = (fit(0, [191.5, 128.0]), fit(1, [95.5, -96.0]));
        self.fit = [fx[0], fx[1], fy[0], fy[1]];
        self.u = normalize(cross(up, dir));
        if dot(self.u, self.u) < 0.5 {
            return;
        }
        self.v = cross(dir, self.u);
        self.cards.clear();
        self.attached.clear();

        // Every caster's corners (cards turned toward the sun), and their extent on the map.
        let flat_sun = normalize(sub(dir, scale(up, dot(dir, up))));
        let turned_across = normalize(cross(up, flat_sun));
        let mut polys: Vec<(Vec<Corner>, i32, u32, u32)> = Vec::new();
        let mut shapes: Vec<(Vec<Corner>, Option<ShadowTex>, u32)> = Vec::new();
        let (mut lo, mut hi) = ([f32::MAX; 2], [f32::MIN; 2]);
        // Every opaque polygon in 3D with its normal.
        let mut placed: Vec<(Vec<Corner>, V3, &ShadowPoly)> = Vec::new();
        for p in g.polys {
            if p.flags & SKIP != 0 || (p.attr >> 16) & 0x1F != 31 {
                continue;
            }
            let mut corners = Vec::new();
            if !place(g, p, rows, &mut corners) {
                continue;
            }
            let n = area_normal(&corners);
            let area = dot(n, n).sqrt();
            if area < 1e-3 {
                continue;
            }
            placed.push((corners, scale(n, 1.0 / area), p));
        }
        // The camera's rays: along -z (an orthographic camera, as the steady overworld's), else
        // from the eye.
        let ortho = proj[2][3].abs() < 1e-6;
        // People are drawn nearer the camera than where they stand (so they show over the
        // ground): their foot goes back along the camera's ray onto the ground behind it.
        let ground = |foot: V3| -> V3 {
            let d = if ortho {
                [0.0, 0.0, -1.0]
            } else {
                normalize(foot)
            };
            let mut best: Option<f32> = None;
            for (c, n, p) in &placed {
                // Flat ground only (a roof behind a person isn't where they stand).
                if p.tex >= 0 || dot(*n, up) < 0.97 {
                    continue;
                }
                let dn = dot(d, *n);
                if dn.abs() < 1e-4 {
                    continue;
                }
                let t = dot(sub(c[0].p, foot), *n) / dn;
                if t < -2.0 || best.is_some_and(|b| b <= t) {
                    continue;
                }
                let hit = add(foot, scale(d, t));
                if inside(c, *n, hit) {
                    best = Some(t);
                }
            }
            best.map_or(foot, |t| add(foot, scale(d, t)))
        };
        for (mut corners, n, p) in placed.iter().map(|(c, n, p)| {
            (
                c.iter()
                    .map(|c| Corner {
                        p: c.p,
                        st: c.st,
                        scr: c.scr,
                    })
                    .collect::<Vec<_>>(),
                *n,
                *p,
            )
        }) {
            let mut card_id = 0;
            if p.tex >= 0 {
                let along_up = dot(n, up);
                // A decal lying on the ground (flowers, grass): casts nothing.
                if along_up.abs() > 0.9 {
                    continue;
                }
                // A card turned toward the camera: stood up at its foot, turned to the sun.
                if n[2] > 0.9 {
                    let across = normalize(cross(up, n));
                    let card_up = normalize(cross(n, across));
                    let o = corners[0].p;
                    let (mut a, mut b) = ([f32::MAX, f32::MIN], [f32::MAX, f32::MIN]);
                    for c in &corners {
                        let d = sub(c.p, o);
                        let (ca, cb) = (dot(d, across), dot(d, card_up));
                        a = [a[0].min(ca), a[1].max(ca)];
                        b = [b[0].min(cb), b[1].max(cb)];
                    }
                    // Where the sprite's feet are: its lowest row of texels that show (a sprite has
                    // empty rows under its feet), up from the card's bottom edge.
                    let tex = (p.tex >= 0)
                        .then(|| g.texs.get(p.tex as usize))
                        .flatten()
                        .copied();
                    let lift = tex.map_or(0.0, |t| {
                        sprite_lift(&corners, o, card_up, b, t, p.tex_param, g.alpha)
                    });
                    let b = [b[0] + lift, b[1]];
                    // The foot: the middle of the feet, on the ground there. The ground is what
                    // the picture shows just under them (the depth behind the sprite's empty
                    // texels); else along the camera's ray onto the polygons behind.
                    let bottom = add(
                        o,
                        add(scale(across, (a[0] + a[1]) / 2.0), scale(card_up, b[0])),
                    );
                    let at = screen_at(&corners, o, across, card_up, (a[0] + a[1]) / 2.0, b[0]);
                    // The first two rows of ground under the feet, and back half a row from the
                    // first: the ground at the feet's bottom edge (the first row is a pixel
                    // nearer the camera than where they stand).
                    let below: Vec<V3> = (0..5)
                        .filter_map(|r| seen(at[0], at[1] + r as f32))
                        .filter_map(|q| self.true_place(q))
                        .filter(|q| dot(sub(*q, bottom), n) < -2.0)
                        .take(2)
                        .collect();
                    let foot = match below[..] {
                        [p1, p2] if dot(sub(p1, p2), sub(p1, p2)) < 16.0 => {
                            add(p1, scale(sub(p1, p2), 0.5))
                        }
                        [p1, ..] => p1,
                        [] => ground(bottom),
                    };
                    let card = Card {
                        origin: o,
                        normal: n,
                        across,
                        up: card_up,
                        a,
                        b,
                        foot,
                        sun: 1.0,
                    };
                    // Where it shows on the picture, for the pixels it covers.
                    let tex = (p.tex >= 0)
                        .then(|| g.texs.get(p.tex as usize))
                        .flatten()
                        .copied();
                    shapes.push((
                        corners
                            .iter()
                            .map(|c| Corner {
                                p: c.p,
                                st: c.st,
                                scr: c.scr,
                            })
                            .collect::<Vec<_>>(),
                        tex,
                        p.tex_param,
                    ));
                    for c in corners.iter_mut() {
                        let d = sub(c.p, o);
                        let ca = dot(d, across) - (a[0] + a[1]) / 2.0;
                        let cb = dot(d, card_up) - b[0];
                        // Sunk into the ground by the test's margin (a surface's own offset and
                        // bias), so the shadow starts where the feet touch it: by the margin it
                        // started a pixel or two short (the user's look).
                        c.p = add(foot, add(scale(turned_across, ca), scale(up, cb - SINK)));
                    }
                    self.cards.push(card);
                    self.attached.push(p.flags & FLAG_ATTACHED != 0);
                    card_id = self.cards.len() as u32;
                }
            }
            for c in &corners {
                let m = [dot(c.p, self.u), dot(c.p, self.v)];
                for k in 0..2 {
                    lo[k] = lo[k].min(m[k]);
                    hi[k] = hi[k].max(m[k]);
                }
            }
            polys.push((corners, p.tex, p.tex_param, card_id));
        }
        if polys.is_empty() {
            return;
        }
        let extent = (hi[0] - lo[0]).max(hi[1] - lo[1]).max(1.0);
        self.texels = TEXELS.min(MAX_SIDE as f32 / extent);
        // The map's origin on whole texels of view space, so a still scene maps the same.
        self.origin = [
            (lo[0] * self.texels).floor() / self.texels,
            (lo[1] * self.texels).floor() / self.texels,
        ];
        self.side = [
            (((hi[0] - self.origin[0]) * self.texels).ceil() as usize + 1).min(MAX_SIDE),
            (((hi[1] - self.origin[1]) * self.texels).ceil() as usize + 1).min(MAX_SIDE),
        ];
        let n = self.side[0] * self.side[1];
        self.depth.clear();
        self.depth.resize(n, f32::MIN);
        self.card.clear();
        self.card.resize(n, 0);
        // Each polygon's triangles on the map, then drawn in bands of rows on the pool's threads.
        let mut tris = Vec::new();
        for (corners, tex, param, card) in &polys {
            let tex = (*tex >= 0)
                .then(|| g.texs.get(*tex as usize))
                .flatten()
                .copied();
            let m: Vec<[f32; 3]> = corners
                .iter()
                .map(|c| {
                    [
                        (dot(c.p, self.u) - self.origin[0]) * self.texels,
                        (dot(c.p, self.v) - self.origin[1]) * self.texels,
                        dot(c.p, self.dir),
                    ]
                })
                .collect();
            for k in 1..corners.len().saturating_sub(1) {
                let t = Tri {
                    m: [m[0], m[k], m[k + 1]],
                    st: [corners[0].st, corners[k].st, corners[k + 1].st],
                    tex,
                    param: *param,
                    card: *card,
                };
                let area = t.area();
                if area.abs() < 1e-6 {
                    continue;
                }
                let y0 =
                    t.m.iter()
                        .map(|p| p[1])
                        .fold(f32::MAX, f32::min)
                        .floor()
                        .max(0.0);
                let y1 = t.m.iter().map(|p| p[1]).fold(f32::MIN, f32::max).ceil();
                tris.push((y0 as usize, y1 as usize, t));
            }
        }
        let width = self.side[0];
        let alpha = g.alpha;
        let draw = |band: usize, depth: &mut [f32], card: &mut [u32]| {
            let (r0, r1) = (band * BAND, band * BAND + depth.len() / width);
            for (y0, y1, t) in &tris {
                if *y1 <= r0 || *y0 >= r1 {
                    continue;
                }
                t.draw((*y0).max(r0), (*y1).min(r1), r0, width, depth, card, alpha);
            }
        };
        let bands = self
            .depth
            .chunks_mut(BAND * width)
            .zip(self.card.chunks_mut(BAND * width))
            .enumerate();
        match pool {
            Some(pool) => {
                use rayon::prelude::*;
                let mut bands: Vec<_> = bands.collect();
                pool.install(|| bands.par_iter_mut().for_each(|(b, (d, c))| draw(*b, d, c)));
            }
            None => bands.for_each(|(b, (d, c))| draw(b, d, c)),
        }
        self.ready = true;
        // Each card's light, itself left out: a person's, their foot's; a tree's (a card taller
        // than a person), the mean along its middle as it stands toward the sun (by its foot alone
        // a forest came out patchy, a tree lit through a gap between trunks: the user's look).
        for k in 0..self.cards.len() {
            let c = self.cards[k];
            let height = c.b[1] - c.b[0];
            self.cards[k].sun = if height > TREE_HEIGHT {
                let at = |f: f32| add(c.foot, scale(up, height * f));
                [0.3, 0.5, 0.7]
                    .iter()
                    .map(|&f| self.visible(at(f), flat_sun, true, Some(k)))
                    .sum::<f32>()
                    / 3.0
            } else {
                self.visible(add(c.foot, scale(up, 0.5)), up, true, Some(k))
            };
        }
        // Which card each pixel of the picture shows (the nearest, its transparent texels left
        // out).
        self.card_px.clear();
        self.card_px.resize(PICTURE_WIDTH * PICTURE_HEIGHT, 0);
        let mut near = vec![f32::MIN; PICTURE_WIDTH * PICTURE_HEIGHT];
        for (k, (c, tex, param)) in shapes.iter().enumerate() {
            let z = c.iter().map(|c| c.p[2]).sum::<f32>() / c.len() as f32;
            for j in 1..c.len().saturating_sub(1) {
                let t = Tri {
                    m: [&c[0], &c[j], &c[j + 1]].map(|c| [c.scr[0], c.scr[1], z]),
                    st: [c[0].st, c[j].st, c[j + 1].st],
                    tex: *tex,
                    param: *param,
                    card: k as u32 + 1,
                };
                if t.area().abs() < 1e-6 {
                    continue;
                }
                t.draw(
                    0,
                    PICTURE_HEIGHT,
                    0,
                    PICTURE_WIDTH,
                    &mut near,
                    &mut self.card_px,
                    alpha,
                );
            }
        }
    }

    /// How much of the sun reaches `p` (view space as the lighting places it, at the picture's
    /// column `col` and row `row`; its surface's normal `n`): 0 or 1. With `attached` false (the
    /// player's own pixels), what the camera follows casts nothing.
    pub fn light(&self, p: V3, n: V3, attached: bool, col: usize, row: usize) -> f32 {
        if !self.ready {
            return 1.0;
        }
        let Some(p) = self.true_place(p) else {
            return 1.0;
        };
        // A card's pixels: lit as its foot.
        if col < PICTURE_WIDTH
            && let Some(&k) = self.card_px.get(row * PICTURE_WIDTH + col)
            && k > 0
        {
            let c = &self.cards[k as usize - 1];
            if (!attached && self.attached[k as usize - 1]) || c.holds(p) {
                return c.sun;
            }
        }
        self.visible(p, n, attached, None)
    }

    /// Whether the sun reaches `p` (where the polygons put it), `card` left out.
    fn visible(&self, p: V3, n: V3, attached: bool, card: Option<usize>) -> f32 {
        // Pushed off the surface further the more it turns from the sun (a wall the sun grazes
        // changes depth fast across a texel).
        let facing = dot(n, self.dir).clamp(0.05, 1.0);
        let slope = ((1.0 - facing * facing).sqrt() / facing).min(6.0);
        let q = add(p, scale(n, NORMAL_OFFSET * (1.0 + slope)));
        let x = (dot(q, self.u) - self.origin[0]) * self.texels;
        let y = (dot(q, self.v) - self.origin[1]) * self.texels;
        if x < 0.0 || y < 0.0 {
            return 1.0;
        }
        let (x, y) = (x as usize, y as usize);
        if x >= self.side[0] || y >= self.side[1] {
            return 1.0;
        }
        let i = y * self.side[0] + x;
        if self.depth[i] <= dot(q, self.dir) + BIAS * (1.0 + slope / self.texels) {
            return 1.0;
        }
        match self.card[i] {
            0 => 0.0,
            k => {
                let k = k as usize - 1;
                if card == Some(k) || self.cards[k].holds(p) || (!attached && self.attached[k]) {
                    1.0
                } else {
                    0.0
                }
            }
        }
    }
}

impl ShadowMap {
    /// A pixel's place as the lighting found it (its column `128 x + 191.5` of the wide picture),
    /// moved to where the polygons put that pixel ([`ShadowMap::fit`]).
    fn true_place(&self, p: V3) -> Option<V3> {
        let m = &self.proj;
        let mut c = [0f32; 4];
        for (k, out) in c.iter_mut().enumerate() {
            *out = p[0] * m[0][k] + p[1] * m[1][k] + p[2] * m[2][k] + m[3][k];
        }
        if c[3].abs() < 1e-9 || self.fit[1].abs() < 1e-3 || self.fit[3].abs() < 1e-3 {
            return None;
        }
        let (xn, yn, zn) = (c[0] / c[3], c[1] / c[3], c[2] / c[3]);
        let (col, row) = (128.0 * xn + 191.5, 95.5 - 96.0 * yn);
        let xn = (col - self.fit[0]) / self.fit[1];
        let yn = (row - self.fit[2]) / self.fit[3];
        super::unproject(&self.rows, xn, yn, zn)
    }
}

/// Cards taller than this (view units) are scenery (trees): lit by their middle, not their foot.
const TREE_HEIGHT: f32 = 44.0;

/// The lighting's picture: the wide picture's columns, the screen's rows.
const PICTURE_WIDTH: usize = 384;
const PICTURE_HEIGHT: usize = 192;

/// Rows of the map a thread draws at a time.
const BAND: usize = 16;

/// A triangle on the map: each corner's column, row (texels) and depth toward the sun, and its
/// texture coordinates.
struct Tri {
    m: [[f32; 3]; 3],
    st: [[f32; 2]; 3],
    tex: Option<ShadowTex>,
    param: u32,
    card: u32,
}

impl Tri {
    fn area(&self) -> f32 {
        let m = &self.m;
        (m[1][0] - m[0][0]) * (m[2][1] - m[0][1]) - (m[2][0] - m[0][0]) * (m[1][1] - m[0][1])
    }

    /// Rows `y0..y1` of the map into a band starting at row `r0` (`width` texels a row).
    #[allow(clippy::too_many_arguments)]
    fn draw(
        &self,
        y0: usize,
        y1: usize,
        r0: usize,
        width: usize,
        depth: &mut [f32],
        card: &mut [u32],
        alpha: &[u8],
    ) {
        let (m, c) = (&self.m, &self.st);
        let inv = 1.0 / self.area();
        let x0 = m
            .iter()
            .map(|p| p[0])
            .fold(f32::MAX, f32::min)
            .floor()
            .max(0.0) as usize;
        let x1 = (m.iter().map(|p| p[0]).fold(f32::MIN, f32::max).ceil() as usize).min(width);
        for y in y0..y1 {
            let py = y as f32 + 0.5;
            let row = (y - r0) * width;
            for x in x0..x1 {
                let px = x as f32 + 0.5;
                let w0 = ((m[1][0] - px) * (m[2][1] - py) - (m[2][0] - px) * (m[1][1] - py)) * inv;
                let w1 = ((m[2][0] - px) * (m[0][1] - py) - (m[0][0] - px) * (m[2][1] - py)) * inv;
                let w2 = 1.0 - w0 - w1;
                if w0 < 0.0 || w1 < 0.0 || w2 < 0.0 {
                    continue;
                }
                let d = w0 * m[0][2] + w1 * m[1][2] + w2 * m[2][2];
                let i = row + x;
                if d <= depth[i] {
                    continue;
                }
                if let Some(t) = self.tex {
                    let s = w0 * c[0][0] + w1 * c[1][0] + w2 * c[2][0];
                    let tt = w0 * c[0][1] + w1 * c[1][1] + w2 * c[2][1];
                    let s = wrap(s.floor() as i32, t.width as i32, self.param, 16, 18);
                    let tt = wrap(tt.floor() as i32, t.height as i32, self.param, 17, 19);
                    let a = alpha
                        .get(t.offset as usize + tt as usize * t.width as usize + s as usize)
                        .copied()
                        .unwrap_or(31);
                    if a < 16 {
                        continue;
                    }
                }
                depth[i] = d;
                card[i] = self.card;
            }
        }
    }
}

/// A texel coordinate wrapped as the DS does: repeat (bit `rep`), mirrored (bit `flip`), else
/// clamped.
fn wrap(v: i32, size: i32, param: u32, rep: u32, flip: u32) -> i32 {
    if param & (1 << rep) != 0 {
        if param & (1 << flip) != 0 && v & size != 0 {
            size - 1 - (v & (size - 1))
        } else {
            v & (size - 1)
        }
    } else {
        v.clamp(0, size - 1)
    }
}

/// `p`'s corners in view space into `out`; false when one isn't placed by this projection (a
/// 2D overlay drawn with its own).
fn place(g: &Geometry, p: &ShadowPoly, rows: &[[f32; 4]; 4], out: &mut Vec<Corner>) -> bool {
    out.clear();
    let (a, b) = (p.first_vert as usize, (p.first_vert + p.num_verts) as usize);
    let Some(verts) = g.verts.get(a..b) else {
        return false;
    };
    for v in verts {
        let c = v.clip.map(|c| c as f32 / 4096.0);
        let mut r = [0f32; 4];
        for (k, out) in r.iter_mut().enumerate() {
            *out = c[0] * rows[0][k] + c[1] * rows[1][k] + c[2] * rows[2][k] + c[3] * rows[3][k];
        }
        // The frame's projection gives w = 1 (an orthographic camera) or the depth; another
        // projection's polygons come out elsewhere.
        if r[3].abs() < 1e-6 {
            return false;
        }
        let p = [r[0] / r[3], r[1] / r[3], r[2] / r[3]];
        if !p.iter().all(|v| v.is_finite()) || p[2] >= 0.0 {
            return false;
        }
        out.push(Corner {
            p,
            st: [v.st[0] as f32 / 16.0, v.st[1] as f32 / 16.0],
            scr: [v.screen[0] as f32, v.screen[1] as f32],
        });
    }
    out.len() >= 3
}

/// Whether `p` (on the polygon's plane) lies within the convex polygon `c` (normal `n`), a unit
/// of slack.
fn inside(c: &[Corner], n: V3, p: V3) -> bool {
    // The distance to each edge's line, signed by its side: all one side (either winding).
    let (mut pos, mut neg) = (true, true);
    for k in 0..c.len() {
        let (a, b) = (c[k].p, c[(k + 1) % c.len()].p);
        let edge = sub(b, a);
        let len = dot(edge, edge).sqrt().max(1e-6);
        let d = dot(cross(edge, sub(p, a)), n) / len;
        pos &= d > -1.0;
        neg &= d < 1.0;
    }
    pos || neg
}

/// How far up a card (along `up`, from its bottom edge `b[0]`) its sprite's lowest row of texels
/// that show is: the texture rows read from the bottom edge's texture coordinates toward the top's.
fn sprite_lift(
    c: &[Corner],
    o: V3,
    up: V3,
    b: [f32; 2],
    t: ShadowTex,
    param: u32,
    alpha: &[u8],
) -> f32 {
    let height = b[1] - b[0];
    if height <= 0.0 {
        return 0.0;
    }
    // The texture coordinates at the bottom and the top edge, and across.
    let (mut bot, mut top, mut nb, mut nt) = ([0f32; 2], [0f32; 2], 0.0, 0.0);
    let (mut s0, mut s1) = (f32::MAX, f32::MIN);
    for k in c {
        let h = dot(sub(k.p, o), up);
        if h - b[0] < 0.5 {
            bot = [bot[0] + k.st[0], bot[1] + k.st[1]];
            nb += 1.0;
        } else if b[1] - h < 0.5 {
            top = [top[0] + k.st[0], top[1] + k.st[1]];
            nt += 1.0;
        }
        s0 = s0.min(k.st[0]);
        s1 = s1.max(k.st[0]);
    }
    if nb == 0.0 || nt == 0.0 {
        return 0.0;
    }
    let (tb, tt) = (bot[1] / nb, top[1] / nt);
    let rows = (tt - tb).abs().round() as i32;
    let step = if tt > tb { 1 } else { -1 };
    for r in 0..rows.max(0) {
        let tex_row = (tb + step as f32 * (r as f32 + 0.5)).floor() as i32;
        let tr = wrap(tex_row, t.height as i32, param, 17, 19);
        let shows = (s0.floor() as i32..s1.ceil() as i32).any(|s| {
            let s = wrap(s, t.width as i32, param, 16, 18);
            alpha
                .get(t.offset as usize + tr as usize * t.width as usize + s as usize)
                .is_some_and(|&a| a >= 16)
        });
        if shows {
            return height * r as f32 / rows as f32;
        }
    }
    0.0
}

/// Where a point of a card (`a` across, `b` up from `o`) lands on the picture, from its corners'
/// places there.
fn screen_at(c: &[Corner], o: V3, across: V3, up: V3, a: f32, b: f32) -> [f32; 2] {
    // The card's corners on its own axes and on the picture: a plane's affine map, fitted from
    // three corners that span it.
    let ab: Vec<[f32; 2]> = c
        .iter()
        .map(|k| {
            let d = sub(k.p, o);
            [dot(d, across), dot(d, up)]
        })
        .collect();
    for i in 0..c.len() {
        for j in i + 1..c.len() {
            for k in j + 1..c.len() {
                let (p, q, r) = (ab[i], ab[j], ab[k]);
                let det = (q[0] - p[0]) * (r[1] - p[1]) - (r[0] - p[0]) * (q[1] - p[1]);
                if det.abs() < 1.0 {
                    continue;
                }
                let u = ((a - p[0]) * (r[1] - p[1]) - (r[0] - p[0]) * (b - p[1])) / det;
                let v = ((q[0] - p[0]) * (b - p[1]) - (a - p[0]) * (q[1] - p[1])) / det;
                let (sp, sq, sr) = (c[i].scr, c[j].scr, c[k].scr);
                return [
                    sp[0] + u * (sq[0] - sp[0]) + v * (sr[0] - sp[0]),
                    sp[1] + u * (sq[1] - sp[1]) + v * (sr[1] - sp[1]),
                ];
            }
        }
    }
    c[0].scr
}

/// The polygon's normal times twice its area (toward the camera for a front face).
fn area_normal(c: &[Corner]) -> V3 {
    let mut n = [0.0; 3];
    for k in 1..c.len() - 1 {
        n = add(n, cross(sub(c[k].p, c[0].p), sub(c[k + 1].p, c[0].p)));
    }
    if n[2] < 0.0 { scale(n, -1.0) } else { n }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_sun_rises_east_and_sets_west() {
        assert!(sun(3.0).is_none());
        assert!(sun(21.0).is_none());
        let (az, el, s) = sun(8.0).unwrap();
        assert!(az > 0.5 && el > 0.3 && s == 1.0);
        let (az, el, _) = sun(12.5).unwrap();
        assert!(az.abs() < 0.01 && (el.to_degrees() - 58.0).abs() < 0.1);
        let (az, _, _) = sun(18.0).unwrap();
        assert!(az < -0.5);
        let up = DEFAULT_UP;
        let morning = ShadowMap::sun_dir(normalize(up), 1.5, 0.5);
        assert!(morning[0] > 0.8, "{morning:?}");
        // At noon in the north: behind the scene, so shadows fall toward the camera.
        let noon = ShadowMap::sun_dir(normalize(up), 0.0, 1.0);
        let south = normalize(cross([1.0, 0.0, 0.0], up));
        assert!(dot(noon, south) < -0.3, "{noon:?}");
    }

    #[test]
    fn wrapping_follows_the_ds() {
        assert_eq!(wrap(17, 16, 1 << 16, 16, 18), 1);
        assert_eq!(wrap(17, 16, (1 << 16) | (1 << 18), 16, 18), 14);
        assert_eq!(wrap(-3, 16, 0, 16, 18), 0);
    }
}
