//! Smooth camera (UC11): HeartGold draws the overworld 30 times a second, so while the player
//! walks the picture changes every other frame and the camera steps 3–4 DS pixels at a time.
//! When a new picture follows a repeated one and the change is a camera move, that frame shows
//! the scene half way between the two instead, at twice the DS resolution: each step becomes
//! two, at 60 a second, and no pixel is blurred.
//!
//! Half way is worked out in 3D, from the scene's depth (the lighting's buffers): the camera's
//! move between the two pictures is found (the one that best explains the new picture from the
//! old), and each pixel goes half way along its own path. In a perspective town what's near the
//! camera (tree tops, roofs) moves further than the ground behind it, by fractions of a pixel;
//! a single shift for the picture, or for a band of rows, made those wobble.
//!
//! What stays put on the screen (the player and their follower, whom the camera follows, the
//! text box) stays where the new picture has it: each pixel is sorted by its neighbourhood (does
//! it match the old picture where it was, or the old picture moved with the camera?). Where the
//! moved scene would show a copy of something that stays put, the old picture moved forward
//! half a step shows what was behind it.

use goldlink_light::Camera;

/// The furthest a step can go, in DS pixels (running, cycling).
const REACH: i32 = 8;
/// Below this share of the picture changing, the camera didn't move: a camera step changes
/// 40–65% of it (the Pokémon Centre's plain floor included), sprites walking or turning a few
/// percent (taken for a step, the whole scene jumped for a frame).
const LEAST_CHANGE: f32 = 0.15;
/// The share of the changed pixels a camera move must explain to count as one.
const LEAST_MATCH: f32 = 0.45;
/// The neighbourhood that sorts a pixel (its radius).
const VOTE: usize = 3;
/// Changed pixels sampled to find the move.
const SAMPLES: usize = 600;

/// The top picture's height (DS pixels).
pub const HEIGHT: usize = 192;

/// The new picture's 3D scene, as the lighting gets it: `width` x 192 (the picture centred).
pub struct Depth<'a> {
    pub depth: &'a [u32],
    /// 1: the 3D scene shows on top; anything else: a 2D layer (stays put).
    pub layer: &'a [u8],
    pub proj: [i32; 16],
    pub width: usize,
}

#[derive(Default)]
pub struct Smooth {
    /// The last new picture: as the game drew it, as it was shown, its width and frame number.
    raw: Vec<u32>,
    lit: Vec<u32>,
    w: usize,
    seq: u64,
    /// Where the last new picture showed its 3D scene (1) or a 2D layer (0).
    layer: Vec<u8>,
    /// Per pixel of the new picture: where it is in view space; how far it moves back to show
    /// half way (DS pixels); whether it stays put.
    pos: Vec<Option<[f32; 3]>>,
    half: Vec<(f32, f32)>,
    still: Vec<bool>,
    vote: Vec<i32>,
    vote_tmp: Vec<i32>,
    /// The sprites' cards: per pixel, its card (0: none), and each card's vote.
    card: Vec<u32>,
    card_votes: Vec<i32>,
    stack: Vec<usize>,
    /// Worker threads for the per-pixel passes (from the first half-way frame).
    pool: Option<rayon::ThreadPool>,
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

/// `px` (`w` x `h`) at twice the size.
pub fn double(px: &[u32], w: usize, h: usize) -> Vec<u32> {
    let mut out = vec![0u32; 4 * w * h];
    for y in 0..h {
        let row = &px[y * w..(y + 1) * w];
        let (a, b) = out[2 * y * 2 * w..(2 * y + 2) * 2 * w].split_at_mut(2 * w);
        for (x, &p) in row.iter().enumerate() {
            a[2 * x] = p;
            a[2 * x + 1] = p;
        }
        b.copy_from_slice(a);
    }
    out
}

fn sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn scale(a: [f32; 3], s: f32) -> [f32; 3] {
    [a[0] * s, a[1] * s, a[2] * s]
}

impl Smooth {
    pub fn new() -> Self {
        Self::default()
    }

