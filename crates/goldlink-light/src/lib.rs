//! GoldLink's lighting (dev/docs/unified-client/UC11_LIGHTING.md): a minimal HD-2D look.
//!
//! HeartGold draws the overworld as a 3D scene (map models, characters as flat cards) under a
//! perspective camera, and melonDS keeps that scene's depth for every pixel. From it this crate
//! finds where each pixel is and which way it faces, and lights the picture **per game pixel**:
//! every DS pixel stays one flat colour, so the pixel art stays crisp. In order:
//!
//! 1. positions from depth and the projection; normals from neighbouring positions (the nearer
//!    side at an edge, so silhouettes don't smear), smoothed over the same surface;
//! 2. ambient occlusion: nearby geometry in front of a surface darkens it a little (where a wall
//!    meets the ground), with a range check so far backgrounds behind an edge stay clean;
//! 3. a key light (the sun by day, a low warm sun in the evening, the moon at night, a ceiling
//!    light indoors) over a coloured ambient: lit faces lean warm, shade leans cool; indoors
//!    and in caves a warm light high above the player lights the room around them (corners
//!    fall darker), and a faint one at night;
//! 4. outdoors at night: bright warm pixels (the lit windows the game draws at night, lamps)
//!    become point lights placed in 3D, casting warm pools on the ground;
//! 5. the time of day's colour grade and a vignette. No blur anywhere: blurs look out of place
//!    on the game's pixels (the user, 2026-11-13).
//!
//! Pixels where the 3D scene doesn't show (a text box, menus) are left exactly as they are.
//! Battles and menus get [`Lighting::grade`] only: a small grade, no lighting.

mod gpu;
mod places;
mod shadow;

pub use shadow::{Geometry, ShadowPoly, ShadowTex, ShadowVert};

/// Whether the sun is up at `hours` (the DS clock) and casts shadows (UC13): by day the game's
/// own shadows are left out (they'd show twice); by night they stay, faint, and ground the scene.
pub fn sun_casts(hours: f32) -> bool {
    shadow::sun(hours).is_some()
}

/// The DS screen's height.
pub const HEIGHT: usize = 192;

/// The game's time of day (pret `GF_RTC_GetTimeOfDayByHour`; late night counts as night).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    Morning,
    Day,
    Evening,
    Night,
}

impl Phase {
    /// 4–9 morning, 10–16 day, 17–19 evening, 20–3 night: the game's own table.
    pub fn of_hour(hour: u32) -> Self {
        match hour % 24 {
            4..=9 => Self::Morning,
            10..=16 => Self::Day,
            17..=19 => Self::Evening,
            _ => Self::Night,
        }
    }
}

/// What kind of place a map is (its map header's type).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Place {
    /// Routes and towns.
    Outdoor,
    Indoor,
    Cave,
}

impl Place {
    /// Unknown maps (hg-engine's additions, if any) count as outdoor.
    pub fn of_map(map: u16) -> Self {
        match places::PLACES.get(map as usize) {
            Some(b'I') => Self::Indoor,
            Some(b'C') => Self::Cave,
            _ => Self::Outdoor,
        }
    }
}

/// The top screen's 3D scene as melonDS drew it (goldlink-core `LightBuffers`): `width` x 192,
/// the screen's x = 0 at column `(width - 256) / 2`.
pub struct Scene<'a> {
    /// 24-bit Z depth.
    pub depth: &'a [u32],
    /// 1: the 3D scene on top (lit); anything else: a 2D layer shows there (left as it is).
    pub layer: &'a [u8],
    /// The projection matrix, 20.12, row by row (vectors on the left).
    pub proj: [i32; 16],
    pub width: usize,
    /// Each line's fades (192, or empty: none).
    pub fade: &'a [Fade],
    /// The picture's polygons, for the cast shadows (UC13).
    pub geometry: Geometry<'a>,
    /// The DS clock: hours, with minutes as a fraction (the sun's place).
    pub hours: f32,
    /// 1 where the steady overworld's attached picture (the player, a follower) shows opaque
    /// (`width` x 192, or empty): lit there as the player, so their own shadow on the ground
    /// under them doesn't darken them.
    pub attached: &'a [u8],
}

/// A line's screen-wide fades as the DS applies them after composing (goldlink-core `Fade`):
/// the master brightness `[mode, factor]` (0 none, 1 toward white, 2 toward black; factor 0..16),
/// then the colour effect's brightness on the 3D layer, where the 3D scene shows on top.
pub type Fade = [u8; 4];

/// The game's fades (a cave's exit flash, a warp's fade) on `px` (`w` x 192, 0x00RRGGBB, column 0
/// at the layer mask's column `x0`): the lighting must light the scene under them, not the faded
/// picture, or shapes and shadows show through a white screen. [`Faded::lift`] takes them off,
/// [`Faded::put_back`] applies them again; a line faded all the way is kept as the game drew it.
struct Faded {
    /// Lines faded all the way: their pixels as drawn.
    whole: Vec<(usize, Vec<u32>)>,
}

/// `c` through a DS brightness fade (`mode` 1 toward white, 2 toward black), `f` = factor / 16.
fn fade_channel(c: f32, mode: u8, f: f32) -> f32 {
    if mode == 1 {
        c + (255.0 - c) * f
    } else {
        c * (1.0 - f)
    }
}

fn unfade_channel(c: f32, mode: u8, f: f32) -> f32 {
    let r = if mode == 1 {
        (c - 255.0 * f) / (1.0 - f)
    } else {
        c / (1.0 - f)
    };
    r.clamp(0.0, 255.0)
}

fn map_rgb(p: u32, g: impl Fn(f32) -> f32) -> u32 {
    let ch = |s: u32| (g(((p >> s) & 0xFF) as f32).round().clamp(0.0, 255.0) as u32) << s;
    (p & 0xFF00_0000) | ch(16) | ch(8) | ch(0)
}

impl Faded {
    fn lift(
        px: &mut [u32],
        w: usize,
        x0: usize,
        fade: &[Fade],
        layer: Option<(&[u8], usize)>,
    ) -> Self {
        let mut whole = Vec::new();
        for (y, f) in fade.iter().enumerate().take(HEIGHT) {
            let (mm, mf, em, ef) = (f[0], f[1], f[2], f[3]);
            if (mm == 0 || mf == 0) && (em == 0 || ef == 0) {
                continue;
            }
            let Some(row) = px.get_mut(y * w..(y + 1) * w) else {
                break;
            };
            if (mm != 0 && mf >= 16) || (em != 0 && ef >= 16) {
                whole.push((y, row.to_vec()));
                continue;
            }
            for (x, p) in row.iter_mut().enumerate() {
                if mm != 0 && mf > 0 {
                    *p = map_rgb(*p, |c| unfade_channel(c, mm, f32::from(mf) / 16.0));
                }
                let on_top = layer.is_none_or(|(l, lw)| l.get(y * lw + x0 + x) == Some(&1));
                if em != 0 && ef > 0 && on_top {
                    *p = map_rgb(*p, |c| unfade_channel(c, em, f32::from(ef) / 16.0));
                }
            }
        }
        Faded { whole }
    }

