//! Smooth camera (UC11, fourth round): HeartGold draws the overworld 30 times a second, so while
//! the player walks the picture changes every other frame and the camera steps 3–4 DS pixels at
//! a time. When a new picture follows a repeated one and the change is a camera scroll, the
//! frame shows the scene half way between the two instead, at twice the DS resolution (half a
//! step is often half a pixel): each step becomes two, at 60 a second, and no pixel is blurred.
//!
//! What scrolls with the camera moves half way; what stays put on the screen (the player and
//! their follower, whom the camera follows, the text box) stays where the new picture has it.
//! Each DS pixel is sorted by its neighbourhood: does it match the old picture moved by the
//! scroll, or the old picture where it was? Where the scrolled scene would show a copy of
//! something that stays put, the old picture gives what was behind it.

/// Rows per band: the scroll is measured per band (in a perspective town the far rows move a
/// little less than the near ones).
const BAND: usize = 16;
/// The furthest a scroll can go in one step, in DS pixels (running, cycling).
const REACH: i32 = 8;
/// Below this share of the picture changing, nothing scrolled.
const LEAST_CHANGE: f32 = 0.01;
/// The share of the changed pixels a scroll must explain to count as one.
const LEAST_MATCH: f32 = 0.5;
/// The neighbourhood that sorts a pixel (its radius).
const VOTE: usize = 3;

#[derive(Default)]
pub struct Smooth {
    /// The last new picture: as the game drew it, as it was shown, its width and frame number.
    raw: Vec<u32>,
    lit: Vec<u32>,
    w: usize,
    seq: u64,
    shifts: Vec<(i32, i32)>,
    /// Per DS pixel: stays put on the screen (new picture, old picture).
    still_new: Vec<bool>,
    still_old: Vec<bool>,
    vote: Vec<i32>,
    vote_tmp: Vec<i32>,
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

impl Smooth {
    pub fn new() -> Self {
        Self::default()
    }

    /// Forgets the last picture (the setting turned off, a new game).
    pub fn reset(&mut self) {
        self.w = 0;
    }

    /// Frame `seq` of the top picture: `raw` as the game drew it, `lit` as it is to be shown,
    /// both `w` x 192. Returns the picture half way from the last new one (`2w` x 384) when this
    /// frame should show that; otherwise `lit` is shown as it is.
    pub fn frame(&mut self, seq: u64, raw: &[u32], lit: &[u32], w: usize) -> Option<Vec<u32>> {
        let n = w * HEIGHT;
        if raw.len() < n || lit.len() < n {
            return None;
        }
        let (raw, lit) = (&raw[..n], &lit[..n]);
        if self.w == w && self.raw == raw {
            // The game showed the same picture again.
            return None;
        }
        // Half way only after exactly one repeated frame: the overworld's 30 a second.
        let between = (self.w == w && seq.wrapping_sub(self.seq) == 2)
            .then(|| self.between(raw, lit, w))
            .flatten();
        self.raw.clear();
        self.raw.extend_from_slice(raw);
        self.lit.clear();
        self.lit.extend_from_slice(lit);
        self.w = w;
        self.seq = seq;
        between
    }