    /// Forgets the last picture (the setting turned off, a new game).
    pub fn reset(&mut self) {
        self.w = 0;
    }

    /// Frame `seq` of the top picture: `raw` as the game drew it, `lit` as it is to be shown,
    /// both `w` x 192, and its 3D scene when it shows one. Returns the picture half way from
    /// the last new one (`2w` x 384) when this frame should show that; otherwise `lit` is shown
    /// as it is.
    pub fn frame(
        &mut self,
        seq: u64,
        raw: &[u32],
        lit: &[u32],
        w: usize,
        scene: Option<&Depth>,
    ) -> Option<Vec<u32>> {
        let n = w * HEIGHT;
        if raw.len() < n || lit.len() < n {
            return None;
        }
        let (raw, lit) = (&raw[..n], &lit[..n]);
        if self.w == w {
            // The game showed the same 3D picture again (what's 2D may change: a map's name
            // sliding in over the walk).
            let same = match scene {
                // (Only when the last new picture's layers are known too.)
                Some(sc)
                    if sc.width >= w
                        && sc.layer.len() >= sc.width * HEIGHT
                        && self.layer.len() == n =>
                {
                    let x0 = (sc.width - w) / 2;
                    (0..n).all(|i| {
                        raw[i] == self.raw[i]
                            || sc.layer[(i / w) * sc.width + x0 + i % w] != 1
                            || self.layer.get(i) != Some(&1)
                    })
                }
                _ => self.raw == raw,
            };
            if same {
                return None;
            }
        }
        // Half way only after the game repeated its picture (the overworld's 30 a second; now
        // and then a step takes a frame longer).
        let between = match scene {
            Some(scene) if self.w == w && (2..=3).contains(&seq.wrapping_sub(self.seq)) => {
                self.between(raw, lit, w, scene)
            }
            _ => None,
        };
        self.raw.clear();
        self.raw.extend_from_slice(raw);
        self.lit.clear();
        self.lit.extend_from_slice(lit);
        self.w = w;
        self.seq = seq;
        self.layer.clear();
        if let Some(sc) = scene.filter(|sc| sc.width >= w && sc.layer.len() >= sc.width * HEIGHT) {
            let x0 = (sc.width - w) / 2;
            self.layer
                .extend((0..n).map(|i| sc.layer[(i / w) * sc.width + x0 + i % w]));
        }
        between
    }