    fn put_back(
        self,
        px: &mut [u32],
        w: usize,
        x0: usize,
        fade: &[Fade],
        layer: Option<(&[u8], usize)>,
    ) {
        for (y, f) in fade.iter().enumerate().take(HEIGHT) {
            let (mm, mf, em, ef) = (f[0], f[1], f[2], f[3]);
            if (mm == 0 || mf == 0) && (em == 0 || ef == 0) {
                continue;
            }
            let Some(row) = px.get_mut(y * w..(y + 1) * w) else {
                break;
            };
            for (x, p) in row.iter_mut().enumerate() {
                let on_top = layer.is_none_or(|(l, lw)| l.get(y * lw + x0 + x) == Some(&1));
                if em != 0 && ef > 0 && on_top {
                    *p = map_rgb(*p, |c| fade_channel(c, em, f32::from(ef) / 16.0));
                }
                if mm != 0 && mf > 0 {
                    *p = map_rgb(*p, |c| fade_channel(c, mm, f32::from(mf) / 16.0));
                }
            }
        }
        for (y, row) in self.whole {
            px[y * w..(y + 1) * w].copy_from_slice(&row);
        }
    }
}

type V3 = [f32; 3];

fn add(a: V3, b: V3) -> V3 {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}
fn sub(a: V3, b: V3) -> V3 {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
fn mul(a: V3, b: V3) -> V3 {
    [a[0] * b[0], a[1] * b[1], a[2] * b[2]]
}
fn scale(a: V3, s: f32) -> V3 {
    [a[0] * s, a[1] * s, a[2] * s]
}
fn dot(a: V3, b: V3) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
fn cross(a: V3, b: V3) -> V3 {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}
fn normalize(a: V3) -> V3 {
    let l = dot(a, a).sqrt();
    if l > 1e-9 {
        scale(a, 1.0 / l)
    } else {
        [0.0, 0.0, 1.0]
    }
}
fn mix(a: V3, b: V3, t: f32) -> V3 {
    add(a, scale(sub(b, a), t))
}
fn lum(c: V3) -> f32 {
    0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2]
}

/// The look of one place at one time of day.
#[derive(Debug, Clone, Copy)]
struct Preset {
    ambient: V3,
    key: V3,
    /// Toward the key light, in view space (x right, y up, z toward the camera).
    key_dir: V3,
    /// Ambient occlusion's strength.
    ao: f32,
    /// Point lights from bright pixels (0: none).
    points: f32,
    /// A light above the player (colour times strength; zero: none), how high above them and
    /// its reach: the room's light indoors (high and wide, so the room is lit around them
    /// rather than a pool at their feet), a lantern's in caves, a faint one at night.
    player_light: V3,
    player_height: f32,
    player_range: f32,
    grade: Grade,
}

#[derive(Debug, Clone, Copy)]
struct Grade {
    tint: V3,
    /// Shadows' tint (blended out as they get lighter).
    shadow: V3,
    /// Highlights' tint (blended in as they get lighter).
    highlight: V3,
    saturation: f32,
    exposure: f32,
    vignette: f32,
}

fn phase_grade(phase: Phase) -> Grade {
    match phase {
        Phase::Morning => Grade {
            tint: [1.03, 0.995, 0.98],
            shadow: [0.95, 0.96, 1.07],
            highlight: [1.05, 1.0, 0.95],
            saturation: 1.07,
            exposure: 1.0,
            vignette: 0.18,
        },
        Phase::Day => Grade {
            tint: [1.01, 1.0, 0.99],
            shadow: [0.93, 0.97, 1.08],
            highlight: [1.06, 1.01, 0.93],
            saturation: 1.1,
            exposure: 1.0,
            vignette: 0.16,
        },
        Phase::Evening => Grade {
            // The game's own evening is already orange: little more.
            tint: [1.01, 0.995, 0.985],
            shadow: [0.95, 0.96, 1.06],
            highlight: [1.02, 1.0, 0.98],
            saturation: 0.97,
            exposure: 0.98,
            vignette: 0.2,
        },
        Phase::Night => Grade {
            tint: [0.93, 0.96, 1.08],
            shadow: [0.90, 0.94, 1.14],
            highlight: [1.04, 1.0, 0.94],
            saturation: 1.02,
            exposure: 0.98,
            vignette: 0.30,
        },
    }
}

fn preset(place: Place, phase: Phase) -> Preset {
    let sun = normalize([-0.5, 0.7, 0.5]);
    let outdoor = Preset {
        ambient: scale([0.76, 0.84, 1.0], 0.66),
        key: scale([1.0, 0.91, 0.78], 0.74),
        key_dir: sun,
        ao: 1.3,
        points: 0.0,
        player_light: [0.0; 3],
        player_height: 0.0,
        player_range: 1.0,
        grade: phase_grade(phase),
    };
    match place {
        Place::Outdoor => match phase {
            Phase::Morning => Preset {
                ambient: scale([0.82, 0.80, 0.96], 0.74),
                key: scale([1.0, 0.86, 0.74], 0.58),
                ..outdoor
            },
            Phase::Day => outdoor,
            // Half way between day and night. The game doesn't light its windows until night, so
            // the light is the low sun's: golden, stronger against a cooler, dimmer sky than by
            // day, so sunlit sides stand out; bloom between the day's and the night's; a faint warm
            // light around the player; lamps the game does light still count as light sources.
            Phase::Evening => Preset {
                ambient: scale([0.70, 0.72, 0.96], 0.62),
                key: scale([1.0, 0.78, 0.52], 0.78),
                key_dir: normalize([-0.8, 0.35, 0.48]),
                points: 0.8,
                player_light: scale([1.0, 0.84, 0.62], 0.16),
                player_height: 40.0,
                player_range: 110.0,
                ..outdoor
            },
            Phase::Night => Preset {
                ambient: scale([0.62, 0.72, 1.0], 0.82),
                key: scale([0.62, 0.74, 1.0], 0.26),
                key_dir: normalize([0.3, 0.8, 0.5]),
                ao: 1.3,
                points: 1.3,
                player_light: scale([1.0, 0.86, 0.66], 0.30),
                player_height: 40.0,
                player_range: 110.0,
                ..outdoor
            },
        },
        Place::Indoor => {
            // Rooms look the same at any hour in the game; here they're darker at night.
            let dim = match phase {
                Phase::Night => 0.76,
                Phase::Evening => 0.9,
                _ => 1.0,
            };
            Preset {
                ambient: scale([0.86, 0.80, 0.74], 0.52 * dim),
                key: scale([1.0, 0.92, 0.80], 0.22),
                key_dir: normalize([-0.2, 0.8, 0.55]),
                ao: 1.9,
                points: 0.0,
                player_light: scale([1.0, 0.82, 0.58], 0.85),
                player_height: 70.0,
                player_range: 210.0,
                grade: Grade {
                    tint: [1.05, 1.0, 0.93],
                    shadow: [0.94, 0.94, 1.08],
                    highlight: [1.05, 1.0, 0.92],
                    saturation: 1.08,
                    exposure: 1.0,
                    vignette: 0.34,
                },
            }
        }
        Place::Cave => Preset {
            ambient: scale([0.62, 0.68, 0.90], 0.62),
            key: scale([0.86, 0.86, 1.0], 0.24),
            key_dir: normalize([0.0, 0.85, 0.5]),
            ao: 1.5,
            points: 0.0,
            player_light: scale([1.0, 0.84, 0.62], 0.75),
            player_height: 60.0,
            player_range: 190.0,
            grade: Grade {
                tint: [0.97, 0.98, 1.04],
                shadow: [0.92, 0.95, 1.10],
                highlight: [1.04, 1.0, 0.94],
                saturation: 1.0,
                exposure: 1.0,
                vignette: 0.34,
            },
        },
    }
}

/// Battles and menus: a small share of the time of day's grade.
fn small_grade(phase: Phase) -> Grade {
    let g = phase_grade(phase);
    let t = 0.4;
    Grade {
        tint: mix([1.0; 3], g.tint, t),
        shadow: mix([1.0; 3], g.shadow, t),
        highlight: mix([1.0; 3], g.highlight, t),
        saturation: 1.0 + (g.saturation - 1.0) * t + 0.03,
        exposure: 1.0 + (g.exposure - 1.0) * t,
        vignette: 0.06,
    }
}

