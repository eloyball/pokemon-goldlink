//! `goldlink.toml`, the player's settings (RFC §4.4): screen layout, audio, controls. One file in
//! the data folder; a missing or damaged file gives the defaults, and unknown keys are ignored so
//! older and newer versions can share it.

use std::path::Path;

use serde::{Deserialize, Serialize};

pub const FILE: &str = "goldlink.toml";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub video: Video,
    pub audio: Audio,
    /// Keyboard keys per DS button, by egui key name ("X", "ArrowUp", "Enter", …).
    pub keys: Bindings,
    /// Gamepad buttons per DS button, by gilrs button name ("South", "DPadUp", "Start", …).
    pub pad: Bindings,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Layout {
    /// Top screen above the bottom one (the DS held normally).
    Vertical,
    /// Side by side.
    Horizontal,
    /// The top screen alone; the touch screen's menus drawn over it as GoldLink's own (the
    /// battle HUD) or as a modal when the game needs it (UC9 §3).
    OneScreen,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Filter {
    /// Sharp pixels.
    Nearest,
    /// Smoothed.
    Linear,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Video {
    pub layout: Layout,
    /// Pixels between the screens, at 1x.
    pub gap: u32,
    pub filter: Filter,
    /// Scale by whole numbers only (crisper, may leave borders).
    pub integer_scale: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Audio {
    /// 0..=100.
    pub volume: u8,
    pub muted: bool,
}

/// DS button → the keys or pad buttons that press it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Bindings {
    pub a: Vec<String>,
    pub b: Vec<String>,
    pub x: Vec<String>,
    pub y: Vec<String>,
    pub l: Vec<String>,
    pub r: Vec<String>,
    pub start: Vec<String>,
    pub select: Vec<String>,
    pub up: Vec<String>,
    pub down: Vec<String>,
    pub left: Vec<String>,
    pub right: Vec<String>,
}

fn names(list: &[&str]) -> Vec<String> {
    list.iter().map(|s| (*s).to_owned()).collect()
}

impl Default for Video {
    fn default() -> Self {
        Self {
            layout: Layout::Vertical,
            gap: 0,
            filter: Filter::Nearest,
            integer_scale: false,
        }
    }
}

impl Default for Audio {
    fn default() -> Self {
        Self {
            volume: 100,
            muted: false,
        }
    }
}

impl Bindings {
    /// The keyboard controls of the BizHawk version's releases, without BizHawk's Tab
    /// (fast-forward) and Shift extras: the game always runs at the DS's speed (D5.6).
    pub fn keyboard() -> Self {
        Self {
            a: names(&["X", "K"]),
            b: names(&["Z", "J"]),
            x: names(&["V"]),
            y: names(&["C"]),
            l: names(&["Q"]),
            r: names(&["E"]),
            start: names(&["Enter"]),
            select: names(&["Space"]),
            up: names(&["W", "ArrowUp"]),
            down: names(&["S", "ArrowDown"]),
            left: names(&["A", "ArrowLeft"]),
            right: names(&["D", "ArrowRight"]),
        }
    }

    /// BizHawk's XInput mapping: the pad's A/B/X/Y (Xbox labels) are the DS's.
    pub fn gamepad() -> Self {
        Self {
            a: names(&["South"]),
            b: names(&["East"]),
            x: names(&["West"]),
            y: names(&["North"]),
            l: names(&["LeftTrigger"]),
            r: names(&["RightTrigger"]),
            start: names(&["Start"]),
            select: names(&["Select"]),
            up: names(&["DPadUp"]),
            down: names(&["DPadDown"]),
            left: names(&["DPadLeft"]),
            right: names(&["DPadRight"]),
        }
    }

    /// `(DS button, names)` for every button.
    pub fn iter(&self) -> impl Iterator<Item = (goldlink_core::Keys, &[String])> {
        use goldlink_core::Keys;
        [
            (Keys::A, &self.a),
            (Keys::B, &self.b),
            (Keys::X, &self.x),
            (Keys::Y, &self.y),
            (Keys::L, &self.l),
            (Keys::R, &self.r),
            (Keys::START, &self.start),
            (Keys::SELECT, &self.select),
            (Keys::UP, &self.up),
            (Keys::DOWN, &self.down),
            (Keys::LEFT, &self.left),
            (Keys::RIGHT, &self.right),
        ]
        .into_iter()
        .map(|(k, v)| (k, v.as_slice()))
    }
}

impl Default for Bindings {
    fn default() -> Self {
        Self::keyboard()
    }
}

impl Default for Config {
    fn default() -> Self {
        Self {
            video: Video::default(),
            audio: Audio::default(),
            keys: Bindings::keyboard(),
            pad: Bindings::gamepad(),
        }
    }
}

impl Config {
    /// The settings in `dir`, or the defaults (with the reason, when the file was unreadable).
    pub fn load(dir: &Path) -> (Self, Option<String>) {
        let path = dir.join(FILE);
        match std::fs::read_to_string(&path) {
            Ok(text) => match toml::from_str(&text) {
                Ok(c) => (c, None),
                Err(e) => (
                    Self::default(),
                    Some(format!(
                        "{}: {e}; using the default settings",
                        path.display()
                    )),
                ),
            },
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => (Self::default(), None),
            Err(e) => (
                Self::default(),
                Some(format!(
                    "{}: {e}; using the default settings",
                    path.display()
                )),
            ),
        }
    }

    pub fn save(&self, dir: &Path) -> Result<(), String> {
        let text = toml::to_string_pretty(self).map_err(|e| e.to_string())?;
        crate::files::write_atomic(&dir.join(FILE), text.as_bytes())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_and_partial_files() {
        let c = Config::default();
        let text = toml::to_string_pretty(&c).unwrap();
        assert_eq!(toml::from_str::<Config>(&text).unwrap(), c);
        // A file with one section keeps the defaults for the rest.
        let partial: Config = toml::from_str("[audio]\nvolume = 40\n").unwrap();
        assert_eq!(partial.audio.volume, 40);
        assert_eq!(partial.keys, Bindings::keyboard());
        assert_eq!(partial.video, Video::default());
        // Unknown keys from a newer version are ignored.
        assert!(toml::from_str::<Config>("future = 1\n[video]\nshader = \"crt\"\n").is_ok());
    }

    #[test]
    fn damaged_file_gives_defaults() {
        let dir = std::env::temp_dir().join(format!("goldlink-config-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(FILE), "[video\nlayout = ").unwrap();
        let (c, err) = Config::load(&dir);
        assert_eq!(c, Config::default());
        assert!(err.is_some());
        std::fs::remove_dir_all(&dir).ok();
    }
}
