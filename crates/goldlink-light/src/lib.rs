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
//!    become point lights placed in 3D, casting warm pools on the ground, and glow a little;
//!    then a soft bloom from what's brightest;
//! 5. a slight blur of what's far behind the player;
//! 6. the time of day's colour grade and a vignette.
//!
//! Pixels where the 3D scene doesn't show (a text box, menus) are left exactly as they are.
//! Battles and menus get [`Lighting::grade`] only: a small grade, no lighting.

mod places;

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
    /// The light sources' own small glow.
    glow: f32,
    /// A light above the player (colour times strength; zero: none), how high above them and
    /// its reach: the room's light indoors (high and wide, so the room is lit around them
    /// rather than a pool at their feet), a lantern's in caves, a faint one at night.
    player_light: V3,
    player_height: f32,
    player_range: f32,
    /// Bloom: what's brighter than `bloom_from` (linear luminance) spreads a soft halo.
    bloom: f32,
    bloom_from: f32,
    /// The most the far blur (what lies well behind the player) mixes in.
    far_blur: f32,
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
        glow: 0.0,
        player_light: [0.0; 3],
        player_height: 0.0,
        player_range: 1.0,
        bloom: 0.10,
        bloom_from: 0.65,
        far_blur: 0.45,
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
                glow: 0.16,
                player_light: scale([1.0, 0.84, 0.62], 0.16),
                player_height: 40.0,
                player_range: 110.0,
                bloom: 0.18,
                bloom_from: 0.4,
                ..outdoor
            },
            Phase::Night => Preset {
                ambient: scale([0.62, 0.72, 1.0], 0.82),
                key: scale([0.62, 0.74, 1.0], 0.26),
                key_dir: normalize([0.3, 0.8, 0.5]),
                ao: 1.3,
                points: 1.3,
                glow: 0.3,
                player_light: scale([1.0, 0.86, 0.66], 0.30),
                player_height: 40.0,
                player_range: 110.0,
                bloom: 0.26,
                bloom_from: 0.22,
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
                glow: 0.0,
                player_light: scale([1.0, 0.82, 0.58], 0.85),
                player_height: 70.0,
                player_range: 210.0,
                bloom: 0.2,
                bloom_from: 0.6,
                far_blur: 0.0,
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
            glow: 0.0,
            player_light: scale([1.0, 0.84, 0.62], 0.75),
            player_height: 60.0,
            player_range: 190.0,
            bloom: 0.22,
            bloom_from: 0.45,
            far_blur: 0.0,
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

/// Ambient occlusion's reach, in view units (a tile is 16).
const AO_RANGE: f32 = 20.0;
/// A window light's reach.
const LIGHT_RANGE: f32 = 90.0;
/// At most this many point lights (the biggest bright patches).
const MAX_LIGHTS: usize = 16;
/// Neighbours further apart than this in depth (one pixel apart) are different surfaces.
const EDGE: f32 = 12.0;
/// The least light the ambient keeps where occlusion is strongest.
const AMBIENT_FLOOR: f32 = 0.55;
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

/// One pixel's colour (linear), position (view space) and what it shows.
#[derive(Debug, Clone, Copy, Default)]
struct Cell {
    base: V3,
    pos: V3,
    /// 1: the 3D scene; 0: a 2D layer shows (left as it is).
    layer: u8,
    /// A light source (a lit window, a lamp, a sign).
    emissive: bool,
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
    out: Vec<V3>,
    /// Half-size buffers for the bloom.
    half: Vec<V3>,
    half_tmp: Vec<V3>,
    ao: Vec<f32>,
    tmp1: Vec<f32>,
    label: Vec<u32>,
    stack: Vec<u32>,
    /// The vignette for (width, height, strength), per pixel.
    vignette: (usize, usize, f32, Vec<f32>),
    /// Where the player stood last frame (view space), for [`Lighting::focus`].
    focus: Option<V3>,
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
        // The emulation and the window have a core each; up to four more share this.
        let threads = std::thread::available_parallelism()
            .map_or(1, |n| n.get().saturating_sub(2).clamp(1, 4));
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
            out: Vec::new(),
            half: Vec::new(),
            half_tmp: Vec::new(),
            ao: Vec::new(),
            tmp1: Vec::new(),
            label: Vec::new(),
            stack: Vec::new(),
            vignette: (0, 0, 0.0, Vec::new()),
            focus: None,
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
    /// column `x0`. Pixels the scene marks as 2D only are left as they are.
    pub fn overworld(
        &mut self,
        px: &mut [u32],
        w: usize,
        x0: usize,
        scene: &Scene,
        place: Place,
        phase: Phase,
    ) {
        // The game's fades come off first and go back on last (the scene is lit, not the fade).
        let layer = Some((scene.layer, scene.width));
        let faded = Faded::lift(px, w, x0, scene.fade, layer);
        self.overworld_scene(px, w, x0, scene, place, phase);
        faded.put_back(px, w, x0, scene.fade, layer);
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
        let p = preset(place, phase);
        self.cells.resize(n, Cell::default());
        for v in [&mut self.nrm, &mut self.tmp3, &mut self.out] {
            v.resize(n, [0.0; 3]);
        }
        self.ao.resize(n, 1.0);
        self.tmp1.resize(n, 1.0);
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

        self.normals(w, h);
        self.occlusion(w, h, p.ao);
        let mut lights = if p.points > 0.0 {
            self.find_lights(w, h, p.points)
        } else {
            Vec::new()
        };
        let window_lights = !lights.is_empty();
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

        // 3. Ambient, key light and point lights, on each pixel's own colour.
        {
            let norm = lum(p.ambient) + lum(p.key) * 0.85 + lum(p.player_light) * 0.45;
            let (cells, nrm, ao) = (&self.cells, &self.nrm, &self.ao);
            let lights = &lights;
            par_rows(self.pool.as_ref(), &mut self.out, w, |y, row| {
                for (x, out) in row.iter_mut().enumerate() {
                    let i = y * w + x;
                    let c = cells[i];
                    if c.layer == 0 || c.emissive {
                        *out = c.base;
                        continue;
                    }
                    let (n, ao) = (nrm[i], ao[i]);
                    let wrap = ((dot(n, p.key_dir) + 0.35) / 1.35).clamp(0.0, 1.0);
                    let mut light = add(
                        scale(p.ambient, AMBIENT_FLOOR + (1.0 - AMBIENT_FLOOR) * ao),
                        scale(p.key, wrap * ao),
                    );
                    for l in lights {
                        let v = sub(l.pos, c.pos);
                        let d2 = dot(v, v);
                        if d2 >= l.range * l.range {
                            continue;
                        }
                        let d = d2.sqrt().max(1e-3);
                        let fall = 1.0 - d / l.range;
                        let facing = (dot(n, v) / d).max(0.0);
                        light = add(light, scale(l.colour, fall * fall * (0.3 + 0.7 * facing)));
                    }
                    let lit = scale(mul(c.base, light), 1.0 / norm);
                    *out = lit;
                }
            });
        }

        // 4. A little glow around the light sources, and a soft bloom from what's brightest.
        if p.glow > 0.0 && window_lights {
            self.glow(w, h, p.glow);
        }
        if p.bloom > 0.0 {
            self.bloom(w, h, p.bloom, p.bloom_from);
        }
        // 5. A slight blur of what lies far behind the player.
        if p.far_blur > 0.0
            && let Some(f) = focus
        {
            self.far_blur(w, h, -f[2], p.far_blur);
        }
        // 6. The grade and the vignette; 2D-only pixels as they were.
        {
            self.vignette_table(w, h, p.grade.vignette);
            let (cells, out, srgb, vig) = (&self.cells, &self.out, &self.to_srgb, &self.vignette.3);
            par_rows(self.pool.as_ref(), &mut px[..n], w, |y, row| {
                for (x, px) in row.iter_mut().enumerate() {
                    let i = y * w + x;
                    if cells[i].layer == 0 {
                        continue;
                    }
                    let v = vig[i];
                    *px = pack(srgb, apply_grade(out[i], &p.grade, v));
                }
            });
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

    /// Screen-space ambient occlusion: taps around each pixel that lie in front of its surface
    /// (along its normal) and within reach darken it; then smoothed over the same surface.
    fn occlusion(&mut self, w: usize, h: usize, strength: f32) {
        let (pos, layer, nrm, taps) = (&self.pos, &self.layer, &self.nrm, &self.ao_taps);
        let k = 1.6 * strength / taps.len() as f32;
        let reach = taps
            .iter()
            .map(|t| t.0.unsigned_abs().max(t.1.unsigned_abs()))
            .max()
            .unwrap_or(0);
        par_rows(self.pool.as_ref(), &mut self.tmp1, w, |y, row| {
            let inner_y = y >= reach && y + reach < h;
            for (x, out) in row.iter_mut().enumerate() {
                let i = y * w + x;
                *out = 1.0;
                if layer[i] == 0 {
                    continue;
                }
                let (p, n) = (pos[i], nrm[i]);
                let inner = inner_y && x >= reach && x + reach < w;
                let mut occ = 0.0;
                for &(dx, dy) in taps {
                    let j = if inner {
                        (i as isize + dy * w as isize + dx) as usize
                    } else {
                        let (xx, yy) = (x as isize + dx, y as isize + dy);
                        if xx < 0 || yy < 0 || xx >= w as isize || yy >= h as isize {
                            continue;
                        }
                        yy as usize * w + xx as usize
                    };
                    if layer[j] == 0 {
                        continue;
                    }
                    let v = sub(pos[j], p);
                    let d2 = dot(v, v);
                    let along = dot(n, v);
                    // In front of the surface (cos > 0.15) and within reach.
                    if along <= 0.0 || d2 >= AO_RANGE * AO_RANGE || along * along <= 0.0225 * d2 {
                        continue;
                    }
                    let d = d2.sqrt();
                    occ += (along / d - 0.15) * (1.0 - d / AO_RANGE);
                }
                *out = (1.0 - k * occ).clamp(0.5, 1.0);
            }
        });
        let raw = &self.tmp1;
        par_rows(self.pool.as_ref(), &mut self.ao, w, |y, row| {
            let (y0, y1) = (y.saturating_sub(1), (y + 1).min(h - 1));
            for (x, out) in row.iter_mut().enumerate() {
                let i = y * w + x;
                if layer[i] == 0 {
                    *out = 1.0;
                    continue;
                }
                let z = pos[i][2];
                let (x0, x1) = (x.saturating_sub(1), (x + 1).min(w - 1));
                let (mut sum, mut cnt) = (0.0, 0.0);
                for yy in y0..=y1 {
                    for j in yy * w + x0..=yy * w + x1 {
                        if layer[j] != 0 && (pos[j][2] - z).abs() < 3.0 {
                            sum += raw[j];
                            cnt += 1.0;
                        }
                    }
                }
                *out = sum / cnt;
            }
        });
    }

    /// The light sources: pixels far brighter than the scene (lit windows, lamps, signs) that
    /// are warm or very bright (the night's ordinary pale surfaces lean blue). Each patch of
    /// them becomes a point light, placed in 3D just off its surface.
    fn find_lights(&mut self, w: usize, h: usize, strength: f32) -> Vec<PointLight> {
        let n = w * h;
        let (mut total, mut count) = (0.0f32, 0u32);
        for c in &self.cells[..n] {
            if c.layer != 0 {
                total += lum(c.base);
                count += 1;
            }
        }
        let mean = total / count.max(1) as f32;
        let threshold = (mean * 2.2).max(0.25);
        for c in &mut self.cells[..n] {
            let b = c.base;
            let l = lum(b);
            let warm = b[0] >= b[2] * 1.15;
            c.emissive = c.layer == 1
                && l > threshold
                && (warm || l > threshold * 1.8)
                && b[0].max(b[1]).max(b[2]) > 0.45;
        }
        // Patches: 4-connected runs of light pixels.
        self.label[..n].fill(0);
        struct Patch {
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
                PointLight {
                    pos: add(scale(p.pos, k), scale(normalize(p.nrm), 6.0)),
                    colour: scale(colour, power),
                    range: LIGHT_RANGE,
                }
            })
            .collect()
    }

    /// The light sources' own colour, spread two or three pixels and added: a small glow (only
    /// the rows around them are worked on).
    fn glow(&mut self, w: usize, h: usize, strength: f32) {
        const R: usize = 2;
        let rows = |c: &[Cell]| c.chunks(w).position(|r| r.iter().any(|c| c.emissive));
        let Some(top) = rows(&self.cells[..w * h]) else {
            return;
        };
        let bottom = h
            - 1
            - self.cells[..w * h]
                .rchunks(w)
                .position(|r| r.iter().any(|c| c.emissive))
                .unwrap_or(0);
        let (y0, y1) = (top.saturating_sub(3 * R), (bottom + 3 * R + 1).min(h));
        let band = (y1 - y0) * w;
        let cells = &self.cells[y0 * w..y1 * w];
        for (out, c) in self.tmp3[..band].iter_mut().zip(cells) {
            *out = if c.emissive { c.base } else { [0.0; 3] };
        }
        for _ in 0..2 {
            box_blur(
                self.pool.as_ref(),
                &mut self.tmp3[..band],
                &mut self.nrm[..band],
                w,
                y1 - y0,
                R,
            );
        }
        for (out, g) in self.out[y0 * w..y1 * w].iter_mut().zip(&self.tmp3[..band]) {
            *out = add(*out, scale(*g, strength * 1.5));
        }
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

    /// Bloom: what's brighter than `from` (linear luminance), and the light sources, at half
    /// size, spread over about eight pixels and added back, on the 3D scene.
    fn bloom(&mut self, w: usize, h: usize, strength: f32, from: f32) {
        let (hw, hh) = (w / 2, h / 2);
        if hw == 0 || hh == 0 {
            return;
        }
        self.half.resize(hw * hh, [0.0; 3]);
        self.half_tmp.resize(hw * hh, [0.0; 3]);
        {
            let (cells, out) = (&self.cells, &self.out);
            par_rows(self.pool.as_ref(), &mut self.half, hw, |y, row| {
                for (x, o) in row.iter_mut().enumerate() {
                    let mut sum = [0.0f32; 3];
                    for (dy, dx) in [(0, 0), (0, 1), (1, 0), (1, 1)] {
                        let i = (2 * y + dy) * w + 2 * x + dx;
                        let c = out[i];
                        let l = lum(c);
                        let k = if cells[i].layer == 0 {
                            0.0
                        } else if cells[i].emissive {
                            1.0
                        } else {
                            ((l - from) / (1.0 - from).max(0.05)).clamp(0.0, 1.0)
                        };
                        sum = add(sum, scale(c, k));
                    }
                    *o = scale(sum, 0.25);
                }
            });
        }
        for _ in 0..2 {
            box_blur(
                self.pool.as_ref(),
                &mut self.half,
                &mut self.half_tmp,
                hw,
                hh,
                3,
            );
        }
        let (cells, half) = (&self.cells, &self.half);
        par_rows(self.pool.as_ref(), &mut self.out, w, |y, row| {
            // What's (nearly) black takes none: a room's dark surround doesn't glow.
            let fy = ((y as f32 + 0.5) / 2.0 - 0.5).clamp(0.0, (hh - 1) as f32);
            let (y0, ty) = (fy as usize, fy.fract());
            let y1 = (y0 + 1).min(hh - 1);
            for (x, o) in row.iter_mut().enumerate() {
                if cells[y * w + x].layer == 0 {
                    continue;
                }
                let fx = ((x as f32 + 0.5) / 2.0 - 0.5).clamp(0.0, (hw - 1) as f32);
                let (x0, tx) = (fx as usize, fx.fract());
                let x1 = (x0 + 1).min(hw - 1);
                let top = mix(half[y0 * hw + x0], half[y0 * hw + x1], tx);
                let bottom = mix(half[y1 * hw + x0], half[y1 * hw + x1], tx);
                let takes = (lum(cells[y * w + x].base) / 0.04).min(1.0);
                *o = add(*o, scale(mix(top, bottom, ty), strength * takes));
            }
        });
    }

    /// A slight blur, mixed in up to `far` for what lies well behind the player (past
    /// `focus_z`). Only the 3D scene blurs, only with itself (text boxes don't bleed in).
    fn far_blur(&mut self, w: usize, h: usize, focus_z: f32, far: f32) {
        const R: isize = 2;
        let smooth = |a: f32, b: f32, v: f32| {
            let t = ((v - a) / (b - a)).clamp(0.0, 1.0);
            t * t * (3.0 - 2.0 * t)
        };
        // Across, then down: the 3D neighbours' sum and count.
        {
            let (cells, out) = (&self.cells, &self.out);
            par_rows(self.pool.as_ref(), &mut self.tmp3, w, |y, row| {
                for (x, o) in row.iter_mut().enumerate() {
                    let mut s = [0.0f32; 3];
                    for xx in (x as isize - R).max(0)..=(x as isize + R).min(w as isize - 1) {
                        let j = y * w + xx as usize;
                        if cells[j].layer != 0 {
                            s = add(s, out[j]);
                        }
                    }
                    *o = s;
                }
            });
            par_rows(self.pool.as_ref(), &mut self.tmp1, w, |y, row| {
                for (x, o) in row.iter_mut().enumerate() {
                    let mut n = 0.0;
                    for xx in (x as isize - R).max(0)..=(x as isize + R).min(w as isize - 1) {
                        if cells[y * w + xx as usize].layer != 0 {
                            n += 1.0;
                        }
                    }
                    *o = n;
                }
            });
        }
        let (cells, across, count) = (&self.cells, &self.tmp3, &self.tmp1);
        par_rows(self.pool.as_ref(), &mut self.out, w, |y, row| {
            for (x, o) in row.iter_mut().enumerate() {
                let c = cells[y * w + x];
                if c.layer == 0 {
                    continue;
                }
                let t = far * smooth(30.0, 250.0, -c.pos[2] - focus_z);
                if t <= 0.01 {
                    continue;
                }
                let (mut s, mut n) = ([0.0f32; 3], 0.0);
                for yy in (y as isize - R).max(0)..=(y as isize + R).min(h as isize - 1) {
                    let j = yy as usize * w + x;
                    s = add(s, across[j]);
                    n += count[j];
                }
                if n > 0.0 {
                    *o = mix(*o, scale(s, 1.0 / n), t);
                }
            }
        });
    }
}

/// A separable box blur of radius `r`, in place (`tmp` is scratch).
fn box_blur(
    pool: Option<&rayon::ThreadPool>,
    buf: &mut [V3],
    tmp: &mut [V3],
    w: usize,
    h: usize,
    r: usize,
) {
    let k = 1.0 / (2 * r + 1) as f32;
    {
        let src = &*buf;
        par_rows(pool, &mut tmp[..w * h], w, |y, row| {
            for (x, out) in row.iter_mut().enumerate() {
                let mut s = [0.0f32; 3];
                for xx in x as isize - r as isize..=x as isize + r as isize {
                    s = add(s, src[y * w + xx.clamp(0, w as isize - 1) as usize]);
                }
                *out = scale(s, k);
            }
        });
    }
    let src = &*tmp;
    par_rows(pool, &mut buf[..w * h], w, |y, row| {
        for (x, out) in row.iter_mut().enumerate() {
            let mut s = [0.0f32; 3];
            for yy in y as isize - r as isize..=y as isize + r as isize {
                s = add(s, src[yy.clamp(0, h as isize - 1) as usize * w + x]);
            }
            *out = scale(s, k);
        }
    });
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
        };
        let mut px = vec![0x0080_8080u32; 256 * HEIGHT];
        let mut lighting = Lighting::new();
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
        };
        let dark = 0x0050_5060u32;
        let mut plain = vec![dark; 256 * HEIGHT];
        let mut lamp = plain.clone();
        for y in 60..64 {
            for x in 126..130 {
                lamp[y * 256 + x] = 0x00FF_C060;
            }
        }
        let mut lighting = Lighting::new();
        lighting.overworld(&mut plain, 256, 64, &scene, Place::Outdoor, Phase::Night);
        lighting.overworld(&mut lamp, 256, 64, &scene, Place::Outdoor, Phase::Night);
        let r = |p: u32| (p >> 16) & 0xFF;
        // Near the lamp, warmer and brighter than without it; far away, the same.
        assert!(r(lamp[70 * 256 + 128]) > r(plain[70 * 256 + 128]) + 8);
        assert_eq!(lamp[140 * 256 + 10], plain[140 * 256 + 10]);
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