/// The point lights nearest the player with the finer shadow maps (UC13).
const MAX_FINE_LIGHTS: usize = 3;
/// A window light's reach.
const LIGHT_RANGE: f32 = 90.0;
/// At most this many point lights (the biggest bright patches).
const MAX_LIGHTS: usize = 16;
/// Neighbours further apart than this in depth (one pixel apart) are different surfaces.
const EDGE: f32 = 12.0;
/// How much the S-curve deepens contrast.
const CONTRAST: f32 = 0.10;
/// 4096 steps of linear light to an sRGB byte, with the contrast curve.
const SRGB_STEPS: usize = 4096;

#[derive(Debug, Clone, Copy)]
struct PointLight {
    pos: V3,
    colour: V3,
    range: f32,
}

/// A light source followed from frame to frame, so it fades in when it appears (at the screen's
/// edge, from behind the player) and out when it goes, instead of popping.
#[derive(Clone, Copy)]
struct Tracked {
    light: PointLight,
    /// 0..1: how much of it shows.
    weight: f32,
}

/// Per frame: how fast a light source fades in (about a fifth of a second) and out (a quarter).
const FADE_IN: f32 = 1.0 / 12.0;
const FADE_OUT: f32 = 1.0 / 15.0;
/// The furthest a light source moves (view space) between two frames and is still the same one:
/// more than a step on the bicycle.
const TRACK_REACH: f32 = 32.0;

/// One pixel's colour (linear), position (view space) and what it shows.
#[derive(Debug, Clone, Copy, Default)]
struct Cell {
    base: V3,
    pos: V3,
    /// 1: the 3D scene; 0: a 2D layer shows (left as it is).
    layer: u8,
    /// A light source (a lit window, a lamp, a sign).
    emissive: bool,
    /// How much of one, 0..1: shown as its own colour that much, lit the rest (a pixel near the
    /// threshold eases between the two instead of flipping).
    glow: f32,
}

/// The lighting's working buffers and tables, kept between frames.
pub struct Lighting {
    to_linear: [f32; 256],
    to_srgb: Vec<u8>,
    ao_taps: Vec<(isize, isize)>,
    pool: Option<rayon::ThreadPool>,
    cells: Vec<Cell>,
    /// Dense copies of the cells' positions and layers, for the neighbourhood passes.
    pos: Vec<V3>,
    layer: Vec<u8>,
    nrm: Vec<V3>,
    tmp3: Vec<V3>,
    label: Vec<u32>,
    stack: Vec<u32>,
    /// The vignette for (width, height, strength), per pixel.
    vignette: (usize, usize, f32, Vec<f32>),
    /// Where the player stood last frame (view space), for [`Lighting::focus`].
    focus: Option<V3>,
    /// The light sources being followed, and when they were last updated (a pause, a battle or
    /// a menu, starts them afresh).
    tracked: Vec<Tracked>,
    tracked_at: Option<std::time::Instant>,
    /// The cast shadows' casters and maps (UC13).
    shadow: shadow::Shadows,
    /// The picture's mean brightness, followed slowly (the light sources' threshold): taken
    /// frame by frame it changed as the view moved, and window panes near it flickered between
    /// lit and glowing (the user's play test, walking up past houses).
    mean: Option<f32>,
    /// The GPU that lights the overworld (UC14); None: no usable GPU.
    gpu: Option<gpu::Gpu>,
    gpu_out: Vec<u32>,
    /// The last overworld picture's lights, for the player's picture put together with it.
    light_maps: LightMaps,
}

/// Each pixel's light in the overworld picture lit last (UC14), for putting the player's picture
/// together with the world's on the window's GPU, each source lit as itself: per pixel, the light
/// as the world (half floats: red, green; blue, glow) and as the player (red, green; blue, 0),
/// multiplied with the pixel's linear colour, then the grade (`grade`: tint and saturation, the
/// shadows' tint and exposure, the highlights' tint and the vignette's strength).
#[derive(Debug, Clone, Default)]
pub struct LightMaps {
    pub width: usize,
    pub data: Vec<[u32; 4]>,
    pub grade: [f32; 12],
    fresh: bool,
}

impl Default for Lighting {
    fn default() -> Self {
        Self::new()
    }
}

/// Runs `f(y, row)` over each `w`-wide row of `out`, on the pool's threads (or this one).
fn par_rows<T: Send>(
    pool: Option<&rayon::ThreadPool>,
    out: &mut [T],
    w: usize,
    f: impl Fn(usize, &mut [T]) + Sync,
) {
    use rayon::prelude::*;
    match pool {
        Some(pool) => pool.install(|| {
            out.par_chunks_mut(w)
                .enumerate()
                .with_min_len(8)
                .for_each(|(y, row)| f(y, row))
        }),
        None => out.chunks_mut(w).enumerate().for_each(|(y, row)| f(y, row)),
    }
}

/// A pool of `threads` workers, or none for one (the caller's thread).
fn make_pool(threads: usize) -> Option<rayon::ThreadPool> {
    (threads > 1)
        .then(|| {
            rayon::ThreadPoolBuilder::new()
                .num_threads(threads)
                .thread_name(|i| format!("goldlink-light-{i}"))
                .build()
                .ok()
        })
        .flatten()
}

fn linear(table: &[f32; 256], p: u32) -> V3 {
    [
        table[(p >> 16 & 0xFF) as usize],
        table[(p >> 8 & 0xFF) as usize],
        table[(p & 0xFF) as usize],
    ]
}

fn pack(table: &[u8], c: V3) -> u32 {
    let q = |v: f32| table[(v.clamp(0.0, 1.0) * (SRGB_STEPS - 1) as f32 + 0.5) as usize] as u32;
    q(c[0]) << 16 | q(c[1]) << 8 | q(c[2])
}

impl Lighting {
    pub fn new() -> Self {
        let mut to_linear = [0f32; 256];
        for (i, v) in to_linear.iter_mut().enumerate() {
            let c = i as f32 / 255.0;
            *v = if c <= 0.04045 {
                c / 12.92
            } else {
                ((c + 0.055) / 1.055).powf(2.4)
            };
        }
        let to_srgb = (0..SRGB_STEPS)
            .map(|i| {
                let l = i as f32 / (SRGB_STEPS - 1) as f32;
                let s = if l <= 0.003_130_8 {
                    l * 12.92
                } else {
                    1.055 * l.powf(1.0 / 2.4) - 0.055
                };
                let curve = s * s * (3.0 - 2.0 * s);
                let s = s + CONTRAST * (curve - s);
                (s.clamp(0.0, 1.0) * 255.0 + 0.5) as u8
            })
            .collect();
        // A spiral of taps 2..7 pixels out (golden angle), for the occlusion.
        let ao_taps = (0..8)
            .map(|k| {
                let r = 1.8 + 5.4 * (k as f32 + 0.5) / 8.0;
                let a = k as f32 * 2.399_963;
                (
                    (a.cos() * r).round() as isize,
                    (a.sin() * r).round() as isize,
                )
            })
            .collect();
        // The emulation and the window have a core each; up to eight more share this (the cast
        // shadows' maps are drawn in bands across them: 8 rather than 4 took the lighting from
        // 8.4 to 6.8 ms a frame, SAVE02 by a morning sun).
        let threads = std::thread::available_parallelism()
            .map_or(1, |n| n.get().saturating_sub(2).clamp(1, 8));
        let pool = make_pool(threads);
        Self {
            to_linear,
            to_srgb,
            ao_taps,
            pool,
            cells: Vec::new(),
            pos: Vec::new(),
            layer: Vec::new(),
            nrm: Vec::new(),
            tmp3: Vec::new(),
            label: Vec::new(),
            stack: Vec::new(),
            vignette: (0, 0, 0.0, Vec::new()),
            focus: None,
            tracked: Vec::new(),
            tracked_at: None,
            mean: None,
            shadow: shadow::Shadows::default(),
            gpu: None,
            gpu_out: Vec::new(),
            light_maps: LightMaps::default(),
        }
    }