    fn between(&mut self, new: &[u32], lit: &[u32], w: usize, scene: &Depth) -> Option<Vec<u32>> {
        let h = HEIGHT;
        let n = w * h;
        if scene.width < w
            || scene.depth.len() < scene.width * h
            || scene.layer.len() < scene.width * h
        {
            return None;
        }
        let x0 = (scene.width - w) / 2;
        if self.pool.is_none() {
            // The emulation and the window have a core each; up to four more share this.
            let threads = std::thread::available_parallelism()
                .map_or(1, |n| n.get().saturating_sub(2).clamp(1, 4));
            self.pool = (threads > 1)
                .then(|| {
                    rayon::ThreadPoolBuilder::new()
                        .num_threads(threads)
                        .thread_name(|i| format!("goldlink-smooth-{i}"))
                        .build()
                        .ok()
                })
                .flatten();
        }
        let pool = self.pool.as_ref();
        let old = &self.raw;
        let changed: Vec<usize> = (0..n).filter(|&i| new[i] != old[i]).collect();
        if (changed.len() as f32) < LEAST_CHANGE * n as f32 {
            return None;
        }

        // 1. Roughly: the whole-pixel shift that explains most of what changed (none, or too
        // little explained: not a camera move, as a fade or a warp).
        let step = (changed.len() / SAMPLES).max(1);
        let moved = |i: usize, dx: i32, dy: i32| {
            let (x, y) = ((i % w) as i32 - dx, (i / w) as i32 - dy);
            x >= 0
                && y >= 0
                && x < w as i32
                && y < h as i32
                && old[y as usize * w + x as usize] == new[i]
        };
        let row_best = |dy: i32| {
            let mut best = (0u32, 0i32, dy);
            for dx in -REACH..=REACH {
                let hit = changed
                    .iter()
                    .step_by(step)
                    .filter(|&&i| moved(i, dx, dy))
                    .count() as u32;
                if hit > best.0 {
                    best = (hit, dx, dy);
                }
            }
            best
        };
        let pick = |a: (u32, i32, i32), b: (u32, i32, i32)| if b.0 > a.0 { b } else { a };
        let best = match pool {
            Some(pool) => pool.install(|| {
                use rayon::prelude::*;
                (-REACH..=REACH)
                    .into_par_iter()
                    .map(row_best)
                    .reduce(|| (0, 0, 0), pick)
            }),
            None => (-REACH..=REACH).map(row_best).fold((0, 0, 0), pick),
        };
        let sampled = changed.len().div_ceil(step) as f32;
        let (_, gx, gy) = best;
        if (gx, gy) == (0, 0) || (best.0 as f32) < LEAST_MATCH * sampled {
            return None;
        }

        // 2. Where each pixel of the new picture is, in view space.
        let cam = Camera::new(&scene.proj, scene.width)?;
        let at = |x: usize, y: usize| y * scene.width + x0 + x;
        self.pos.clear();
        self.pos.resize(n, None);
        par_rows(pool, &mut self.pos, w, |y, row| {
            for (x, out) in row.iter_mut().enumerate() {
                *out = (scene.layer[at(x, y)] == 1)
                    .then(|| {
                        cam.unproject((x0 + x) as f32 + 0.5, y as f32 + 0.5, scene.depth[at(x, y)])
                    })
                    .flatten();
            }
        });
        let pos = &self.pos;

        // 3. The camera's move (in view space, what the scene moved by): the one that best
        // explains the changed pixels, starting from the whole-pixel shift.
        let samples: Vec<usize> = changed
            .iter()
            .step_by(step)
            .copied()
            .filter(|&i| pos[i].is_some())
            .collect();
        if samples.len() < 32 {
            return None;
        }
        // The picture's pixel a view-space point falls in.
        let pixel = |p: [f32; 3]| -> Option<usize> {
            let (c, r) = cam.project(p)?;
            let (x, y) = ((c - x0 as f32).floor(), r.floor());
            (x >= 0.0 && y >= 0.0 && x < w as f32 && y < h as f32)
                .then(|| y as usize * w + x as usize)
        };
        let score = |d: [f32; 3]| {
            samples
                .iter()
                .filter(|&&i| {
                    let p = pos[i].expect("sampled with a position");
                    pixel(sub(p, d)).is_some_and(|j| old[j] == new[i])
                })
                .count()
        };
        let mut by_z: Vec<usize> = samples.clone();
        by_z.sort_by(|&a, &b| pos[a].unwrap()[2].total_cmp(&pos[b].unwrap()[2]));
        let mid = by_z[by_z.len() / 2];
        let (mx, my) = (mid % w, mid / w);
        let from = cam.unproject(
            (x0 + mx) as f32 + 0.5 - gx as f32,
            my as f32 + 0.5 - gy as f32,
            scene.depth[at(mx, my)],
        )?;
        let mut d = sub(pos[mid].unwrap(), from);
        let mut got = score(d);
        let size = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt().max(1e-3);
        let mut steps = [size * 0.2, size * 0.2, size * 0.8];
        for _ in 0..7 {
            loop {
                let mut better = None;
                for axis in 0..3 {
                    for sign in [-1.0, 1.0] {
                        let mut t = d;
                        t[axis] += sign * steps[axis];
                        let s = score(t);
                        if s > better.map_or(got, |(s, _)| s) {
                            better = Some((s, t));
                        }
                    }
                }
                match better {
                    Some((s, t)) => (got, d) = (s, t),
                    None => break,
                }
            }
            steps = steps.map(|s| s / 2.0);
        }
        if (got as f32) < LEAST_MATCH * samples.len() as f32 {
            return None;
        }

        // 4. Each pixel's way back to half way, and what stays put: a sprite (a card facing the
        // camera, flat in depth) as a whole, by its pixels' vote; anything else by its
        // neighbourhood's.
        let half_d = scale(d, 0.5);
        self.half.clear();
        self.half.resize(n, (0.0, 0.0));
        par_rows(pool, &mut self.half, w, |y, row| {
            for (x, out) in row.iter_mut().enumerate() {
                *out = pos[y * w + x]
                    .and_then(|p| cam.project(sub(p, half_d)))
                    .map_or((0.0, 0.0), |(c, r)| {
                        (x as f32 + 0.5 - (c - x0 as f32), y as f32 + 0.5 - r)
                    });
            }
        });
        self.vote.clear();
        self.vote.resize(n, 0);
        par_rows(pool, &mut self.vote, w, |y, row| {
            for (x, out) in row.iter_mut().enumerate() {
                let i = y * w + x;
                let stays = new[i] == old[i];
                let moves =
                    pos[i].is_some_and(|p| pixel(sub(p, d)).is_some_and(|j| old[j] == new[i]));
                *out = i32::from(stays && !moves) - i32::from(moves && !stays);
            }
        });
        let cards = self.cards(w, h);
        for (i, &c) in self.card.iter().enumerate() {
            if c != 0 {
                self.card_votes[c as usize] += self.vote[i];
            }
        }
        box_sum(&mut self.vote, &mut self.vote_tmp, w, h, VOTE);
        let pos = &self.pos;
        self.still.clear();
        self.still.extend((0..n).map(|i| {
            let c = self.card[i] as usize;
            pos[i].is_none()
                || if c != 0 && cards[c] {
                    self.card_votes[c] > 0
                } else {
                    self.vote[i] > 0
                }
        }));
        let (half, still) = (&self.half, &self.still);
        let back = |i: usize| if still[i] { (0.0, 0.0) } else { half[i] };

        // 5. Twice the size: each moving pixel half way back along its path, what stays put
        // where it is.
        let (w2, h2) = (2 * w, 2 * h);
        let index = |x: f32, y: f32| -> Option<usize> {
            let (x, y) = (x.floor(), y.floor());
            (x >= 0.0 && y >= 0.0 && x < w as f32 && y < h as f32)
                .then(|| y as usize * w + x as usize)
        };
        let mut out = vec![0u32; w2 * h2];
        let old_lit = &self.lit;
        par_rows(self.pool.as_ref(), &mut out, w2, |y2, row| {
            for (x2, out) in row.iter_mut().enumerate() {
                let (x, y) = ((x2 as f32 + 0.5) / 2.0, (y2 as f32 + 0.5) / 2.0);
                let here = (y2 / 2) * w + x2 / 2;
                *out = if still[here] {
                    lit[here]
                } else {
                    // The new pixel whose half-way place is here (its path is smooth: two
                    // rounds settle it).
                    let mut from = Some(here);
                    for _ in 0..2 {
                        let (bx, by) = back(from.unwrap_or(here));
                        from = index(x + bx, y + by);
                    }
                    match from {
                        Some(j) if !still[j] => lit[j],
                        _ => {
                            // Behind what stays put: the old picture, half a step back.
                            let (bx, by) = half[here];
                            match index(x - bx, y - by) {
                                Some(j) => old_lit[j],
                                None => lit[here],
                            }
                        }
                    }
                };
            }
        });
        Some(out)
    }
}

impl Smooth {
    /// The sprites' cards in the new picture: 4-connected pixels whose depth is flat across
    /// (a card facing the camera; the ground and walls slope away). Fills `card` (0: none) and
    /// zeroes `card_votes`; returns which cards are big enough to decide as a whole.
    fn cards(&mut self, w: usize, h: usize) -> Vec<bool> {
        /// Depth change per pixel (view units) below which a surface faces the camera.
        const FLAT: f32 = 0.03;
        /// Fewer pixels than this: not a sprite (a flat speck of something else).
        const LEAST: u32 = 12;
        let n = w * h;
        let pos = &self.pos;
        let z = |i: usize| pos[i].map(|p| p[2]);
        let flat = |i: usize, j: usize| match (z(i), z(j)) {
            (Some(a), Some(b)) => (a - b).abs() < FLAT,
            _ => false,
        };
        let is_card = |i: usize| {
            let (x, y) = (i % w, i / w);
            let across = (x + 1 < w && flat(i, i + 1)) || (x > 0 && flat(i, i - 1));
            let down = (y + 1 < h && flat(i, i + w)) || (y > 0 && flat(i, i - w));
            across && down
        };
        self.card.clear();
        self.card.resize(n, 0);
        let mut sizes = vec![0u32];
        for start in 0..n {
            if self.card[start] != 0 || !is_card(start) {
                continue;
            }
            let id = sizes.len() as u32;
            let mut size = 0;
            self.card[start] = id;
            self.stack.clear();
            self.stack.push(start);
            while let Some(i) = self.stack.pop() {
                size += 1;
                let (x, y) = (i % w, i / w);
                let next = [
                    (x > 0).then(|| i - 1),
                    (x + 1 < w).then(|| i + 1),
                    (y > 0).then(|| i - w),
                    (y + 1 < h).then(|| i + w),
                ];
                for j in next.into_iter().flatten() {
                    if self.card[j] == 0 && flat(i, j) && is_card(j) {
                        self.card[j] = id;
                        self.stack.push(j);
                    }
                }
            }
            sizes.push(size);
        }
        self.card_votes.clear();
        self.card_votes.resize(sizes.len(), 0);
        sizes.iter().map(|&s| s >= LEAST).collect()
    }
}

/// Each value replaced by the sum over the (2r+1)² square around it (clipped at the edges).
fn box_sum(v: &mut [i32], tmp: &mut Vec<i32>, w: usize, h: usize, r: usize) {
    tmp.clear();
    tmp.resize(w * h, 0);
    for y in 0..h {
        let row = &v[y * w..(y + 1) * w];
        let mut s: i32 = row[..r.min(w)].iter().sum();
        for x in 0..w {
            if x + r < w {
                s += row[x + r];
            }
            tmp[y * w + x] = s;
            if x >= r {
                s -= row[x - r];
            }
        }
    }
    for x in 0..w {
        let mut s: i32 = (0..r.min(h)).map(|y| tmp[y * w + x]).sum();
        for y in 0..h {
            if y + r < h {
                s += tmp[(y + r) * w + x];
            }
            v[y * w + x] = s;
            if y >= r {
                s -= tmp[(y - r) * w + x];
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An orthographic camera, one view unit a pixel (as HeartGold's rooms): x right, y up.
    const ORTHO: [i32; 16] = [32, 0, 0, 0, 0, 43, 0, 0, 0, 0, -5, 0, 0, 0, -4871, 4096];

    /// A textured "world" seen through a `w` x 192 window at (ox, oy), with a "player" (a
    /// solid square) fixed at the window's middle; flat, at one depth.
    fn view(ox: i32, oy: i32, w: usize) -> Vec<u32> {
        let mut px = vec![0u32; w * HEIGHT];
        for y in 0..HEIGHT {
            for x in 0..w {
                let (wx, wy) = (x as i32 + ox, y as i32 + oy);
                let v = (wx.wrapping_mul(73) ^ wy.wrapping_mul(151)).rem_euclid(97) as u32;
                px[y * w + x] = v * 0x010203;
            }
        }
        for y in 88..104 {
            for x in w / 2 - 8..w / 2 + 8 {
                px[y * w + x] = 0x00FF_0000;
            }
        }
        px
    }

    #[test]
    fn box_sum_sums_the_square() {
        let (w, h) = (5, 4);
        let mut v = vec![1; w * h];
        let mut tmp = Vec::new();
        box_sum(&mut v, &mut tmp, w, h, 1);
        assert_eq!(v[0], 4);
        assert_eq!(v[w + 1], 9);
        assert_eq!(v[w * h - 1], 4);
    }

    #[test]
    fn a_camera_step_after_a_repeat_shows_half_way_with_the_player_in_place() {
        let w = 256;
        let depth = vec![0x80_0000u32; w * HEIGHT];
        let layer = vec![1u8; w * HEIGHT];
        let scene = Depth {
            depth: &depth,
            layer: &layer,
            proj: ORTHO,
            width: w,
        };
        let a = view(0, 0, w);
        let b = view(4, 2, w);
        let mut s = Smooth::new();
        assert!(s.frame(10, &a, &a, w, Some(&scene)).is_none());
        assert!(s.frame(11, &a, &a, w, Some(&scene)).is_none(), "a repeat");
        let mid = s.frame(12, &b, &b, w, Some(&scene)).expect("half way");
        let half = view(2, 1, w);
        // At twice the size, the world half way (2, 1); the player where it always is.
        let mut wrong = 0;
        for y in 8..HEIGHT - 8 {
            for x in 8..w - 8 {
                if mid[(2 * y) * 2 * w + 2 * x] != half[y * w + x] {
                    wrong += 1;
                }
            }
        }
        assert!(wrong < (w * HEIGHT) / 100, "{wrong} pixels off");
        assert_eq!(mid[(2 * 96) * 2 * w + 2 * (w / 2)], 0x00FF_0000);
    }

    #[test]
    fn sprites_moving_alone_are_not_a_camera_step() {
        let w = 256;
        let depth = vec![0x80_0000u32; w * HEIGHT];
        let layer = vec![1u8; w * HEIGHT];
        let scene = Depth {
            depth: &depth,
            layer: &layer,
            proj: ORTHO,
            width: w,
        };
        let a = view(0, 0, w);
        // The same scene with a 40x40 patch of it moved 4 pixels: a sprite walking.
        let mut b = a.clone();
        for y in 40..80 {
            for x in 40..80 {
                b[y * w + x + 4] = a[y * w + x];
            }
        }
        let mut s = Smooth::new();
        s.frame(1, &a, &a, w, Some(&scene));
        assert!(s.frame(3, &b, &b, w, Some(&scene)).is_none());
    }

    #[test]
    fn a_first_picture_without_its_scene_doesnt_stop_it() {
        // The first new picture can come before the 3D scene's buffers (the game booting):
        // the next ones must still be told apart (all of their pixels compared).
        let w = 256;
        let depth = vec![0x80_0000u32; w * HEIGHT];
        let layer = vec![1u8; w * HEIGHT];
        let scene = Depth {
            depth: &depth,
            layer: &layer,
            proj: ORTHO,
            width: w,
        };
        let (a, b) = (view(0, 0, w), view(4, 2, w));
        let mut s = Smooth::new();
        assert!(s.frame(1, &a, &a, w, None).is_none());
        assert!(s.frame(2, &a, &a, w, Some(&scene)).is_none(), "a repeat");
        assert!(s.frame(3, &b, &b, w, Some(&scene)).is_some(), "half way");
    }

    #[test]
    fn no_half_way_at_60_a_second_without_a_move_or_without_depth() {
        let w = 256;
        let depth = vec![0x80_0000u32; w * HEIGHT];
        let layer = vec![1u8; w * HEIGHT];
        let scene = Depth {
            depth: &depth,
            layer: &layer,
            proj: ORTHO,
            width: w,
        };
        let mut s = Smooth::new();
        s.frame(1, &view(0, 0, w), &view(0, 0, w), w, Some(&scene));
        // The next frame already new: shown as it is.
        assert!(
            s.frame(2, &view(1, 0, w), &view(1, 0, w), w, Some(&scene))
                .is_none()
        );
        // A change that isn't a camera move (a fade): shown as it is.
        s.frame(3, &view(1, 0, w), &view(1, 0, w), w, Some(&scene));
        let fade: Vec<u32> = view(1, 0, w).iter().map(|p| p / 2).collect();
        assert!(s.frame(4, &fade, &fade, w, Some(&scene)).is_none());
        // No 3D scene: shown as it is.
        s.frame(5, &fade, &fade, w, Some(&scene));
        assert!(
            s.frame(7, &view(3, 0, w), &view(3, 0, w), w, None)
                .is_none()
        );
    }
}
