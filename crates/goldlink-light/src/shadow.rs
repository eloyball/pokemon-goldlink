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
    /// The object it belongs to (a model, a sprite; -1 none), within the frame.
    pub object: i32,
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
/// Past the picture's edges (patch 0030): not drawn, a caster only (a building or a row of trees
/// just above the screen shadows the ground in view).
const FLAG_OFFSCREEN: u32 = 128;
/// Cast nothing: the DS's shadow volumes (4, 8), lines (32), the game's own shadows (64). See-through
/// polygons (alpha under 31) cast nothing either; ones drawn whole with an alpha texture (the
/// windmill's soft-edged blades, flag 2) cast through their texels that show.
const SKIP: u32 = 4 | 8 | 32 | 64;

/// The ground's normal when no polygons say (HeartGold's field camera).
const DEFAULT_UP: V3 = [0.0, 0.66, 0.75];

/// Shadow map texels per view unit (a DS pixel across is one unit).
pub const TEXELS: f32 = 2.0;
/// At most this many texels a side.
const MAX_SIDE: usize = 1536;
/// How far a pixel is pushed off its surface (along its normal, then toward the sun) before the
/// test, so a surface doesn't shadow itself.
const NORMAL_OFFSET: f32 = 0.4;
const BIAS: f32 = 0.4;
/// How far an upright face's bottom edge goes on back along the camera's rays (view units): past
/// the ground behind a model standing a little above it.
const SKIRT: f32 = 6.0;
/// How thick an upright face is as a caster, and how far its copy reaches down (view units).
const SLAB: f32 = 3.0;
/// How far a standing card goes into the ground when it casts (view units): the test's margin.
const SINK: f32 = 1.5;

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

/// The moon at `hours` (its azimuth, elevation and how strong its shadows are): it crosses the
/// northern sky by night as the sun does by day (rising at 20, setting at 5, 45 degrees at its
/// highest), its shadows half the sun's strength (the night's key light is faint as it is).
pub fn moon(hours: f32) -> Option<(f32, f32, f32)> {
    let t = (hours.rem_euclid(24.0) - 20.0).rem_euclid(24.0) / 9.0;
    if !(0.0..=1.0).contains(&t) {
        return None;
    }
    let arc = std::f32::consts::PI * t;
    let elevation = 45f32.to_radians() * arc.sin();
    let strength = 0.5 * (elevation.to_degrees() / 10.0).clamp(0.0, 1.0);
    if strength <= 0.0 {
        return None;
    }
    let azimuth = 80f32.to_radians() * arc.cos();
    Some((azimuth, elevation.max(25f32.to_radians()), strength))
}

/// A card: a sprite turned toward the camera (a person, a Pokémon, a sign, a tree). Casting, it
/// stands at its foot turned toward the light.
#[derive(Debug, Clone)]
struct Card {
    /// On the card's plane: a point, its normal, across and up the card (unit).
    origin: V3,
    normal: V3,
    across: V3,
    up: V3,
    /// Its extent along `across` and `up` from `origin` (`b[0]`: its feet, its lowest texels that
    /// show).
    a: [f32; 2],
    b: [f32; 2],
    /// Where it stands on the ground.
    foot: V3,
    /// Its corners: across from its middle, up from its feet, texture coordinates.
    local: Vec<([f32; 2], [f32; 2])>,
    tex: Option<ShadowTex>,
    param: u32,
    attached: bool,
    /// How much of each light reaches it (the sun's, then each point light's): the whole card
    /// is lit as one (another card's shadow across a flat sprite cut it in blocks: the user's
    /// look, 2026-11-14): a person as their foot, a tree (taller than a person) as the mean along
    /// its middle (by its foot alone a forest came out patchy).
    sun: f32,
    points: Vec<f32>,
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

    /// Its corners standing at its foot, turned to face `toward` (a horizontal direction), sunk
    /// into the ground by the test's margin so the shadow starts where the feet touch it.
    fn standing(&self, up: V3, toward: V3) -> Vec<Corner> {
        let across = normalize(cross(up, toward));
        self.local
            .iter()
            .map(|&([a, b], st)| Corner {
                p: add(self.foot, add(scale(across, a), scale(up, b - SINK))),
                st,
                scr: [0.0; 2],
            })
            .collect()
    }
}

/// A polygon as a caster: its corners in view space (where the polygons put them), its texture's
/// transparency, the card it belongs to (index + 1; 0 none).
struct Caster {
    corners: Vec<Corner>,
    tex: Option<ShadowTex>,
    param: u32,
    card: u32,
}

/// How a depth map sees the scene: along the sun (orthographic: axes `u`, `v`, the map's origin
/// and texels per unit) or from a point light through one face of a cube (`forward`, `right`,
/// `upward`, 90 degrees, `size` texels a side).
#[derive(Debug, Clone, Copy)]
enum View {
    Sun {
        dir: V3,
        u: V3,
        v: V3,
        origin: [f32; 2],
        texels: f32,
    },
    Face {
        pos: V3,
        forward: V3,
        right: V3,
        upward: V3,
        size: f32,
    },
}