    fn vignette_table(&mut self, w: usize, h: usize, strength: f32) {
        let v = &mut self.vignette;
        if v.0 != w || v.1 != h || v.2 != strength {
            *v = (
                w,
                h,
                strength,
                (0..w * h)
                    .map(|i| vignette(i % w, i / w, w, h, strength))
                    .collect(),
            );
        }
    }

    /// Lights the overworld on the GPU (UC14), a device of the lighting's own; false when there's
    /// no usable one (the lighting is then off).
    pub fn use_gpu(&mut self) -> bool {
        self.gpu = gpu::Gpu::new();
        self.gpu.is_some()
    }

    /// How many threads share the work (tests and timing).
    pub fn set_threads(&mut self, threads: usize) {
        self.pool = make_pool(threads);
    }

    /// Battles, menus and the touch screen: the small grade over `w` x `h` pixels (0x00RRGGBB).
    pub fn grade(&mut self, px: &mut [u32], w: usize, h: usize, phase: Phase) {
        let g = small_grade(phase);
        self.vignette_table(w, h, g.vignette);
        let (lin, srgb, vig) = (&self.to_linear, &self.to_srgb, &self.vignette.3);
        par_rows(self.pool.as_ref(), &mut px[..w * h], w, |y, row| {
            for (x, p) in row.iter_mut().enumerate() {
                let v = vig[y * w + x];
                *p = pack(srgb, apply_grade(linear(lin, *p), &g, v));
            }
        });
    }

    /// The overworld: lights `px` (`w` x 192, 0x00RRGGBB), whose column 0 is the scene's
    /// column `x0`. Pixels the scene marks as 2D only are left as they are. False when there's no
    /// GPU to light it on (UC14: the picture is left unlit).
    pub fn overworld(
        &mut self,
        px: &mut [u32],
        w: usize,
        x0: usize,
        scene: &Scene,
        place: Place,
        phase: Phase,
    ) -> bool {
        if !self.has_gpu() {
            return false;
        }
        // The game's fades come off first and go back on last (the scene is lit, not the fade).
        let layer = Some((scene.layer, scene.width));
        let faded = Faded::lift(px, w, x0, scene.fade, layer);
        self.overworld_scene(px, w, x0, scene, place, phase);
        faded.put_back(px, w, x0, scene.fade, layer);
        true
    }

    /// The lights of the overworld picture lit since the last call, once (none when it wasn't).
    pub fn take_light_maps(&mut self) -> Option<LightMaps> {
        if !self.light_maps.fresh {
            return None;
        }
        self.light_maps.fresh = false;
        Some(self.light_maps.clone())
    }

    /// Whether the overworld is lit on a GPU (UC14); without one it's left unlit.
    pub fn has_gpu(&self) -> bool {
        self.gpu.as_ref().is_some_and(|g| !g.failed())
    }

    /// Battles and menus with the game's fades known (`fade`, 192 lines): the small grade on the
    /// picture under them, the fades applied again after.
    pub fn grade_faded(&mut self, px: &mut [u32], w: usize, phase: Phase, fade: &[Fade]) {
        let faded = Faded::lift(px, w, 0, fade, None);
        self.grade(px, w, HEIGHT, phase);
        faded.put_back(px, w, 0, fade, None);
    }

    fn overworld_scene(
        &mut self,
        px: &mut [u32],
        w: usize,
        x0: usize,
        scene: &Scene,
        place: Place,
        phase: Phase,
    ) {
        let h = HEIGHT;
        let n = w * h;
        if w == 0
            || px.len() < n
            || x0 + w > scene.width
            || scene.depth.len() < scene.width * h
            || scene.layer.len() < scene.width * h
        {
            return;
        }
        let Some(rows) = unprojection(&scene.proj) else {
            return self.grade(px, w, h, phase);
        };
        let mut p = preset(place, phase);
        self.cells.resize(n, Cell::default());
        for v in [&mut self.nrm, &mut self.tmp3] {
            v.resize(n, [0.0; 3]);
        }
        self.label.resize(n, 0);

        // 1. Colour, position, which pixels are the 3D scene.
        {
            let lin = &self.to_linear;
            let px = &px[..n];
            par_rows(self.pool.as_ref(), &mut self.cells, w, |y, row| {
                let yn = 1.0 - (y as f32 + 0.5) / (h as f32 / 2.0);
                for (x, cell) in row.iter_mut().enumerate() {
                    let col = x0 + x;
                    let g = y * scene.width + col;
                    cell.base = linear(lin, px[y * w + x]);
                    cell.emissive = false;
                    cell.glow = 0.0;
                    cell.layer = 0;
                    // Only where the 3D scene itself shows: a text box, a sign's banner or a menu
                    // over it stays as the game drew it.
                    let layer = scene.layer[g];
                    if layer != 1 {
                        continue;
                    }
                    let xn = (col as f32 + 0.5 - scene.width as f32 / 2.0) / 128.0;
                    let zn = scene.depth[g] as f32 / 8_388_607.5 - 1.0;
                    if let Some(pos) = unproject(&rows, xn, yn, zn) {
                        cell.pos = pos;
                        cell.layer = layer;
                    }
                }
            });
        }
        if self.cells.iter().all(|c| c.layer == 0) {
            return self.grade(px, w, h, phase);
        }
        self.pos.clear();
        self.pos.extend(self.cells.iter().map(|c| c.pos));
        self.layer.clear();
        self.layer.extend(self.cells.iter().map(|c| c.layer));

        // Cast shadows (UC13): the picture's polygons as casters; outdoors the sun by day, the
        // moon by night: the key light comes from where it stands, and what stands in its way
        // casts a shadow.
        let mut sun_strength = 0.0;
        self.shadow.ready = false;
        if place == Place::Outdoor && !scene.geometry.polys.is_empty() {
            let up = shadow::Shadows::find_up(&scene.geometry, &rows);
            let fwd: [[f32; 4]; 4] = std::array::from_fn(|r| {
                std::array::from_fn(|c| scene.proj[r * 4 + c] as f32 / 4096.0)
            });
            // What the picture shows at a place (the scene's columns, rows): the ground under a
            // sprite's feet.
            let cells = &self.cells;
            let seen = |col: f32, row: f32| -> Option<V3> {
                let (x, y) = (col.floor() as isize - x0 as isize, row.floor() as isize);
                if x < 0 || y < 0 || x as usize >= w || y as usize >= h {
                    return None;
                }
                let c = cells[y as usize * w + x as usize];
                (c.layer != 0).then_some(c.pos)
            };
            self.shadow.prepare(&scene.geometry, &fwd, &rows, up, &seen);
            if let Some((azimuth, elevation, strength)) = shadow::sun(scene.hours) {
                let dir = shadow::Shadows::sun_dir(up, azimuth, elevation);
                self.shadow.build_sun(dir, shadow::TEXELS);
                if self.shadow.has_sun() {
                    p.key_dir = dir;
                    sun_strength = strength;
                }
            } else if let Some((azimuth, elevation, _)) = shadow::moon(scene.hours) {
                // By night the moon's light comes from where it stands, but casts no shadows of
                // its own: faint beside the lit windows' and lamps', and its map cost a fifth of
                // the night's lighting.
                p.key_dir = shadow::Shadows::sun_dir(up, azimuth, elevation);
            }
        }
        // The normals, occlusion and light are the GPU's; finding the light sources needs the
        // normals here too.
        if p.points > 0.0 {
            self.normals(w, h);
        }
        let found = if p.points > 0.0 {
            self.find_lights(w, h, p.points)
        } else {
            Vec::new()
        };
        let mut lights = self.track_lights(&found);
        let focus = self.focus(w, h);
        if let Some(f) = focus
            && lum(p.player_light) > 0.0
        {
            // Above the player, a little toward the camera.
            lights.push(PointLight {
                pos: add(f, [0.0, p.player_height, 12.0]),
                colour: p.player_light,
                range: p.player_range,
            });
        }
        // The lights that cast shadows (UC13), outdoors: the lit windows and lamps nearest the
        // player (at most MAX_SHADOW_LIGHTS). Indoors and in caves the game's own shadows stay
        // (the user, 2026-11-14: "indoor shadows should remain vanilla"). `slots[i]`: light i's
        // shadow maps, if it has them.
        let mut slots = vec![None; lights.len()];
        if self.shadow.ready && place == Place::Outdoor {
            let player = (lum(p.player_light) > 0.0 && focus.is_some()).then(|| lights.len() - 1);
            let near = focus.unwrap_or([0.0, 0.0, -600.0]);
            // Every light that shines casts shadows: with only the nearest the player casting, the
            // others lit what they couldn't reach (the roof over a lit window, past its eaves),
            // and which ones did changed as the player walked (the user's look, Violet City).
            // The nearest the player get finer maps.
            let mut chosen: Vec<usize> = (0..lights.len())
                .filter(|&i| Some(i) != player && lum(lights[i].colour) > 0.0)
                .collect();
            chosen.sort_by(|&a, &b| {
                let da = sub(lights[a].pos, near);
                let db = sub(lights[b].pos, near);
                dot(da, da).total_cmp(&dot(db, db))
            });
            let at: Vec<(V3, f32, usize)> = chosen
                .iter()
                .enumerate()
                .map(|(k, &i)| {
                    let face = if k < MAX_FINE_LIGHTS {
                        shadow::FACE
                    } else {
                        shadow::FACE_COARSE
                    };
                    (lights[i].pos, lights[i].range, face)
                })
                .collect();
            // Every third pixel's place, for which faces of a light's cube anything looks out
            // through.
            let shadows = &self.shadow;
            let receivers: Vec<V3> = self
                .cells
                .iter()
                .step_by(3)
                .filter(|c| c.layer != 0)
                .filter_map(|c| shadows.place(c.pos))
                .collect();
            self.shadow
                .build_points(&at, &receivers, self.pool.as_ref());
            for (slot, &i) in chosen.iter().enumerate() {
                slots[i] = Some(slot);
            }
        }

        // 3 to 5 on the GPU (UC14): the light, the cast shadows, the grade.
        self.light_on_gpu(px, w, h, x0, scene, &p, sun_strength, &lights, &slots);
    }

