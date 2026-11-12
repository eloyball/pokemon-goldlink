//! Typing a name on the computer's keyboard (UC9): while the game's naming screen is open, each
//! typed character becomes a tap on its key on the touch screen (the page's button first when
//! it's on another page), Backspace taps BACK and Enter taps OK. The game takes the taps as its
//! own input, so its length limit, sounds and the finished name are unchanged.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

use eframe::egui;
use goldlink_bridge::naming::{self, Naming};

/// How long a tap is held, and the pause after it (the game reads a new touch each frame).
const HOLD: Duration = Duration::from_millis(67);
const GAP: Duration = Duration::from_millis(50);
/// Typed keys the game hasn't taken after this long are dropped (the screen stopped taking
/// input: it's closing, or the name is full and nothing comes free).
const STALE: Duration = Duration::from_secs(4);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Typed {
    Char(char),
    Back,
    Ok,
}

#[derive(Debug, Default)]
pub struct Typing {
    /// The naming screen, while it's open.
    pub naming: Option<Naming>,
    queue: VecDeque<Typed>,
    /// The tap being made, and when it began.
    tap: Option<((u16, u16), Instant)>,
    /// When the last tap ended, or the queue last moved.
    last: Option<Instant>,
}

impl Typing {
    /// Follows the game's naming screen; takes the typed text and keys from this frame's events
    /// when `listen` (the game window has the keyboard).
    pub fn update(&mut self, naming: Option<Naming>, events: &[egui::Event], listen: bool) {
        self.naming = naming;
        let Some(n) = naming else {
            self.queue.clear();
            self.tap = None;
            return;
        };
        if listen {
            for e in events {
                match e {
                    egui::Event::Text(t) => self.queue.extend(t.chars().map(Typed::Char)),
                    egui::Event::Key {
                        key: egui::Key::Backspace,
                        pressed: true,
                        ..
                    } => self.queue.push_back(Typed::Back),
                    egui::Event::Key {
                        key: egui::Key::Enter,
                        pressed: true,
                        ..
                    } => self.queue.push_back(Typed::Ok),
                    _ => {}
                }
            }
        }
        let now = Instant::now();
        if let Some((_, since)) = self.tap {
            if now - since < HOLD {
                return;
            }
            self.tap = None;
            self.last = Some(now);
            return;
        }
        if self.queue.is_empty() {
            self.last = None;
            return;
        }
        let last = *self.last.get_or_insert(now);
        if now - last < GAP {
            return;
        }
        if !n.ready {
            if now - last > STALE {
                self.queue.clear();
            }
            return;
        }
        let at = match self.queue[0] {
            Typed::Back => {
                self.queue.pop_front();
                naming::BACK
            }
            Typed::Ok => {
                self.queue.pop_front();
                naming::OK
            }
            Typed::Char(c) => match naming::tap_for(c, n.page) {
                Some((at, typed)) => {
                    if typed {
                        self.queue.pop_front();
                    }
                    at
                }
                None => {
                    self.queue.pop_front();
                    return;
                }
            },
        };
        self.tap = Some((at, now));
    }

    /// The touch this frame, when a typed key is being tapped.
    pub fn touch(&self) -> Option<(u16, u16)> {
        self.tap.map(|(at, _)| at)
    }
}
