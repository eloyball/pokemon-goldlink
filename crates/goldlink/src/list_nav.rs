//! Up and Down in the one-screen layout's white box walk it as the list it shows, when the game's
//! own buttons stand in a grid (the PC's storage menu: two columns, so the game's Down kept to a
//! column and skipped half the box's items; the user's v0.7.2 play test). Each Up or Down picks
//! the box's previous or next item (wrapping, as the game's lists do), and the game's own arrows
//! are pressed, one at a time, until its selected button (the red outline) is that one. Left and
//! Right go to the game as they are; a menu in one column is left alone.

use std::time::{Duration, Instant};

use goldlink_core::Keys;

use crate::hud::TouchChoice;

/// How long a key is held for the game, and how long a press may go unanswered (the selection
/// not moving: the game ignored it) before the walk gives up.
const PRESS: Duration = Duration::from_millis(50);
const ANSWER: Duration = Duration::from_millis(400);
/// A held Up or Down repeats after this, then this often (as the game's own lists do).
const REPEAT_AFTER: Duration = Duration::from_millis(400);
const REPEAT_EVERY: Duration = Duration::from_millis(150);
/// Buttons whose tops are this close (DS pixels) share a row.
const ROW: usize = 4;

#[derive(Default)]
pub struct ListNav {
    /// The player's Up or Down held last frame, and when its next repeat is due.
    held: Option<(Keys, Instant)>,
    /// The item being walked to.
    target: Option<usize>,
    /// The game's arrow being pressed for the walk: since when, and the selection then.
    press: Option<(Keys, Instant, usize)>,
}

impl ListNav {
    /// The keys for the game this frame, from the player's (`keys`): Up and Down taken over while
    /// the box (`shown`) lists `choices` in a grid.
    pub fn keys(&mut self, keys: Keys, shown: bool, choices: &[TouchChoice], now: Instant) -> Keys {
        if !(shown && is_grid(choices)) {
            *self = Self::default();
            return keys;
        }
        // A frame without a selected button (one changing over): the walk waits, Up and Down
        // kept from the game.
        let Some(selected) = choices.iter().position(|c| c.selected) else {
            return Keys(keys.0 & !(Keys::UP.0 | Keys::DOWN.0));
        };
        let vertical = [Keys::UP, Keys::DOWN].into_iter().find(|&k| {
            keys.contains(k) && !keys.contains(if k == Keys::UP { Keys::DOWN } else { Keys::UP })
        });
        let mut out = Keys(keys.0 & !(Keys::UP.0 | Keys::DOWN.0));
        // The player's Up or Down: a step through the list, at once and then repeating.
        let step = match (vertical, self.held) {
            (Some(k), Some((h, due))) if k == h => (now >= due).then(|| {
                self.held = Some((k, now + REPEAT_EVERY));
                k
            }),
            (Some(k), _) => {
                self.held = Some((k, now + REPEAT_AFTER));
                Some(k)
            }
            (None, _) => {
                self.held = None;
                None
            }
        };
        if let Some(k) = step {
            let from = self.target.unwrap_or(selected);
            let n = choices.len();
            self.target = Some(if k == Keys::DOWN {
                (from + 1) % n
            } else {
                (from + n - 1) % n
            });
        }
        // Left or Right from the player: theirs to steer.
        if keys.contains(Keys::LEFT) || keys.contains(Keys::RIGHT) {
            self.target = None;
            self.press = None;
            return out;
        }
        let Some(target) = self.target else {
            return out;
        };
        if target == selected || target >= choices.len() {
            self.target = None;
            self.press = None;
            return out;
        }
        match self.press {
            // Held for the game a moment, then let go until the selection answers.
            Some((k, since, _)) if now.duration_since(since) < PRESS => out |= k,
            Some((_, since, before)) if selected == before => {
                if now.duration_since(since) > ANSWER {
                    // The game didn't move: leave it where it is.
                    self.target = None;
                    self.press = None;
                }
            }
            // Let go a moment before the next press: the game moves on a new press, and the
            // same arrow held on wouldn't be one.
            Some((_, since, _)) if now.duration_since(since) < 2 * PRESS => {}
            _ => {
                if let Some(k) = toward(choices, selected, target) {
                    self.press = Some((k, now, selected));
                    out |= k;
                } else {
                    self.target = None;
                }
            }
        }
        out
    }
}

impl ListNav {
    /// The item the box's arrow points at: where a walk is going, else the game's selection.
    pub fn shown(&self, choices: &[TouchChoice]) -> Option<usize> {
        self.target
            .filter(|&t| t < choices.len())
            .or_else(|| choices.iter().position(|c| c.selected))
    }
}

/// Whether the buttons stand in more than one column.
fn is_grid(choices: &[TouchChoice]) -> bool {
    choices
        .iter()
        .any(|c| c.x0.abs_diff(choices[0].x0) > ROW && c.x1.abs_diff(choices[0].x1) > ROW)
}

fn same_row(a: &TouchChoice, b: &TouchChoice) -> bool {
    a.y0.abs_diff(b.y0) <= ROW
}

fn same_column(a: &TouchChoice, b: &TouchChoice) -> bool {
    a.x0.abs_diff(b.x0) <= ROW
}