    /// Steps 3 to 5 on the GPU (UC14): the light, the cast shadows, the grade. Unlit when the GPU
    /// fails.
    #[allow(clippy::too_many_arguments)]
    fn light_on_gpu(
        &mut self,
        px: &mut [u32],
        w: usize,
        h: usize,
        x0: usize,
        scene: &Scene,
        p: &Preset,
        sun_strength: f32,
        lights: &[PointLight],
        slots: &[Option<usize>],
    ) {
        let n = w * h;
        let pixels: Vec<gpu::Pixel> = (0..n)
            .map(|i| {
                let c = self.cells[i];
                let (x, y) = (i % w, i / w);
                let player = scene.attached.get(y * scene.width + x0 + x) == Some(&1);
                gpu::Pixel {
                    pos: [
                        c.pos[0],
                        c.pos[1],
                        c.pos[2],
                        f32::from(u8::from(c.layer != 0)),
                    ],
                    info: [px[i], c.glow.to_bits(), u32::from(player), 0],
                }
            })
            .collect();
        let glights: Vec<gpu::Light> = lights
            .iter()
            .zip(slots)
            .map(|(l, s)| gpu::Light {
                pos: [l.pos[0], l.pos[1], l.pos[2], l.range],
                colour: [
                    l.colour[0],
                    l.colour[1],
                    l.colour[2],
                    s.map_or(-1.0, |s| s as f32),
                ],
            })
            .collect();
        let v4 = |v: V3, w: f32| [v[0], v[1], v[2], w];
        let g = &p.grade;
        let mut taps = [[0i32; 4]; 8];
        for (t, &(dx, dy)) in taps.iter_mut().zip(&self.ao_taps) {
            *t = [dx as i32, dy as i32, 0, 0];
        }
        let frame = gpu::Frame {
            dims: [w as u32, h as u32, x0 as u32, shadow::PICTURE_WIDTH as u32],
            counts: [glights.len() as u32, 0, 0, 0],
            params: [
                sun_strength,
                lum(p.ambient) + lum(p.key) * 0.85 + lum(p.player_light) * 0.45,
                1.6 * p.ao / self.ao_taps.len() as f32,
                g.vignette,
            ],
            key_dir: v4(p.key_dir, 0.0),
            ambient: v4(p.ambient, 0.0),
            key: v4(p.key, 0.0),
            tint: v4(g.tint, g.saturation),
            shadow: v4(g.shadow, g.exposure),
            high: v4(g.highlight, 0.0),
            taps,
            ..Default::default()
        };
        self.gpu_out.resize(n, 0);
        let input = gpu::Input {
            frame,
            pixels: &pixels,
            lights: &glights,
            shadows: &self.shadow,
        };
        let Some(gpu) = self.gpu.as_mut() else {
            return;
        };
        if gpu.run(&input, &mut self.gpu_out, &mut self.light_maps.data) {
            px[..n].copy_from_slice(&self.gpu_out[..n]);
            self.light_maps.width = w;
            self.light_maps.grade = [
                g.tint[0],
                g.tint[1],
                g.tint[2],
                g.saturation,
                g.shadow[0],
                g.shadow[1],
                g.shadow[2],
                g.exposure,
                g.highlight[0],
                g.highlight[1],
                g.highlight[2],
                g.vignette,
            ];
            self.light_maps.fresh = true;
        }
    }