/// Nothing nearer a point light than this casts (view units).
const NEAR: f32 = 0.5;

impl View {
    /// A point on the map: its column, row (texels) and nearness to the light (the larger, the
    /// nearer: along the sun's direction, or one over the distance in front of the face).
    fn map(&self, p: V3) -> Option<[f32; 3]> {
        match *self {
            View::Sun {
                dir,
                u,
                v,
                origin,
                texels,
            } => Some([
                (dot(p, u) - origin[0]) * texels,
                (dot(p, v) - origin[1]) * texels,
                dot(p, dir),
            ]),
            View::Face {
                pos,
                forward,
                right,
                upward,
                size,
            } => {
                let d = sub(p, pos);
                let z = dot(d, forward);
                (z >= NEAR).then(|| {
                    [
                        (dot(d, right) / z * 0.5 + 0.5) * size,
                        (dot(d, upward) / z * 0.5 + 0.5) * size,
                        1.0 / z,
                    ]
                })
            }
        }
    }

    /// A polygon's corners on the map, cut where it passes behind a point light's face.
    fn polygon(&self, c: &[Corner]) -> Vec<([f32; 3], [f32; 2])> {
        let View::Face { pos, forward, .. } = *self else {
            return c
                .iter()
                .filter_map(|c| Some((self.map(c.p)?, c.st)))
                .collect();
        };
        let z = |p: V3| dot(sub(p, pos), forward);
        let mut out = Vec::with_capacity(c.len() + 2);
        for k in 0..c.len() {
            let (a, b) = (&c[k], &c[(k + 1) % c.len()]);
            let (za, zb) = (z(a.p), z(b.p));
            if za >= NEAR {
                out.extend(self.map(a.p).map(|m| (m, a.st)));
            }
            if (za >= NEAR) != (zb >= NEAR) {
                let t = (NEAR - za) / (zb - za);
                let p = add(a.p, scale(sub(b.p, a.p), t));
                let st = [
                    a.st[0] + (b.st[0] - a.st[0]) * t,
                    a.st[1] + (b.st[1] - a.st[1]) * t,
                ];
                // A hair in front of the plane, so it maps.
                let p = add(p, scale(forward, 1e-3));
                out.extend(self.map(p).map(|m| (m, st)));
            }
        }
        out
    }
}

/// A depth map of the casters as one light sees them.
struct DepthMap {
    view: View,
    side: [usize; 2],
    /// Per texel: the nearest to the light, and its card (index + 1; 0 none).
    depth: Vec<f32>,
    card: Vec<u32>,
}

impl DepthMap {
    fn new(view: View, side: [usize; 2]) -> Self {
        let n = side[0] * side[1];
        Self {
            view,
            side,
            depth: vec![f32::MIN; n],
            card: vec![0; n],
        }
    }

    /// Draws the casters (`casters`, then `extra`) in bands of rows on the pool's threads.
    fn draw(
        &mut self,
        casters: &[&Caster],
        extra: &[&Caster],
        alpha: &[u8],
        pool: Option<&rayon::ThreadPool>,
    ) {
        let mut tris = Vec::new();
        for c in casters.iter().chain(extra) {
            // A point light's face: casters wholly outside its view (behind it, or past one of
            // its four sides) are passed over at once (each caster lies in one or two faces).
            if let View::Face {
                pos,
                forward,
                right,
                upward,
                ..
            } = self.view
            {
                let mut out = [true; 5];
                for k in &c.corners {
                    let d = sub(k.p, pos);
                    let (z, x, y) = (dot(d, forward), dot(d, right), dot(d, upward));
                    out[0] &= z < NEAR;
                    out[1] &= x > z;
                    out[2] &= x < -z;
                    out[3] &= y > z;
                    out[4] &= y < -z;
                }
                if out.iter().any(|&o| o) {
                    continue;
                }
            }
            let m = self.view.polygon(&c.corners);
            for k in 1..m.len().saturating_sub(1) {
                let t = Tri {
                    m: [m[0].0, m[k].0, m[k + 1].0],
                    st: [m[0].1, m[k].1, m[k + 1].1],
                    tex: c.tex,
                    param: c.param,
                    card: c.card,
                };
                if t.area().abs() < 1e-6 {
                    continue;
                }
                let y0 = t.m.iter().map(|p| p[1]).fold(f32::MAX, f32::min);
                let y1 = t.m.iter().map(|p| p[1]).fold(f32::MIN, f32::max);
                if y1 < 0.0 || y0 >= self.side[1] as f32 {
                    continue;
                }
                tris.push((y0.floor().max(0.0) as usize, y1.ceil() as usize, t));
            }
        }
        let width = self.side[0];
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
    }