/// The game's arrow that takes the selection from `from` toward `to`: across first when this row
/// has a button in the target's column (so a column that ends early isn't walked off), else up
/// or down.
fn toward(choices: &[TouchChoice], from: usize, to: usize) -> Option<Keys> {
    let (a, b) = (&choices[from], &choices[to]);
    let across = || if b.x0 < a.x0 { Keys::LEFT } else { Keys::RIGHT };
    let vertical = || if b.y0 < a.y0 { Keys::UP } else { Keys::DOWN };
    if same_row(a, b) {
        return (!same_column(a, b)).then(across);
    }
    if same_column(a, b) {
        return Some(vertical());
    }
    let column_here = choices.iter().any(|c| same_row(c, a) && same_column(c, b));
    Some(if column_here { across() } else { vertical() })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn button(x0: usize, y0: usize, selected: bool) -> TouchChoice {
        TouchChoice {
            x0,
            y0,
            x1: x0 + 112,
            y1: y0 + 36,
            selected,
        }
    }

    /// The PC's storage menu (measured): DEPOSIT, WITHDRAW / MOVE POKéMON, MOVE ITEMS / SEE YA!
    fn pc(selected: usize) -> Vec<TouchChoice> {
        [(8, 30), (136, 30), (8, 78), (136, 78), (136, 126)]
            .iter()
            .enumerate()
            .map(|(i, &(x, y))| button(x, y, i == selected))
            .collect()
    }

    /// The game's grid as the PC's menu moves on an arrow (down past the left column's end does
    /// nothing).
    fn game(selected: usize, k: Keys) -> usize {
        match (selected, k) {
            (0, Keys::RIGHT) => 1,
            (0, Keys::DOWN) => 2,
            (1, Keys::LEFT) => 0,
            (1, Keys::DOWN) => 3,
            (2, Keys::UP) => 0,
            (2, Keys::RIGHT) => 3,
            (3, Keys::UP) => 1,
            (3, Keys::LEFT) => 2,
            (3, Keys::DOWN) => 4,
            (4, Keys::UP) => 3,
            (s, _) => s,
        }
    }

    /// Plays a press of `k` by the player and lets the walk run against the game.
    fn walk(nav: &mut ListNav, selected: &mut usize, k: Keys, t: &mut Instant) {
        let mut sent_before = Keys::NONE;
        for frame in 0..120 {
            let player = if frame < 2 { k } else { Keys::NONE };
            let sent = nav.keys(player, true, &pc(*selected), *t);
            // The game moves on a new press of an arrow.
            for a in [Keys::UP, Keys::DOWN, Keys::LEFT, Keys::RIGHT] {
                if sent.contains(a) && !sent_before.contains(a) {
                    *selected = game(*selected, a);
                }
            }
            sent_before = sent;
            *t += Duration::from_millis(16);
        }
    }

    #[test]
    fn down_and_up_walk_the_list_in_order() {
        let (mut nav, mut selected, mut t) = (ListNav::default(), 0, Instant::now());
        let mut seen = vec![selected];
        for _ in 0..5 {
            walk(&mut nav, &mut selected, Keys::DOWN, &mut t);
            seen.push(selected);
        }
        assert_eq!(seen, [0, 1, 2, 3, 4, 0]);
        let mut back = vec![selected];
        for _ in 0..5 {
            walk(&mut nav, &mut selected, Keys::UP, &mut t);
            back.push(selected);
        }
        assert_eq!(back, [0, 4, 3, 2, 1, 0]);
    }

    /// The box's arrow goes straight to the item being walked to (Withdraw to Move Pokémon
    /// passes Deposit in the game's grid), and back to the game's selection after.
    #[test]
    fn the_arrow_waits_at_where_the_walk_goes() {
        let (mut nav, mut t) = (ListNav::default(), Instant::now());
        let mut selected = 1;
        let mut sent_before = Keys::NONE;
        let mut shown = Vec::new();
        for frame in 0..60 {
            let player = if frame < 2 { Keys::DOWN } else { Keys::NONE };
            let sent = nav.keys(player, true, &pc(selected), t);
            for a in [Keys::UP, Keys::DOWN, Keys::LEFT, Keys::RIGHT] {
                if sent.contains(a) && !sent_before.contains(a) {
                    selected = game(selected, a);
                }
            }
            sent_before = sent;
            shown.push(nav.shown(&pc(selected)).unwrap());
            t += Duration::from_millis(16);
        }
        assert_eq!(selected, 2);
        assert!(shown.iter().all(|&k| k == 2), "{shown:?}");
    }

    #[test]
    fn one_column_and_sideways_keys_are_left_alone() {
        let column: Vec<TouchChoice> = (0..3).map(|i| button(8, 30 + 48 * i, i == 0)).collect();
        let mut nav = ListNav::default();
        let t = Instant::now();
        assert_eq!(nav.keys(Keys::DOWN, true, &column, t), Keys::DOWN);
        assert_eq!(nav.keys(Keys::RIGHT, true, &pc(0), t), Keys::RIGHT);
        // Without the box, Down goes to the game.
        assert_eq!(nav.keys(Keys::DOWN, false, &pc(0), t), Keys::DOWN);
    }
}