    /// Normals from neighbouring positions: per axis, the neighbour nearer in depth (an edge
    /// takes its own surface's side); none across an edge (then the pixel faces the camera).
    /// Then averaged with the neighbours on the same surface.
    fn normals(&mut self, w: usize, h: usize) {
        let (pos, layer) = (&self.pos, &self.layer);
        // The nearer of two one-sided differences (each present when its neighbour is 3D).
        let pick = |fwd: Option<V3>, bwd: Option<V3>| -> Option<V3> {
            let best = match (fwd, bwd) {
                (Some(a), Some(b)) => {
                    if a[2].abs() <= b[2].abs() {
                        a
                    } else {
                        b
                    }
                }
                (Some(a), None) | (None, Some(a)) => a,
                (None, None) => return None,
            };
            (best[2].abs() < EDGE).then_some(best)
        };
        par_rows(self.pool.as_ref(), &mut self.tmp3, w, |y, row| {
            for (x, out) in row.iter_mut().enumerate() {
                let i = y * w + x;
                *out = [0.0, 0.0, 1.0];
                if layer[i] == 0 {
                    continue;
                }
                let c = pos[i];
                let right = (x + 1 < w && layer[i + 1] != 0).then(|| sub(pos[i + 1], c));
                let left = (x > 0 && layer[i - 1] != 0).then(|| sub(c, pos[i - 1]));
                // Up the screen, so the cross product faces the camera.
                let up = (y > 0 && layer[i - w] != 0).then(|| sub(pos[i - w], c));
                let down = (y + 1 < h && layer[i + w] != 0).then(|| sub(c, pos[i + w]));
                if let (Some(a), Some(b)) = (pick(right, left), pick(up, down)) {
                    let n = normalize(cross(a, b));
                    *out = if dot(n, c) > 0.0 { scale(n, -1.0) } else { n };
                }
            }
        });
        let raw = &self.tmp3;
        par_rows(self.pool.as_ref(), &mut self.nrm, w, |y, row| {
            let (y0, y1) = (y.saturating_sub(1), (y + 1).min(h - 1));
            for (x, out) in row.iter_mut().enumerate() {
                let i = y * w + x;
                if layer[i] == 0 {
                    *out = [0.0, 0.0, 1.0];
                    continue;
                }
                let z = pos[i][2];
                let (x0, x1) = (x.saturating_sub(1), (x + 1).min(w - 1));
                let mut sum = [0.0f32; 3];
                for yy in y0..=y1 {
                    for j in yy * w + x0..=yy * w + x1 {
                        if layer[j] != 0 && (pos[j][2] - z).abs() < 3.0 {
                            sum = add(sum, raw[j]);
                        }
                    }
                }
                *out = normalize(sum);
            }
        });
    }

    /// The light sources: pixels far brighter than the scene (lit windows, lamps, signs) that
    /// are warm or very bright (the night's ordinary pale surfaces lean blue). Each patch of
    /// them becomes a point light, placed in 3D just off its surface.
    fn find_lights(&mut self, w: usize, h: usize, strength: f32) -> Vec<(u32, PointLight)> {
        let n = w * h;
        let (mut total, mut count) = (0.0f32, 0u32);
        for c in &self.cells[..n] {
            if c.layer != 0 {
                total += lum(c.base);
                count += 1;
            }
        }
        let now_mean = total / count.max(1) as f32;
        // Followed over about a second; afresh after a pause (as the tracked lights are).
        let fresh = self
            .tracked_at
            .is_none_or(|t| t.elapsed() > std::time::Duration::from_millis(500));
        let mean = match self.mean {
            Some(m) if !fresh => m + (now_mean - m) * 0.02,
            _ => now_mean,
        };
        self.mean = Some(mean);
        let threshold = (mean * 2.2).max(0.25);
        let ease = |l: f32, at: f32| ((l / at - 0.9) / 0.2).clamp(0.0, 1.0);
        for c in &mut self.cells[..n] {
            let b = c.base;
            let l = lum(b);
            let warm = b[0] >= b[2] * 1.15;
            c.glow = if c.layer == 1 && b[0].max(b[1]).max(b[2]) > 0.45 {
                let bright = ease(l, threshold);
                let s = if warm { 1.0 } else { ease(l, threshold * 1.8) };
                let g = bright * s;
                g * g * (3.0 - 2.0 * g)
            } else {
                0.0
            };
            c.emissive = c.glow > 0.5;
        }
        // Patches: 4-connected runs of light pixels.
        self.label[..n].fill(0);
        struct Patch {
            id: u32,
            size: u32,
            pos: V3,
            nrm: V3,
            colour: V3,
        }
        let mut patches: Vec<Patch> = Vec::new();
        for start in 0..n {
            if !self.cells[start].emissive || self.label[start] != 0 {
                continue;
            }
            let id = patches.len() as u32 + 1;
            let mut patch = Patch {
                id,
                size: 0,
                pos: [0.0; 3],
                nrm: [0.0; 3],
                colour: [0.0; 3],
            };
            self.stack.clear();
            self.stack.push(start as u32);
            self.label[start] = id;
            while let Some(i) = self.stack.pop() {
                let i = i as usize;
                let c = self.cells[i];
                patch.size += 1;
                patch.pos = add(patch.pos, c.pos);
                patch.nrm = add(patch.nrm, self.nrm[i]);
                patch.colour = add(patch.colour, c.base);
                let (x, y) = (i % w, i / w);
                let next = [
                    (x > 0).then(|| i - 1),
                    (x + 1 < w).then(|| i + 1),
                    (y > 0).then(|| i - w),
                    (y + 1 < h).then(|| i + w),
                ];
                for j in next.into_iter().flatten() {
                    if self.cells[j].emissive && self.label[j] == 0 {
                        self.label[j] = id;
                        self.stack.push(j as u32);
                    }
                }
            }
            patches.push(patch);
        }
        patches.retain(|p| p.size >= 3);
        patches.sort_by_key(|p| std::cmp::Reverse(p.size));
        patches.truncate(MAX_LIGHTS);
        patches
            .iter()
            .map(|p| {
                let k = 1.0 / p.size as f32;
                let colour = scale(p.colour, k);
                let peak = colour[0].max(colour[1]).max(colour[2]).max(1e-3);
                let colour = mix(scale(colour, 1.0 / peak), [1.0, 0.85, 0.65], 0.3);
                let power = strength * (p.size as f32 / 20.0).sqrt().min(1.5);
                let light = PointLight {
                    pos: add(scale(p.pos, k), scale(normalize(p.nrm), 6.0)),
                    colour: scale(colour, power),
                    range: LIGHT_RANGE,
                };
                (p.id, light)
            })
            .collect()
    }

    /// Follows the light sources found this frame (`(patch, light)`) from the last ones: each
    /// fades in over [`FADE_IN`] frames, and one no longer found (covered by the player, past
    /// the screen's edge) fades out where it was. Gives the lights to shade with, each as strong
    /// as it shows.
    fn track_lights(&mut self, found: &[(u32, PointLight)]) -> Vec<PointLight> {
        let now = std::time::Instant::now();
        if self
            .tracked_at
            .is_none_or(|t| now.duration_since(t) > std::time::Duration::from_millis(500))
        {
            self.tracked.clear();
        }
        self.tracked_at = Some(now);
        let mut seen = vec![false; self.tracked.len()];
        let mut next = Vec::with_capacity(found.len() + self.tracked.len());
        for &(_, light) in found {
            let near = self
                .tracked
                .iter()
                .enumerate()
                .filter(|(i, _)| !seen[*i])
                .map(|(i, t)| {
                    let d = sub(t.light.pos, light.pos);
                    (i, dot(d, d))
                })
                .filter(|&(_, d2)| d2 < TRACK_REACH * TRACK_REACH)
                .min_by(|a, b| a.1.total_cmp(&b.1));
            let weight = match near {
                Some((i, _)) => {
                    seen[i] = true;
                    (self.tracked[i].weight + FADE_IN).min(1.0)
                }
                None => FADE_IN,
            };
            next.push(Tracked { light, weight });
        }
        for (t, seen) in self.tracked.iter().zip(&seen) {
            if !seen && t.weight > FADE_OUT {
                next.push(Tracked {
                    light: t.light,
                    weight: t.weight - FADE_OUT,
                });
            }
        }
        next.truncate(2 * MAX_LIGHTS);
        self.tracked = next;
        self.tracked
            .iter()
            .map(|t| PointLight {
                colour: scale(t.light.colour, t.weight),
                ..t.light
            })
            .collect()
    }