    /// Whether something nearer the light lies over `q` (a point already pushed off its surface)
    /// with `slope` (how far its surface turns from the light, for the bias): the card it is
    /// (index + 1), 0 for a polygon; None when nothing does.
    fn over(&self, q: V3, slope: f32) -> Option<u32> {
        let [x, y, d] = self.view.map(q)?;
        if x < 0.0 || y < 0.0 {
            return None;
        }
        let (x, y) = (x as usize, y as usize);
        if x >= self.side[0] || y >= self.side[1] {
            return None;
        }
        let i = y * self.side[0] + x;
        let occluded = match self.view {
            View::Sun { texels, .. } => self.depth[i] > d + BIAS * (1.0 + slope / texels),
            // One over the distance: nearer than the receiver by the bias.
            View::Face { .. } => {
                let z = 1.0 / d;
                self.depth[i] > 1.0 / (z - BIAS * (1.0 + slope)).max(NEAR)
            }
        };
        occluded.then_some(self.card[i])
    }
}

/// A point light's depth maps: the six faces of a cube round it.
struct PointMaps {
    pos: V3,
    faces: Vec<DepthMap>,
}

/// Texels a side of a point light's face: the nearest the player's, the others'.
pub const FACE: usize = 64;
pub const FACE_COARSE: usize = 32;

/// The picture's polygons as shadow casters, and the depth maps of the lights that cast shadows:
/// the sun (or the moon) and the nearest point lights.
#[derive(Default)]
pub struct Shadows {
    /// The ground's normal found in the last picture.
    pub up: V3,
    /// The projection and its inverse (rows, vectors on the left).
    proj: [[f32; 4]; 4],
    rows: [[f32; 4]; 4],
    /// Where the polygons truly land: column = fit[0] + fit[1] * x, row = fit[2] + fit[3] * y (x, y
    /// normalised device coordinates). The steady overworld's viewport isn't the screen's (it's
    /// 4/3 as wide), so the lighting's pixels ([`super::unproject`]) are placed on another scale
    /// across than the polygons: [`Shadows::true_place`] moves them onto the polygons'.
    fit: [f32; 4],
    /// Everything that casts but the cards (with their skirts and slabs).
    casters: Vec<Caster>,
    cards: Vec<Card>,
    /// Per pixel of the picture (`PICTURE_WIDTH` x 192): the card it shows (index + 1; 0 none).
    card_px: Vec<u32>,
    alpha: Vec<u8>,
    /// The sun's map and its direction (toward it); the point lights' maps.
    sun: Option<(DepthMap, V3)>,
    points: Vec<PointMaps>,
    /// A corner of the drawn ground (view space), for laying the sun's texels on the world.
    anchor: Option<[f64; 3]>,
    /// Where the picture's pixels lie (view space, a sample): the sun's map covers them (what
    /// casts past the picture's edges can shadow only what's in it).
    shown: Vec<V3>,
    pub ready: bool,
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
#[derive(Debug, Clone, Copy)]
struct Corner {
    p: V3,
    st: [f32; 2],
    /// Where it lands on the picture (its columns and rows).
    scr: [f32; 2],
}

impl Shadows {
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