    fn between(&mut self, new: &[u32], lit: &[u32], w: usize) -> Option<Vec<u32>> {
        let h = HEIGHT;
        let old = &self.raw;
        let changed: Vec<usize> = (0..w * h).filter(|&i| new[i] != old[i]).collect();
        if (changed.len() as f32) < LEAST_CHANGE * (w * h) as f32 {
            return None;
        }
        // Does the new pixel at i show the old one moved by (dx, dy)?
        let moved = |i: usize, dx: i32, dy: i32| {
            let (x, y) = ((i % w) as i32 - dx, (i / w) as i32 - dy);
            x >= 0
                && y >= 0
                && x < w as i32
                && y < h as i32
                && old[y as usize * w + x as usize] == new[i]
        };
        let score = |pixels: &mut dyn Iterator<Item = usize>, dx: i32, dy: i32| {
            let (mut hit, mut all) = (0u32, 0u32);
            for i in pixels {
                all += 1;
                hit += u32::from(moved(i, dx, dy));
            }
            (hit, all)
        };
        // The scroll over the whole picture (on a sample of what changed), then per band.
        let step = (changed.len() / 1500).max(1);
        let mut best = (0u32, 0i32, 0i32);
        for dy in -REACH..=REACH {
            for dx in -REACH..=REACH {
                let (hit, _) = score(&mut changed.iter().step_by(step).copied(), dx, dy);
                if hit > best.0 {
                    best = (hit, dx, dy);
                }
            }
        }
        let sampled = changed.len().div_ceil(step) as f32;
        let (_, gx, gy) = best;
        if (gx, gy) == (0, 0) || (best.0 as f32) < LEAST_MATCH * sampled {
            return None;
        }
        let bands = h.div_ceil(BAND);
        self.shifts.clear();
        let mut at = 0;
        for b in 0..bands {
            let end = changed[at..].partition_point(|&i| i / w < (b + 1) * BAND) + at;
            let band = &changed[at..end];
            at = end;
            let mut pick = (0u32, gx, gy);
            if band.len() >= 64 {
                for dy in gy - 1..=gy + 1 {
                    for dx in gx - 1..=gx + 1 {
                        let (hit, _) = score(&mut band.iter().copied(), dx, dy);
                        if hit > pick.0 {
                            pick = (hit, dx, dy);
                        }
                    }
                }
            }
            self.shifts.push((pick.1, pick.2));
        }
        let shift = |y: usize| self.shifts[(y / BAND).min(bands - 1)];

        // What stays put, in each picture: a vote of the neighbourhood between "where it was"
        // and "moved with the scroll" (pixels both or neither explain don't vote).
        let votes =
            |still: &mut Vec<bool>, vote: &mut Vec<i32>, tmp: &mut Vec<i32>, old_side: bool| {
                vote.clear();
                vote.extend((0..w * h).map(|i| {
                    let (dx, dy) = shift(i / w);
                    let stays = new[i] == old[i];
                    let scrolls = if old_side {
                        // The old pixel at i, moved: where is it in the new picture?
                        let (x, y) = ((i % w) as i32 + dx, (i / w) as i32 + dy);
                        x >= 0
                            && y >= 0
                            && x < w as i32
                            && y < h as i32
                            && new[y as usize * w + x as usize] == old[i]
                    } else {
                        moved(i, dx, dy)
                    };
                    i32::from(stays && !scrolls) - i32::from(scrolls && !stays)
                }));
                box_sum(vote, tmp, w, h, VOTE);
                still.clear();
                still.extend(vote.iter().map(|&v| v > 0));
            };
        let (mut still_new, mut still_old) = (
            std::mem::take(&mut self.still_new),
            std::mem::take(&mut self.still_old),
        );
        let (mut vote, mut tmp) = (
            std::mem::take(&mut self.vote),
            std::mem::take(&mut self.vote_tmp),
        );
        votes(&mut still_new, &mut vote, &mut tmp, false);
        votes(&mut still_old, &mut vote, &mut tmp, true);

        // Twice the size: the new picture's scrolling part half a step back (s at 2x is half
        // of s DS pixels), what stays put where it is.
        let (w2, h2) = (2 * w, 2 * h);
        let mut out = vec![0u32; w2 * h2];
        let ds = |x: i32, y: i32| -> Option<usize> {
            let (x, y) = (x.div_euclid(2), y.div_euclid(2));
            (x >= 0 && y >= 0 && x < w as i32 && y < h as i32).then(|| y as usize * w + x as usize)
        };
        for y2 in 0..h2 {
            let (sx, sy) = shift(y2 / 2);
            for x2 in 0..w2 {
                let here = (y2 / 2) * w + x2 / 2;
                let (x, y) = (x2 as i32, y2 as i32);
                out[y2 * w2 + x2] = if still_new[here] {
                    lit[here]
                } else {
                    match ds(x + sx, y + sy) {
                        Some(j) if !still_new[j] => lit[j],
                        from_new => match ds(x - sx, y - sy) {
                            // Behind what stays put: the old picture, moved forward half a step.
                            Some(j) if !still_old[j] => self.lit[j],
                            _ => lit[from_new.unwrap_or(here)],
                        },
                    }
                };
            }
        }
        self.still_new = still_new;
        self.still_old = still_old;
        self.vote = vote;
        self.vote_tmp = tmp;
        Some(out)
    }
}

/// The top picture's height (DS pixels).
pub const HEIGHT: usize = 192;

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

    /// A textured "world" seen through a `w` x 192 window at (ox, oy), with a "player" (a
    /// solid square) fixed at the window's middle.
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
        let a = view(0, 0, w);
        let b = view(4, 2, w);
        let mut s = Smooth::new();
        assert!(s.frame(10, &a, &a, w).is_none());
        assert!(s.frame(11, &a, &a, w).is_none(), "a repeat");
        let mid = s.frame(12, &b, &b, w).expect("half way");
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
    fn no_half_way_at_60_a_second_or_without_a_scroll() {
        let w = 256;
        let mut s = Smooth::new();
        s.frame(1, &view(0, 0, w), &view(0, 0, w), w);
        // The next frame already new: shown as it is.
        assert!(s.frame(2, &view(1, 0, w), &view(1, 0, w), w).is_none());
        // A change that isn't a scroll (a fade): shown as it is.
        s.frame(3, &view(1, 0, w), &view(1, 0, w), w);
        let fade: Vec<u32> = view(1, 0, w).iter().map(|p| p / 2).collect();
        assert!(s.frame(4, &fade, &fade, w).is_none());
    }
}