    /// Where the player stands: the ground at the screen's middle (the camera follows them),
    /// eased from frame to frame. The ground, not the player's sprite: it's the furthest of
    /// what shows there, and doesn't change with the walking animation.
    fn focus(&mut self, w: usize, h: usize) -> Option<V3> {
        let (cx, cy) = (w / 2, h / 2 + 4);
        let window = || {
            (cy.saturating_sub(6)..(cy + 7).min(h))
                .flat_map(move |y| (cx.saturating_sub(6)..(cx + 7).min(w)).map(move |x| y * w + x))
        };
        let far = window()
            .filter(|&i| self.cells[i].layer != 0)
            .map(|i| self.cells[i].pos[2])
            .reduce(f32::min);
        let now = far.map(|far| {
            let (mut sum, mut cnt) = ([0.0f32; 3], 0.0f32);
            for i in window() {
                let c = self.cells[i];
                if c.layer != 0 && c.pos[2] < far + 6.0 {
                    sum = add(sum, c.pos);
                    cnt += 1.0;
                }
            }
            scale(sum, 1.0 / cnt)
        });
        // Eased toward where it is now; a jump (a new map, a warp) is taken at once.
        self.focus = match (self.focus, now) {
            (Some(was), Some(now)) => {
                let d = sub(now, was);
                Some(if dot(d, d) < 64.0 * 64.0 {
                    add(was, scale(d, 0.25))
                } else {
                    now
                })
            }
            (_, now) => now,
        };
        self.focus
    }
}

fn vignette(x: usize, y: usize, w: usize, h: usize, strength: f32) -> f32 {
    if strength <= 0.0 {
        return 1.0;
    }
    let dx = (x as f32 + 0.5 - w as f32 / 2.0) / (h as f32 * 0.75);
    let dy = (y as f32 + 0.5 - h as f32 / 2.0) / (h as f32 / 2.0);
    let t = ((dx * dx + dy * dy - 0.45) / 1.3).clamp(0.0, 1.0);
    1.0 - strength * t * t.sqrt()
}

fn apply_grade(c: V3, g: &Grade, vignette: f32) -> V3 {
    let c = scale(mul(c, g.tint), g.exposure);
    let l = lum(c);
    let c = add([l; 3], scale(sub(c, [l; 3]), g.saturation));
    let lift = (l * 4.0).min(1.0);
    let c = mul(c, mix(g.shadow, [1.0; 3], lift));
    let high = ((l - 0.35) / 0.65).clamp(0.0, 1.0);
    let c = mul(c, mix([1.0; 3], g.highlight, high));
    scale(c, vignette).map(|v| v.max(0.0))
}

/// The inverse of a DS matrix (20.12, row by row), or None when it has none.
fn invert(m: &[i32; 16]) -> Option<[f64; 16]> {
    let a: Vec<f64> = m.iter().map(|&v| v as f64 / 4096.0).collect();
    let mut inv = [0f64; 16];
    inv[0] = a[5] * a[10] * a[15] - a[5] * a[11] * a[14] - a[9] * a[6] * a[15]
        + a[9] * a[7] * a[14]
        + a[13] * a[6] * a[11]
        - a[13] * a[7] * a[10];
    inv[4] = -a[4] * a[10] * a[15] + a[4] * a[11] * a[14] + a[8] * a[6] * a[15]
        - a[8] * a[7] * a[14]
        - a[12] * a[6] * a[11]
        + a[12] * a[7] * a[10];
    inv[8] = a[4] * a[9] * a[15] - a[4] * a[11] * a[13] - a[8] * a[5] * a[15]
        + a[8] * a[7] * a[13]
        + a[12] * a[5] * a[11]
        - a[12] * a[7] * a[9];
    inv[12] = -a[4] * a[9] * a[14] + a[4] * a[10] * a[13] + a[8] * a[5] * a[14]
        - a[8] * a[6] * a[13]
        - a[12] * a[5] * a[10]
        + a[12] * a[6] * a[9];
    inv[1] = -a[1] * a[10] * a[15] + a[1] * a[11] * a[14] + a[9] * a[2] * a[15]
        - a[9] * a[3] * a[14]
        - a[13] * a[2] * a[11]
        + a[13] * a[3] * a[10];
    inv[5] = a[0] * a[10] * a[15] - a[0] * a[11] * a[14] - a[8] * a[2] * a[15]
        + a[8] * a[3] * a[14]
        + a[12] * a[2] * a[11]
        - a[12] * a[3] * a[10];
    inv[9] = -a[0] * a[9] * a[15] + a[0] * a[11] * a[13] + a[8] * a[1] * a[15]
        - a[8] * a[3] * a[13]
        - a[12] * a[1] * a[11]
        + a[12] * a[3] * a[9];
    inv[13] = a[0] * a[9] * a[14] - a[0] * a[10] * a[13] - a[8] * a[1] * a[14]
        + a[8] * a[2] * a[13]
        + a[12] * a[1] * a[10]
        - a[12] * a[2] * a[9];
    inv[2] = a[1] * a[6] * a[15] - a[1] * a[7] * a[14] - a[5] * a[2] * a[15]
        + a[5] * a[3] * a[14]
        + a[13] * a[2] * a[7]
        - a[13] * a[3] * a[6];
    inv[6] = -a[0] * a[6] * a[15] + a[0] * a[7] * a[14] + a[4] * a[2] * a[15]
        - a[4] * a[3] * a[14]
        - a[12] * a[2] * a[7]
        + a[12] * a[3] * a[6];
    inv[10] = a[0] * a[5] * a[15] - a[0] * a[7] * a[13] - a[4] * a[1] * a[15]
        + a[4] * a[3] * a[13]
        + a[12] * a[1] * a[7]
        - a[12] * a[3] * a[5];
    inv[14] = -a[0] * a[5] * a[14] + a[0] * a[6] * a[13] + a[4] * a[1] * a[14]
        - a[4] * a[2] * a[13]
        - a[12] * a[1] * a[6]
        + a[12] * a[2] * a[5];
    inv[3] = -a[1] * a[6] * a[11] + a[1] * a[7] * a[10] + a[5] * a[2] * a[11]
        - a[5] * a[3] * a[10]
        - a[9] * a[2] * a[7]
        + a[9] * a[3] * a[6];
    inv[7] = a[0] * a[6] * a[11] - a[0] * a[7] * a[10] - a[4] * a[2] * a[11]
        + a[4] * a[3] * a[10]
        + a[8] * a[2] * a[7]
        - a[8] * a[3] * a[6];
    inv[11] = -a[0] * a[5] * a[11] + a[0] * a[7] * a[9] + a[4] * a[1] * a[11]
        - a[4] * a[3] * a[9]
        - a[8] * a[1] * a[7]
        + a[8] * a[3] * a[5];
    inv[15] = a[0] * a[5] * a[10] - a[0] * a[6] * a[9] - a[4] * a[1] * a[10]
        + a[4] * a[2] * a[9]
        + a[8] * a[1] * a[6]
        - a[8] * a[2] * a[5];
    let det = a[0] * inv[0] + a[1] * inv[4] + a[2] * inv[8] + a[3] * inv[12];
    if det.abs() < 1e-12 || !det.is_finite() {
        return None;
    }
    Some(inv.map(|v| v / det))
}

/// The inverse projection's rows, for [`unproject`].
fn unprojection(proj: &[i32; 16]) -> Option<[[f32; 4]; 4]> {
    let inv = invert(proj)?;
    let mut rows = [[0f32; 4]; 4];
    for (r, row) in rows.iter_mut().enumerate() {
        for (c, v) in row.iter_mut().enumerate() {
            *v = inv[r * 4 + c] as f32;
        }
    }
    Some(rows)
}