    /// The picture's polygons as casters: placed in 3D, cards found and stood on their feet
    /// (`seen`: the lighting's place of a pixel of the picture, column and row), upright faces
    /// with their skirts and slabs. No light's map yet.
    pub fn prepare(
        &mut self,
        g: &Geometry,
        proj: &[[f32; 4]; 4],
        rows: &[[f32; 4]; 4],
        up: V3,
        seen: &dyn Fn(f32, f32) -> Option<V3>,
        pool: Option<&rayon::ThreadPool>,
    ) {
        self.ready = false;
        self.sun = None;
        self.points.clear();
        self.up = up;
        self.proj = *proj;
        self.rows = *rows;
        self.alpha.clear();
        self.alpha.extend_from_slice(g.alpha);
        // Where the vertices land against their device coordinates.
        let mut s = [[0f64; 5]; 2];
        let drawn = g
            .polys
            .iter()
            .filter(|p| p.flags & FLAG_OFFSCREEN == 0)
            .filter_map(|p| {
                g.verts
                    .get(p.first_vert as usize..(p.first_vert + p.num_verts) as usize)
            })
            .flatten();
        for v in drawn {
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
        // A point that moves with the world: where its origin is drawn (the wide picture's
        // columns: the screen's plus 64), on the level plane through the eye (the camera moves
        // level). Orthographic cameras only (the steady overworld's).
        self.casters.clear();
        self.cards.clear();

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
        // Where the picture's pixels lie, every fourth (the drawn polygons reach the guard band,
        // far past the picture).
        self.shown.clear();
        for row in (0..PICTURE_HEIGHT).step_by(4).chain([PICTURE_HEIGHT - 1]) {
            for col in (0..PICTURE_WIDTH).step_by(4).chain([PICTURE_WIDTH - 1]) {
                if let Some(q) = seen(col as f32 + 0.5, row as f32 + 0.5) {
                    self.shown.extend(self.true_place(q));
                }
            }
        }
        // The camera's rays: along -z (an orthographic camera, as the steady overworld's), else
        // from the eye.
        let ortho = proj[2][3].abs() < 1e-6;
        let back = |p: V3| {
            if ortho {
                [0.0, 0.0, -1.0]
            } else {
                normalize(p)
            }
        };
        // People are drawn nearer the camera than where they stand (so they show over the
        // ground): their foot goes back along the camera's ray onto the ground behind it.
        // Flat ground only (a roof behind a person isn't where they stand).
        let flats: Vec<&(Vec<Corner>, V3, &ShadowPoly)> = placed
            .iter()
            .filter(|(_, n, p)| p.tex < 0 && dot(*n, up) >= 0.97)
            .collect();
        let ground = |foot: V3| -> V3 {
            let d = back(foot);
            let mut best: Option<f32> = None;
            for (c, n, _) in flats.iter().map(|f| (&f.0, &f.1, f.2)) {
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
        let north = normalize(cross(up, [1.0, 0.0, 0.0]));
        // A corner of the drawn ground that moves with the world exactly: the nearest the
        // picture's middle of the untextured flat polygons wholly in the picture (not cut at its
        // edge). The sun's texels are laid from it (else they slid under the scene as the camera
        // moved, and shadows' edges flickered: the user's look).
        {
            let mut best: Option<(f32, V3)> = None;
            for (c, n, p) in &placed {
                if p.tex >= 0 || dot(*n, up) < 0.97 {
                    continue;
                }
                if !c.iter().all(|k| {
                    k.scr[0] > 8.0
                        && k.scr[0] < (PICTURE_WIDTH - 8) as f32
                        && k.scr[1] > 8.0
                        && k.scr[1] < (PICTURE_HEIGHT - 8) as f32
                }) {
                    continue;
                }
                for k in c {
                    let d = (k.scr[0] - 192.0).powi(2) + (k.scr[1] - 96.0).powi(2);
                    if best.is_none_or(|b| d < b.0) {
                        best = Some((d, k.p));
                    }
                }
            }
            self.anchor = best.map(|(_, p)| p.map(|v| v as f64));
        }
        // The ground's main level: the height (along up) most of the flat ground lies at. Flat
        // ground there casts nothing (nothing at that level lies under it): left out of the maps,
        // which were mostly ground (the user's machine draws them every frame). Raised flat
        // ground (a bridge, a plateau) still casts.
        let level = {
            let mut bins: Vec<(f32, f32)> = Vec::new();
            for (c, n, p) in &placed {
                // What the picture shows (past its edges, another level can be most of it).
                if p.tex >= 0 || dot(*n, up) < 0.97 || p.flags & FLAG_OFFSCREEN != 0 {
                    continue;
                }
                let h = (dot(c[0].p, up)).round();
                let area = dot(area_normal(c), area_normal(c)).sqrt();
                match bins.iter_mut().find(|b| b.0 == h) {
                    Some(b) => b.1 += area,
                    None => bins.push((h, area)),
                }
            }
            // The lowest level (flat ground can shadow only what lies lower), at the middle of
            // the ground's heights within LEVEL_SPREAD of it: the picture in between moves each
            // map block by its own whole pixels, and the same ground came out at two heights two
            // units apart. By the most ground instead, the level flipped between a town's ground
            // and a river's bed as the water came into view, and the town's ground's shadows on
            // the water popped in and out (the user, walking up Violet City). The lowest changes
            // only as lower ground comes into view at the picture's edge, a sliver.
            let lowest = bins
                .iter()
                .filter(|b| b.1 >= LEVEL_MIN_AREA)
                .map(|b| b.0)
                .fold(None, |m: Option<f32>, h| Some(m.map_or(h, |m| m.min(h))));
            lowest.map(|h| {
                let (area, sum) = bins
                    .iter()
                    .filter(|b| b.0 - h <= LEVEL_SPREAD && b.0 >= h)
                    .fold((0.0f32, 0.0f32), |s, b| (s.0 + b.1, s.1 + b.0 * b.1));
                sum / area
            })
        };
        let mut shapes: Vec<(Vec<Corner>, Option<ShadowTex>, u32)> = Vec::new();
        for (corners, n, p) in &placed {
            let tex = (p.tex >= 0)
                .then(|| g.texs.get(p.tex as usize))
                .flatten()
                .copied();
            if p.tex >= 0 {
                // A decal lying on the ground (flowers, grass): casts nothing.
                if dot(*n, up).abs() > 0.9 {
                    continue;
                }
                // A card turned toward the camera: stood up at its foot.
                if n[2] > 0.9 {
                    let n = *n;
                    let across = normalize(cross(up, n));
                    let card_up = normalize(cross(n, across));
                    let o = corners[0].p;
                    let (mut a, mut b) = ([f32::MAX, f32::MIN], [f32::MAX, f32::MIN]);
                    for c in corners {
                        let d = sub(c.p, o);
                        let (ca, cb) = (dot(d, across), dot(d, card_up));
                        a = [a[0].min(ca), a[1].max(ca)];
                        b = [b[0].min(cb), b[1].max(cb)];
                    }
                    // Where the sprite's feet are: its lowest row of texels that show (a sprite has
                    // empty rows under its feet), up from the card's bottom edge.
                    let lift = tex.map_or(0.0, |t| {
                        sprite_lift(corners, o, card_up, b, t, p.tex_param, g.alpha)
                    });
                    let b = [b[0] + lift, b[1]];
                    let mid = (a[0] + a[1]) / 2.0;
                    // The foot: the middle of the feet, on the ground there. The ground is what
                    // the picture shows just under them (the depth behind the sprite's empty
                    // texels); else along the camera's ray onto the polygons behind.
                    let bottom = add(o, add(scale(across, mid), scale(card_up, b[0])));
                    let at = screen_at(corners, o, across, card_up, mid, b[0]);
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
                    let local = corners
                        .iter()
                        .map(|c| {
                            let d = sub(c.p, o);
                            ([dot(d, across) - mid, dot(d, card_up) - b[0]], c.st)
                        })
                        .collect();
                    self.cards.push(Card {
                        origin: o,
                        normal: n,
                        across,
                        up: card_up,
                        a,
                        b,
                        foot,
                        local,
                        tex,
                        param: p.tex_param,
                        attached: p.flags & FLAG_ATTACHED != 0,
                        sun: 1.0,
                        points: Vec::new(),
                    });
                    shapes.push((corners.clone(), tex, p.tex_param));
                    continue;
                }
            }
            // An upright face (a wall, a post, a sign's box) meets the ground where it shows on
            // the picture: its bottom edge goes on back along the camera's rays (a strip seen
            // edge-on, so the picture is the same) to the ground behind. The game stands some
            // models a unit or two above the ground (the sign, the mailbox, the windmill) and the
            // DS draws no face turned from the camera, so from the north the sun saw under them,
            // and their shadows started a pixel or two short (the user's look). And it's solid:
            // a copy of the face behind it (away from the camera, reaching down to the ground)
            // makes it a slab, not an open shell.
            if dot(*n, up).abs() < 0.3 {
                let low = corners
                    .iter()
                    .map(|c| dot(c.p, up))
                    .fold(f32::MAX, f32::min);
                let bottom: Vec<usize> = (0..corners.len())
                    .filter(|&k| dot(corners[k].p, up) - low < 0.5)
                    .collect();
                if let [k0, k1, ..] = bottom[..] {
                    let (a, b) = (corners[k0], corners[k1]);
                    let away = |c: Corner| Corner {
                        p: add(c.p, scale(back(c.p), SKIRT)),
                        ..c
                    };
                    self.casters.push(Caster {
                        corners: vec![a, b, away(b), away(a)],
                        tex: None,
                        param: 0,
                        card: 0,
                    });
                }
                let slab = corners
                    .iter()
                    .map(|c| {
                        let down = if dot(c.p, up) - low < 0.5 { SLAB } else { 0.0 };
                        Corner {
                            p: sub(add(c.p, scale(north, SLAB)), scale(up, down)),
                            ..*c
                        }
                    })
                    .collect();
                self.casters.push(Caster {
                    corners: slab,
                    tex,
                    param: p.tex_param,
                    card: 0,
                });
            }
            let ground_level = dot(*n, up) > 0.97
                && level.is_some_and(|l| {
                    corners
                        .iter()
                        .all(|c| (dot(c.p, up) - l).abs() < LEVEL_SPREAD + 1.5)
                });
            if !ground_level {
                self.casters.push(Caster {
                    corners: corners.clone(),
                    tex,
                    param: p.tex_param,
                    card: 0,
                });
            }
        }
        // Which card each pixel of the picture shows (the nearest, its transparent texels left
        // out).
        self.card_px.clear();
        self.card_px.resize(PICTURE_WIDTH * PICTURE_HEIGHT, 0);
        let mut near = vec![f32::MIN; PICTURE_WIDTH * PICTURE_HEIGHT];
        let mut tris = Vec::new();
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
                // Its own rows only.
                let y0 = t.m.iter().map(|p| p[1]).fold(f32::MAX, f32::min);
                let y1 = t.m.iter().map(|p| p[1]).fold(f32::MIN, f32::max);
                let y0 = y0.floor().clamp(0.0, PICTURE_HEIGHT as f32) as usize;
                let y1 = y1.ceil().clamp(0.0, PICTURE_HEIGHT as f32) as usize;
                if y0 < y1 {
                    tris.push((y0, y1, t));
                }
            }
        }
        // In bands of rows on the pool's threads (a forest's cards cost milliseconds on one).
        let draw = |band: usize, near: &mut [f32], card: &mut [u32]| {
            let (r0, r1) = (band * BAND, band * BAND + near.len() / PICTURE_WIDTH);
            for (y0, y1, t) in &tris {
                if *y1 <= r0 || *y0 >= r1 {
                    continue;
                }
                t.draw(
                    (*y0).max(r0),
                    (*y1).min(r1),
                    r0,
                    PICTURE_WIDTH,
                    near,
                    card,
                    g.alpha,
                );
            }
        };
        let bands = near
            .chunks_mut(BAND * PICTURE_WIDTH)
            .zip(self.card_px.chunks_mut(BAND * PICTURE_WIDTH))
            .enumerate();
        match pool {
            Some(pool) => {
                use rayon::prelude::*;
                let mut bands: Vec<_> = bands.collect();
                pool.install(|| bands.par_iter_mut().for_each(|(b, (n, c))| draw(*b, n, c)));
            }
            None => bands.for_each(|(b, (n, c))| draw(b, n, c)),
        }
        self.ready = true;
    }

    /// The cards standing at their feet turned toward a light (`toward(foot)`: the horizontal
    /// direction to it from there).
    fn standing_cards(&self, toward: impl Fn(V3) -> V3, trees: bool) -> Vec<Caster> {
        self.cards
            .iter()
            .enumerate()
            .filter(|(_, c)| trees || c.b[1] - c.b[0] <= TREE_HEIGHT)
            .map(|(k, c)| Caster {
                corners: c.standing(self.up, toward(c.foot)),
                tex: c.tex,
                param: c.param,
                card: k as u32 + 1,
            })
            .collect()
    }

    /// The sun's (or the moon's) map, toward `dir` at `texels` per unit, and each card's light
    /// from it.
    pub fn build_sun(&mut self, dir: V3, texels: f32, pool: Option<&rayon::ThreadPool>) {
        self.sun = None;
        let up = self.up;
        let u = normalize(cross(up, dir));
        if !self.ready || dot(u, u) < 0.5 {
            return;
        }
        let v = cross(dir, u);
        let flat = normalize(sub(dir, scale(up, dot(dir, up))));
        // Trees cast no sun shadows: their sprites have one painted under them, and stood up
        // along the sun a forest's were many, costly, and never quite the tree's shape (the
        // user).
        let cards = self.standing_cards(|_| flat, false);
        let (mut lo, mut hi) = ([f32::MAX; 2], [f32::MIN; 2]);
        for c in self.casters.iter().chain(&cards) {
            for k in &c.corners {
                let m = [dot(k.p, u), dot(k.p, v)];
                for i in 0..2 {
                    lo[i] = lo[i].min(m[i]);
                    hi[i] = hi[i].max(m[i]);
                }
            }
        }
        // No more than what the picture shows (the casters past its edges reach far past it; a
        // map over them all is more texels to draw).
        if !self.shown.is_empty() {
            let (mut slo, mut shi) = ([f32::MAX; 2], [f32::MIN; 2]);
            for p in &self.shown {
                let m = [dot(*p, u), dot(*p, v)];
                for i in 0..2 {
                    slo[i] = slo[i].min(m[i]);
                    shi[i] = shi[i].max(m[i]);
                }
            }
            for i in 0..2 {
                lo[i] = lo[i].max(slo[i] - 1.0);
                hi[i] = hi[i].min(shi[i] + 1.0);
            }
        }
        if lo[0] > hi[0] || lo[1] > hi[1] {
            return;
        }
        let extent = (hi[0] - lo[0]).max(hi[1] - lo[1]).max(1.0);
        let texels = texels.min(MAX_SIDE as f32 / extent);
        // The map's origin on whole texels from the world's anchor (texels laid on the world, so
        // they stay put on it as the camera moves), else from the eye.
        let (au, av) = self.anchor.map_or((0.0, 0.0), |a| {
            let d = |x: V3| a[0] * x[0] as f64 + a[1] * x[1] as f64 + a[2] * x[2] as f64;
            (d(u), d(v))
        });
        let snap = |lo: f32, a: f64| -> f32 {
            let t = texels as f64;
            (a + ((lo as f64 - a) * t).floor() / t) as f32
        };
        let origin = [snap(lo[0], au), snap(lo[1], av)];
        let side = [
            (((hi[0] - origin[0]) * texels).ceil() as usize + 1).min(MAX_SIDE),
            (((hi[1] - origin[1]) * texels).ceil() as usize + 1).min(MAX_SIDE),
        ];
        let view = View::Sun {
            dir,
            u,
            v,
            origin,
            texels,
        };
        let mut map = DepthMap::new(view, side);
        let casters: Vec<&Caster> = self.casters.iter().collect();
        let cards: Vec<&Caster> = cards.iter().collect();
        map.draw(&casters, &cards, &self.alpha, pool);
        self.sun = Some((map, dir));
        for k in 0..self.cards.len() {
            self.cards[k].sun = self.card_light(k, None, flat);
        }
    }

    /// The point lights' maps (`lights`: where each is, as the lighting places it), and each
    /// card's light from them.
    /// The point lights' maps (`lights`: where each is as the lighting places it, its reach and
    /// its faces' size; `receivers`: a sample of the picture's pixels where the polygons put
    /// them), and each card's light from them. Only the faces something in reach looks out
    /// through are drawn; all of every light's at once on the pool's threads.
    pub fn build_points(
        &mut self,
        lights: &[(V3, f32, usize)],
        receivers: &[V3],
        pool: Option<&rayon::ThreadPool>,
    ) {
        self.points.clear();
        if !self.ready {
            return;
        }
        let up = self.up;
        let axes: [V3; 6] = [
            [1.0, 0.0, 0.0],
            [-1.0, 0.0, 0.0],
            [0.0, 1.0, 0.0],
            [0.0, -1.0, 0.0],
            [0.0, 0.0, 1.0],
            [0.0, 0.0, -1.0],
        ];
        // Per light: where it is, what casts within its reach, and its faces (empty where nothing
        // in reach looks out through them).
        let mut work: Vec<(Vec<&Caster>, Vec<Caster>)> = Vec::new();
        let mut faces: Vec<(usize, DepthMap)> = Vec::new();
        for &(light, range, size) in lights {
            let Some(pos) = self.true_place(light) else {
                continue;
            };
            let k = self.points.len();
            let cards: Vec<Caster> = self
                .standing_cards(
                    |foot| {
                        let d = sub(pos, foot);
                        normalize(sub(d, scale(up, dot(d, up))))
                    },
                    true,
                )
                .into_iter()
                .filter(|c| reaches(&c.corners, pos, range))
                .collect();
            // Only what's within its reach casts (what's beyond it lies in the dark anyway).
            let near: Vec<&Caster> = self
                .casters
                .iter()
                .filter(|c| reaches(&c.corners, pos, range))
                .collect();
            let mut used = [false; 6];
            for r in receivers {
                let d = sub(*r, pos);
                if dot(d, d) >= range * range {
                    continue;
                }
                let a = (0..3)
                    .max_by(|&a, &b| d[a].abs().total_cmp(&d[b].abs()))
                    .unwrap_or(0);
                used[2 * a + usize::from(d[a] < 0.0)] = true;
            }
            for (f, &forward) in axes.iter().enumerate() {
                let helper = if forward[1].abs() > 0.5 {
                    [0.0, 0.0, 1.0]
                } else {
                    [0.0, 1.0, 0.0]
                };
                let right = normalize(cross(helper, forward));
                let upward = cross(forward, right);
                let side = if used[f] { size } else { 0 };
                faces.push((
                    k,
                    DepthMap::new(
                        View::Face {
                            pos,
                            forward,
                            right,
                            upward,
                            size: size as f32,
                        },
                        [side, side],
                    ),
                ));
            }
            work.push((near, cards));
            self.points.push(PointMaps {
                pos,
                faces: Vec::new(),
            });
        }
        let alpha = &self.alpha;
        let draw = |(k, f): &mut (usize, DepthMap)| {
            if f.side[0] == 0 {
                return;
            }
            let (near, cards) = &work[*k];
            let cards: Vec<&Caster> = cards.iter().collect();
            f.draw(near, &cards, alpha, None);
        };
        match pool {
            Some(pool) => {
                use rayon::prelude::*;
                pool.install(|| faces.par_iter_mut().for_each(draw));
            }
            None => faces.iter_mut().for_each(draw),
        }
        drop(work);
        for (k, f) in faces {
            self.points[k].faces.push(f);
        }
        for k in 0..self.cards.len() {
            let points = (0..self.points.len())
                .map(|i| {
                    let d = sub(self.points[i].pos, self.cards[k].foot);
                    let flat = normalize(sub(d, scale(up, dot(d, up))));
                    self.card_light(k, Some(i), flat)
                })
                .collect();
            self.cards[k].points = points;
        }
    }

    /// A card's light from the sun (`point` None) or a point light, itself left out: a person's,
    /// their foot's; a tree's, the mean along its middle (`flat`: toward the light, level).
    fn card_light(&self, k: usize, point: Option<usize>, flat: V3) -> f32 {
        let c = &self.cards[k];
        let up = self.up;
        let height = c.b[1] - c.b[0];
        if height > TREE_HEIGHT {
            let at = |f: f32| add(c.foot, scale(up, height * f));
            [0.3, 0.5, 0.7]
                .iter()
                .map(|&f| self.visible(point, at(f), flat, true, Some(k)))
                .sum::<f32>()
                / 3.0
        } else {
            self.visible(point, add(c.foot, scale(up, 0.5)), up, true, Some(k))
        }
    }

    /// Whether the sun's map was drawn this frame.
    pub fn has_sun(&self) -> bool {
        self.sun.is_some()
    }

    /// How much of a light (the sun with `point` None, else that point light) reaches `p` (where
    /// the polygons put the pixel: [`Shadows::place`]; at the picture's column `col` and row
    /// `row`; its surface's normal `n`): 0 or 1. With `attached` false (the player's own pixels),
    /// what the camera follows casts nothing.
    pub fn light(
        &self,
        point: Option<usize>,
        p: V3,
        n: V3,
        attached: bool,
        col: usize,
        row: usize,
    ) -> f32 {
        if !self.ready {
            return 1.0;
        }
        // A card's pixels: lit as one.
        if col < PICTURE_WIDTH
            && let Some(&k) = self.card_px.get(row * PICTURE_WIDTH + col)
            && k > 0
        {
            let c = &self.cards[k as usize - 1];
            if (!attached && c.attached) || c.holds(p) {
                return match point {
                    None => c.sun,
                    Some(i) => c.points.get(i).copied().unwrap_or(1.0),
                };
            }
        }
        self.visible(point, p, n, attached, None)
    }

    /// Whether a light reaches `p` (where the polygons put it), `card` left out.
    fn visible(
        &self,
        point: Option<usize>,
        p: V3,
        n: V3,
        attached: bool,
        card: Option<usize>,
    ) -> f32 {
        // Toward the light, and the map that sees `p`.
        let (to, map) = match point {
            None => match &self.sun {
                Some((map, dir)) => (*dir, map),
                None => return 1.0,
            },
            Some(i) => {
                let Some(pm) = self.points.get(i) else {
                    return 1.0;
                };
                let d = sub(p, pm.pos);
                // The face whose axis is nearest the direction from the light.
                let k = (0..3)
                    .max_by(|&a, &b| d[a].abs().total_cmp(&d[b].abs()))
                    .unwrap_or(0);
                let face = 2 * k + usize::from(d[k] < 0.0);
                (normalize(scale(d, -1.0)), &pm.faces[face])
            }
        };
        // Pushed off the surface further the more it turns from the light (a wall the light
        // grazes changes depth fast across a texel).
        let facing = dot(n, to).clamp(0.05, 1.0);
        let slope = ((1.0 - facing * facing).sqrt() / facing).min(3.0);
        let q = add(p, scale(n, NORMAL_OFFSET * (1.0 + slope)));
        match map.over(q, slope) {
            None => 1.0,
            Some(0) => 0.0,
            Some(k) => {
                let k = k as usize - 1;
                let c = &self.cards[k];
                if card == Some(k) || c.holds(p) || (!attached && c.attached) {
                    1.0
                } else {
                    0.0
                }
            }
        }
    }

    /// A pixel's place as the lighting found it, where the polygons put it (for
    /// [`Shadows::light`]).
    pub fn place(&self, p: V3) -> Option<V3> {
        self.true_place(p)
    }

    /// A pixel's place as the lighting found it (its column `128 x + 191.5` of the wide picture),
    /// moved to where the polygons put that pixel ([`Shadows::fit`]).
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

/// How far apart (view units, along up) flat ground still counts as one level: the picture in
/// between moves each map block by its own whole pixels (two units apart at most seen); raised
/// ground (a bridge, a ledge) is 8 or more above.
const LEVEL_SPREAD: f32 = 3.0;
/// The least flat ground (square view units) a level needs to count (a polygon's sliver at the
/// picture's edge, a step's top, aren't one).
const LEVEL_MIN_AREA: f32 = 64.0;

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
            // Only the row's span inside the triangle (its edges crossing the row's middle, a
            // texel's margin either side; the tests below decide each texel as before): a long
            // slanted triangle (a tree's shadow by a low sun) covers little of its box.
            let (mut lo, mut hi) = (f32::MAX, f32::MIN);
            for k in 0..3 {
                let (a, b) = (m[k], m[(k + 1) % 3]);
                if (a[1] - py) * (b[1] - py) > 0.0 || a[1] == b[1] {
                    continue;
                }
                let x = a[0] + (py - a[1]) * (b[0] - a[0]) / (b[1] - a[1]);
                lo = lo.min(x);
                hi = hi.max(x);
            }
            if lo > hi {
                continue;
            }
            let xs = ((lo - 1.0).floor().max(0.0) as usize).max(x0);
            let xe = (((hi + 1.0).ceil().max(0.0)) as usize).min(x1);
            for x in xs..xe {
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

/// Whether a polygon comes within `range` of `pos` (its corners' box does).
fn reaches(c: &[Corner], pos: V3, range: f32) -> bool {
    let (mut lo, mut hi) = ([f32::MAX; 3], [f32::MIN; 3]);
    for k in c {
        for i in 0..3 {
            lo[i] = lo[i].min(k.p[i]);
            hi[i] = hi[i].max(k.p[i]);
        }
    }
    let d: V3 = std::array::from_fn(|i| (lo[i] - pos[i]).max(pos[i] - hi[i]).max(0.0));
    dot(d, d) < range * range
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
        let morning = Shadows::sun_dir(normalize(up), 1.5, 0.5);
        assert!(morning[0] > 0.8, "{morning:?}");
        // At noon in the north: behind the scene, so shadows fall toward the camera.
        let noon = Shadows::sun_dir(normalize(up), 0.0, 1.0);
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
