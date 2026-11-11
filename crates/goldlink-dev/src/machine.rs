//! The emulated DS a dev run drives: the core, its ROM and save file, the next frame's input,
//! the speed cap, and BizHawk's save-file behaviour (LUA_SHIM_SPEC §4.5).

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use goldlink_core::{FirmwareUser, Keys, Nds, SCREEN_HEIGHT, SCREEN_WIDTH};

/// The DS's frame rate (33,513,982 Hz / 560,190 cycles per frame).
pub const DS_FPS: f64 = 59.826_098_5;

/// The stylus position `joypad.set` leaves when it isn't given one: the screen's centre.
pub const TOUCH_CENTRE: (u16, u16) = (128, 96);

pub struct Machine {
    pub nds: Nds,
    /// The real co-op bridge (EmuHawk's form with `--open-ext-tool-dll`), run after every frame.
    pub bridge: Option<crate::devbridge::DevTicker>,
    rom: Vec<u8>,
    save_path: Option<PathBuf>,
    pub rtc: Option<chrono::NaiveDateTime>,
    /// The next frame's input (BizHawk's joypad.set / setanalog: one frame only).
    pub keys: Keys,
    pub touching: bool,
    pub touch: (u16, u16),
    /// client.speedmode percent; None = uncapped.
    pub speed: Option<f64>,
    uncapped: bool,
    next_frame_at: Option<Instant>,
    /// GOLDLINK_DEV_BATTLE_LOG=<folder>: what the one-screen battle HUD reads, each time it changes
    /// (UC9 §3.2: checks the reader in team and link battles run by the e2e tests).
    battle_log: Option<(std::fs::File, String)>,
}

impl Machine {
    pub fn new(
        rom: Vec<u8>,
        save_path: Option<PathBuf>,
        rtc: Option<chrono::NaiveDateTime>,
        uncapped: bool,
    ) -> Result<Self, String> {
        let nds = Nds::new(&FirmwareUser::default(), None).map_err(|e| e.to_string())?;
        // A folder: each process writes battle-<pid>.log there (the e2e runners start several).
        let battle_log = std::env::var_os("GOLDLINK_DEV_BATTLE_LOG")
            .map(PathBuf::from)
            .and_then(|d| {
                std::fs::create_dir_all(&d).ok()?;
                std::fs::File::create(d.join(format!("battle-{}.log", std::process::id()))).ok()
            })
            .map(|f| (f, String::new()));
        let mut m = Self {
            battle_log,
            nds,
            bridge: None,
            rom,
            save_path,
            rtc,
            keys: Keys::NONE,
            touching: false,
            touch: TOUCH_CENTRE,
            speed: Some(100.0),
            uncapped,
            next_frame_at: None,
        };
        m.power_on()?;
        Ok(m)
    }

    /// Loads the ROM with the save file's contents (BizHawk reads the .SaveRAM at boot and on
    /// reboot_core) and sets the clock.
    pub fn power_on(&mut self) -> Result<(), String> {
        let save = match &self.save_path {
            Some(p) if p.is_file() => {
                Some(std::fs::read(p).map_err(|e| format!("{}: {e}", p.display()))?)
            }
            _ => None,
        };
        self.nds
            .load_rom(&self.rom, save.as_deref())
            .map_err(|e| e.to_string())?;
        match self.rtc {
            Some(at) => self.nds.set_rtc(at),
            None => self.nds.set_rtc_now(),
        }
        Ok(())
    }

