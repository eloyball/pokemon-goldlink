//! The player's controls → the DS's buttons: keyboard (egui key names), gamepads (gilrs) and the
//! mouse as the stylus.

use eframe::egui;
use goldlink_core::Keys;
use goldlink_runtime::config::Bindings;

/// Resolved bindings: egui keys and gilrs buttons per DS button.
pub struct Controls {
    keys: Vec<(Keys, Vec<egui::Key>)>,
    pad: Vec<(Keys, Vec<gilrs::Button>)>,
}

impl Controls {
    pub fn new(keys: &Bindings, pad: &Bindings) -> Self {
        Self {
            keys: keys
                .iter()
                .map(|(k, names)| {
                    (
                        k,
                        names
                            .iter()
                            .filter_map(|n| egui::Key::from_name(n))
                            .collect(),
                    )
                })
                .collect(),
            pad: pad
                .iter()
                .map(|(k, names)| (k, names.iter().filter_map(|n| pad_button(n)).collect()))
                .collect(),
        }
    }

    /// The DS buttons held on the keyboard.
    pub fn keyboard(&self, held: &std::collections::HashSet<egui::Key>) -> Keys {
        let mut out = Keys::NONE;
        for (k, keys) in &self.keys {
            if keys.iter().any(|key| held.contains(key)) {
                out |= *k;
            }
        }
        out
    }

    /// The DS buttons held on any connected gamepad (sticks count as the d-pad).
    pub fn gamepads(&self, gilrs: &gilrs::Gilrs) -> Keys {
        let mut out = Keys::NONE;
        for (_, pad) in gilrs.gamepads() {
            for (k, buttons) in &self.pad {
                if buttons.iter().any(|b| pad.is_pressed(*b)) {
                    out |= *k;
                }
            }
            let x = pad.value(gilrs::Axis::LeftStickX);
            let y = pad.value(gilrs::Axis::LeftStickY);
            const DEAD: f32 = 0.5;
            if x < -DEAD {
                out |= Keys::LEFT;
            }
            if x > DEAD {
                out |= Keys::RIGHT;
            }
            if y > DEAD {
                out |= Keys::UP;
            }
            if y < -DEAD {
                out |= Keys::DOWN;
            }
        }
        out
    }
}

/// gilrs button by its name in `goldlink.toml`.
pub fn pad_button(name: &str) -> Option<gilrs::Button> {
    use gilrs::Button::*;
    Some(match name {
        "South" => South,
        "East" => East,
        "North" => North,
        "West" => West,
        "LeftTrigger" => LeftTrigger,
        "LeftTrigger2" => LeftTrigger2,
        "RightTrigger" => RightTrigger,
        "RightTrigger2" => RightTrigger2,
        "Select" => Select,
        "Start" => Start,
        "Mode" => Mode,
        "LeftThumb" => LeftThumb,
        "RightThumb" => RightThumb,
        "DPadUp" => DPadUp,
        "DPadDown" => DPadDown,
        "DPadLeft" => DPadLeft,
        "DPadRight" => DPadRight,
        _ => return None,
    })
}

/// The stylus position for a pointer at `pos` over the bottom screen drawn in `rect`.
pub fn touch_at(rect: egui::Rect, pos: egui::Pos2) -> Option<(u16, u16)> {
    if !rect.contains(pos) || rect.width() <= 0.0 || rect.height() <= 0.0 {
        return None;
    }
    let x = ((pos.x - rect.min.x) / rect.width() * 256.0).clamp(0.0, 255.0);
    let y = ((pos.y - rect.min.y) / rect.height() * 192.0).clamp(0.0, 191.0);
    Some((x as u16, y as u16))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_bindings_resolve() {
        let c = Controls::new(&Bindings::keyboard(), &Bindings::gamepad());
        assert!(
            c.keys.iter().all(|(_, keys)| !keys.is_empty()),
            "every key name is egui's"
        );
        assert!(
            c.pad.iter().all(|(_, b)| !b.is_empty()),
            "every pad name is gilrs's"
        );
        let held: std::collections::HashSet<_> = [egui::Key::X, egui::Key::ArrowUp].into();
        assert_eq!(c.keyboard(&held), Keys::A | Keys::UP);
    }

    #[test]
    fn touch_maps_to_ds_pixels() {
        let r = egui::Rect::from_min_size(egui::pos2(10.0, 400.0), egui::vec2(512.0, 384.0));
        assert_eq!(touch_at(r, egui::pos2(10.0, 400.0)), Some((0, 0)));
        assert_eq!(touch_at(r, egui::pos2(266.0, 592.0)), Some((128, 96)));
        assert_eq!(touch_at(r, egui::pos2(521.9, 783.9)), Some((255, 191)));
        assert_eq!(touch_at(r, egui::pos2(5.0, 500.0)), None);
    }
}