/// A view-space position from normalised device coordinates (vectors on the left: the clip
/// position times the inverse), if it's in front of the camera.
fn unproject(rows: &[[f32; 4]; 4], xn: f32, yn: f32, zn: f32) -> Option<V3> {
    let mut r = [0f32; 4];
    for (c, out) in r.iter_mut().enumerate() {
        *out = xn * rows[0][c] + yn * rows[1][c] + zn * rows[2][c] + rows[3][c];
    }
    if r[3].abs() < 1e-9 {
        return None;
    }
    let p = [r[0] / r[3], r[1] / r[3], r[2] / r[3]];
    (p.iter().all(|v| v.is_finite()) && p[2] < 0.0).then_some(p)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The town camera the spike measured: vertical FOV ~16 degrees, near 150, far 1200.
    const PROJ: [i32; 16] = [
        21628, 0, 0, 0, 0, 28836, 0, 0, 0, 0, -5266, -4096, 0, 0, -1_404_343, 0,
    ];

    fn depth_of(z_view: f64) -> u32 {
        let a: Vec<f64> = PROJ.iter().map(|&v| v as f64 / 4096.0).collect();
        let zn = (a[10] * z_view + a[14]) / -z_view;
        ((zn + 1.0) * 8_388_607.5) as u32
    }

    #[test]
    fn phases_follow_the_games_table() {
        let got: Vec<Phase> = (0..24).map(Phase::of_hour).collect();
        assert_eq!(got[3], Phase::Night);
        assert_eq!(got[4], Phase::Morning);
        assert_eq!(got[9], Phase::Morning);
        assert_eq!(got[10], Phase::Day);
        assert_eq!(got[16], Phase::Day);
        assert_eq!(got[17], Phase::Evening);
        assert_eq!(got[19], Phase::Evening);
        assert_eq!(got[20], Phase::Night);
    }

    #[test]
    fn places_from_the_map_headers() {
        assert_eq!(Place::of_map(60), Place::Outdoor); // New Bark Town
        assert_eq!(Place::of_map(33), Place::Outdoor); // Route 29
        assert_eq!(Place::of_map(64), Place::Indoor); // the player's room
        assert_eq!(Place::of_map(99), Place::Cave); // Union Cave 1F
        assert_eq!(Place::of_map(9999), Place::Outdoor);
    }

    #[test]
    fn unprojecting_gives_back_the_depth() {
        let rows = unprojection(&PROJ).expect("invertible");
        for z in [-200.0f64, -500.0, -900.0] {
            let d = depth_of(z);
            let zn = d as f32 / 8_388_607.5 - 1.0;
            let p = unproject(&rows, 0.0, 0.0, zn).expect("in front");
            assert!((p[2] as f64 - z).abs() < 0.5, "{z}: got {}", p[2]);
        }
    }

    /// A tilted floor (the ground under the camera) with the text box's rows marked 2D.
    fn floor(width: usize) -> (Vec<u32>, Vec<u8>) {
        let mut depth = vec![0u32; width * HEIGHT];
        let mut layer = vec![1u8; width * HEIGHT];
        for y in 0..HEIGHT {
            // Further away toward the top of the screen.
            let z = -500.0 - (HEIGHT - y) as f64 * 1.5;
            for x in 0..width {
                depth[y * width + x] = depth_of(z);
                if y >= 150 {
                    layer[y * width + x] = 0;
                }
            }
        }
        (depth, layer)
    }

    #[test]
    fn the_text_box_is_left_alone_and_the_floor_is_lit() {
        let width = 384;
        let (depth, layer) = floor(width);
        let scene = Scene {
            depth: &depth,
            layer: &layer,
            proj: PROJ,
            fade: &[],
            width,
            geometry: Geometry::default(),
            hours: 12.0,
            attached: &[],
        };
        let mut px = vec![0x0080_8080u32; 256 * HEIGHT];
        let mut lighting = Lighting::new();
        if !lighting.use_gpu() {
            eprintln!("no GPU here: the overworld's lighting isn't tested");
            return;
        }
        lighting.overworld(&mut px, 256, 64, &scene, Place::Outdoor, Phase::Day);
        // The text box's rows: untouched.
        assert!(px[160 * 256..].iter().all(|&p| p == 0x0080_8080));
        // The floor: changed, and evenly (a flat surface, no occlusion) in the middle.
        let a = px[100 * 256 + 120];
        let b = px[100 * 256 + 136];
        assert_ne!(a, 0x0080_8080);
        assert_eq!(a, b);
    }

    #[test]
    fn light_sources_light_their_surroundings_at_night() {
        let width = 384;
        let (depth, layer) = floor(width);
        let scene = Scene {
            depth: &depth,
            layer: &layer,
            proj: PROJ,
            fade: &[],
            width,
            geometry: Geometry::default(),
            hours: 12.0,
            attached: &[],
        };
        let dark = 0x0050_5060u32;
        let mut plain = vec![dark; 256 * HEIGHT];
        let mut lamp = plain.clone();
        for y in 60..64 {
            for x in 126..130 {
                lamp[y * 256 + x] = 0x00FF_C060;
            }
        }
        if !Lighting::new().use_gpu() {
            eprintln!("no GPU here: the overworld's lighting isn't tested");
            return;
        }
        let lit = |frames: usize, px: &[u32]| {
            let mut lighting = Lighting::new();
            lighting.use_gpu();
            let mut out = px.to_vec();
            for _ in 0..frames {
                out = px.to_vec();
                lighting.overworld(&mut out, 256, 64, &scene, Place::Outdoor, Phase::Night);
            }
            out
        };
        let plain_lit = lit(1, &plain);
        let r = |p: u32| (p >> 16) & 0xFF;
        // The lamp's first frame: it only starts to show (it fades in, no pop).
        let first = lit(1, &lamp);
        assert!(r(first[70 * 256 + 128]) < r(plain_lit[70 * 256 + 128]) + 4);
        // A fifth of a second on: near the lamp, warmer and brighter than without it; far away,
        // the same.
        let shown = lit(14, &lamp);
        assert!(r(shown[70 * 256 + 128]) > r(plain_lit[70 * 256 + 128]) + 8);
        assert_eq!(shown[140 * 256 + 10], plain_lit[140 * 256 + 10]);
        plain.clear();
    }

    #[test]
    fn the_grade_keeps_black_black() {
        let mut px = vec![0u32; 256 * 192];
        Lighting::new().grade(&mut px, 256, 192, Phase::Night);
        assert!(px.iter().all(|&p| p == 0));
    }

    #[test]
    fn the_games_fades_stay_on_top_of_the_lighting() {
        let mut lighting = Lighting::new();
        // Line 0 faded all the way to white, line 1 half way, the rest plain.
        let mut fade = vec![[0u8; 4]; 192];
        fade[0] = [1, 16, 0, 0];
        fade[1] = [1, 8, 0, 0];
        let scene = 0x0040_6080u32;
        let half = map_rgb(scene, |c| fade_channel(c, 1, 0.5));
        let mut px = vec![scene; 256 * 192];
        px[..256].fill(0x00FF_FFFF);
        px[256..512].fill(half);
        lighting.grade_faded(&mut px, 256, Phase::Night, &fade);
        // The white line is the game's, untouched by the grade.
        assert!(px[..256].iter().all(|&p| p == 0x00FF_FFFF));
        // The half-faded line: the grade's change to the scene, then the fade again; the plain
        // line shows the grade alone.
        let graded = px[2 * 256 + 128];
        let expect = map_rgb(graded, |c| fade_channel(c, 1, 0.5));
        let got = px[256 + 128];
        for s in [16u32, 8, 0] {
            let (a, b) = ((got >> s) & 0xFF, (expect >> s) & 0xFF);
            assert!(a.abs_diff(b) <= 2, "{got:06x} {expect:06x}");
        }
    }
}