    /// Writes the save memory to the save file (client.saveram, and on exit as BizHawk does).
    pub fn flush_save(&self) -> Result<(), String> {
        let Some(path) = &self.save_path else {
            return Ok(());
        };
        let save = self.nds.save();
        if save.is_empty() {
            return Ok(());
        }
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        }
        let tmp = path.with_extension("SaveRAM.tmp");
        std::fs::write(&tmp, save)
            .and_then(|_| std::fs::rename(&tmp, path))
            .map_err(|e| format!("{}: {e}", path.display()))
    }

    /// Runs one frame with the pending input, then clears it (joypad.set lasts one frame), and
    /// waits to keep the speed cap.
    pub fn frame(&mut self) {
        let touch = self.touching.then_some(self.touch);
        self.nds.set_input(self.keys, touch);
        if let Some(d) = self.bridge.as_ref() {
            goldlink::goldlink_bridge::hook::before_frame(&d.bridge, &mut self.nds, crate::MAILBOX);
        }
        self.nds.run_frame();
        self.log_battle();
        if let Some(d) = self.bridge.as_mut() {
            // A world's save swapped into the cart reaches the .SaveRAM when the run ends, as
            // BizHawk writes it on close.
            goldlink::goldlink_bridge::hook::after_frame(&d.bridge, &mut self.nds, crate::MAILBOX);
            d.tick();
        }
        self.keys = Keys::NONE;
        self.touching = false;
        self.touch = TOUCH_CENTRE;
        self.pace();
    }

    fn pace(&mut self) {
        let Some(pct) = self.speed.filter(|_| !self.uncapped) else {
            self.next_frame_at = None;
            return;
        };
        let period = Duration::from_secs_f64(1.0 / (DS_FPS * pct.max(1.0) / 100.0));
        let now = Instant::now();
        let due = self.next_frame_at.map_or(now + period, |t| t + period);
        if due > now {
            std::thread::sleep(due - now);
            self.next_frame_at = Some(due);
        } else {
            // Behind (a slow frame): don't try to catch up.
            self.next_frame_at = Some(now);
        }
    }

    /// The top screen widened by `side` pixels each side (UC9 §4), as an RGB PNG.
    pub fn wide_screenshot(&self, path: &Path, side: usize) -> Result<(), String> {
        let frame = self.nds.wide_frame(0, side);
        let w = frame.len() / SCREEN_HEIGHT;
        let mut rgb = Vec::with_capacity(frame.len() * 3);
        for &px in &frame {
            rgb.extend_from_slice(&[(px >> 16) as u8, (px >> 8) as u8, px as u8]);
        }
        let file = std::fs::File::create(path).map_err(|e| format!("{}: {e}", path.display()))?;
        let mut enc = png::Encoder::new(
            std::io::BufWriter::new(file),
            w as u32,
            SCREEN_HEIGHT as u32,
        );
        enc.set_color(png::ColorType::Rgb);
        enc.set_depth(png::BitDepth::Eight);
        let mut writer = enc.write_header().map_err(|e| e.to_string())?;
        writer.write_image_data(&rgb).map_err(|e| e.to_string())
    }

    /// One line for GOLDLINK_DEV_BATTLE_LOG when what the battle HUD reads changes (HP while a
    /// bar drains is left out of the comparison, so a drain is one line).
    fn log_battle(&mut self) {
        use std::io::Write;
        let Some((file, last)) = self.battle_log.as_mut() else {
            return;
        };
        let line = match goldlink::goldlink_bridge::battle_ui::read(self.nds.main_ram()) {
            None => "no battle".to_owned(),
            Some(b) => {
                let boxes: Vec<String> = b
                    .boxes
                    .iter()
                    .map(|x| {
                        format!(
                            "[battler {} kind {} at {},{} drawn {} hp {}/{} lv {}]",
                            x.battler, x.kind, x.x, x.y, x.drawn, x.hp, x.max_hp, x.level
                        )
                    })
                    .collect();
                let targets: Vec<String> = b
                    .targets
                    .iter()
                    .enumerate()
                    .filter(|(_, t)| t.shown)
                    .map(|(i, t)| format!("{i}:{}/{}", t.hp, t.max_hp))
                    .collect();
                format!(
                    "menu {} waiting {} cursor {:?} back {} bag/party {} targets [{}] boxes {}",
                    b.menu,
                    b.waiting,
                    b.cursor,
                    b.run_is_back,
                    b.bag_or_party,
                    targets.join(" "),
                    boxes.join(" ")
                )
            }
        };
        if line != *last {
            writeln!(file, "f{} {line}", self.nds.frame_count()).ok();
            *last = line;
        }
    }

    /// Both screens, top above bottom with no gap (BizHawk's ScreenLayout 0, ScreenGap 0),
    /// as an RGB PNG.
    pub fn screenshot(&self, path: &Path) -> Result<(), String> {
        let (w, h) = (SCREEN_WIDTH, SCREEN_HEIGHT * 2);
        let mut rgb = Vec::with_capacity(w * h * 3);
        for screen in 0..2 {
            for &px in self.nds.framebuffer(screen) {
                rgb.extend_from_slice(&[(px >> 16) as u8, (px >> 8) as u8, px as u8]);
            }
        }
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).ok();
        }
        let file = std::fs::File::create(path).map_err(|e| format!("{}: {e}", path.display()))?;
        let mut enc = png::Encoder::new(std::io::BufWriter::new(file), w as u32, h as u32);
        enc.set_color(png::ColorType::Rgb);
        enc.set_depth(png::BitDepth::Eight);
        let mut writer = enc.write_header().map_err(|e| e.to_string())?;
        writer.write_image_data(&rgb).map_err(|e| e.to_string())
    }
}
