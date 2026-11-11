//! A safe Rust Nintendo DS over melonDS (unified-client RFC §2.2): one [`Nds`] per emulated DS.
//! Several can run in one process (each owns its melonDS instance).
//!
//! The co-op code reads and writes main RAM between frames ([`Nds::main_ram_mut`]), the way the
//! C# bridge does through BizHawk's "Main RAM" domain, at the same offsets
//! (`ARM9 address - 0x02000000`).

use std::ffi::{CStr, c_char, c_int, c_void};
use std::ptr::NonNull;

use chrono::{Datelike, Local, NaiveDateTime, Timelike};
use goldlink_core_sys as sys;

pub use sys::{GL_SCREEN_HEIGHT as SCREEN_HEIGHT, GL_SCREEN_WIDTH as SCREEN_WIDTH};

/// The ARM9 address main RAM starts at.
pub const MAIN_RAM_BASE: u32 = 0x0200_0000;

/// The DS's buttons, for [`Nds::set_input`]. Combine with `|`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Keys(pub u32);

impl Keys {
    pub const A: Keys = Keys(1 << 0);
    pub const B: Keys = Keys(1 << 1);
    pub const SELECT: Keys = Keys(1 << 2);
    pub const START: Keys = Keys(1 << 3);
    pub const RIGHT: Keys = Keys(1 << 4);
    pub const LEFT: Keys = Keys(1 << 5);
    pub const UP: Keys = Keys(1 << 6);
    pub const DOWN: Keys = Keys(1 << 7);
    pub const R: Keys = Keys(1 << 8);
    pub const L: Keys = Keys(1 << 9);
    pub const X: Keys = Keys(1 << 10);
    pub const Y: Keys = Keys(1 << 11);
    pub const NONE: Keys = Keys(0);

    /// BizHawk's button names (its `joypad` table), for the probes.
    pub fn from_name(name: &str) -> Option<Keys> {
        Some(match name {
            "A" => Self::A,
            "B" => Self::B,
            "Select" => Self::SELECT,
            "Start" => Self::START,
            "Right" => Self::RIGHT,
            "Left" => Self::LEFT,
            "Up" => Self::UP,
            "Down" => Self::DOWN,
            "R" => Self::R,
            "L" => Self::L,
            "X" => Self::X,
            "Y" => Self::Y,
            _ => return None,
        })
    }

    pub fn contains(self, other: Keys) -> bool {
        self.0 & other.0 == other.0
    }
}

impl std::ops::BitOr for Keys {
    type Output = Keys;
    fn bitor(self, rhs: Keys) -> Keys {
        Keys(self.0 | rhs.0)
    }
}

impl std::ops::BitOrAssign for Keys {
    fn bitor_assign(&mut self, rhs: Keys) {
        self.0 |= rhs.0;
    }
}

/// The generated firmware's owner settings. The default is what BizHawk's NDS settings give the
/// game today (`BizHawk/config.ini`), so saves and probes behave the same on both.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FirmwareUser {
    pub nickname: String,
    pub message: String,
    pub birthday_month: u8,
    pub birthday_day: u8,
    pub favorite_color: u8,
    /// 0 Japanese, 1 English, 2 French, 3 German, 4 Italian, 5 Spanish.
    pub language: u8,
}

impl Default for FirmwareUser {
    fn default() -> Self {
        Self {
            nickname: "melonDS".into(),
            message: "Melons Taste Great!".into(),
            birthday_month: 11,
            birthday_day: 3,
            favorite_color: 2,
            language: 1,
        }
    }
}

impl FirmwareUser {
    fn to_sys(&self) -> sys::GlFirmwareUser {
        fn utf16<const N: usize>(s: &str) -> ([u16; N], u32) {
            let mut out = [0u16; N];
            let mut n = 0;
            for (slot, unit) in out.iter_mut().zip(s.encode_utf16()) {
                *slot = unit;
                n += 1;
            }
            (out, n)
        }
        let (nickname, nickname_len) = utf16::<10>(&self.nickname);
        let (message, message_len) = utf16::<26>(&self.message);
        sys::GlFirmwareUser {
            nickname,
            nickname_len,
            message,
            message_len,
            birthday_month: self.birthday_month,
            birthday_day: self.birthday_day,
            favorite_color: self.favorite_color,
            language: self.language,
        }
    }
}

/// Called with the whole save memory, the changed range's offset and length, every time the game
/// writes its save (melonDS `Platform::WriteNDSSave`): exact save events.
pub type SaveHook = Box<dyn FnMut(&[u8], u32, u32) + Send>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CoreError {
    /// melonDS couldn't be created.
    Create,
    /// melonDS couldn't parse the ROM.
    BadRom,
    /// The ROM or save is larger than 4 GiB.
    TooLarge,
    /// melonDS rejected a savestate.
    BadState,
}

impl std::fmt::Display for CoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Create => write!(f, "the emulator couldn't start"),
            Self::BadRom => write!(f, "the emulator couldn't read that ROM"),
            Self::TooLarge => write!(f, "the ROM or save is too large"),
            Self::BadState => write!(f, "the emulator rejected that savestate"),
        }
    }
}

impl std::error::Error for CoreError {}

/// Receives melonDS's log lines: `(level, message)`.
pub type LogSink = fn(u8, &str);

/// melonDS's log, process-wide: `(level, message)` with level 0 debug .. 3 error. Without a sink
/// the log is dropped.
pub fn set_log_sink(sink: Option<LogSink>) {
    static SINK: std::sync::Mutex<Option<LogSink>> = std::sync::Mutex::new(None);
    unsafe extern "C" fn forward(level: c_int, message: *const c_char) {
        // SAFETY: platform.cpp passes a NUL-terminated buffer that lives for this call.
        let text = unsafe { CStr::from_ptr(message) }.to_string_lossy();
        if let Some(sink) = *SINK.lock().unwrap_or_else(|e| e.into_inner()) {
            sink(level.clamp(0, 3) as u8, text.trim_end());
        }
    }
    *SINK.lock().unwrap_or_else(|e| e.into_inner()) = sink;
    // SAFETY: `forward` matches GlLogCallback and is valid for the whole process.
    unsafe { sys::gl_set_log_callback(if sink.is_some() { Some(forward) } else { None }) };
}

/// One emulated Nintendo DS.
pub struct Nds {
    raw: NonNull<sys::GlCore>,
    /// Boxed twice so its address, given to C as the save callback's context, never moves.
    /// Only C reads it (through that pointer); it's here to live as long as the core.
    #[allow(dead_code)]
    save_hook: Box<Option<SaveHook>>,
    frames: u32,
}

// SAFETY: an Nds owns its melonDS instance outright (no shared state between instances; the
// software renderer runs on the calling thread). It may move between threads; `&mut self` on
// every mutating call keeps it single-threaded at any moment. Not Sync.
unsafe impl Send for Nds {}

unsafe extern "C" fn on_save(
    ctx: *mut c_void,
    save: *const u8,
    save_len: u32,
    offset: u32,
    len: u32,
) {
    // SAFETY: ctx is the Box<Option<SaveHook>> the Nds holds for the core's whole life; melonDS
    // calls this from run_frame, i.e. while the Nds is borrowed mutably, so nothing else touches
    // the hook. `save` points to save_len bytes of the cart's save memory for this call.
    let hook = unsafe { &mut *(ctx as *mut Option<SaveHook>) };
    if let Some(hook) = hook.as_mut() {
        let bytes = unsafe { std::slice::from_raw_parts(save, save_len as usize) };
        hook(bytes, offset, len);
    }
}

impl Nds {
    /// A DS with FreeBIOS and generated firmware, no cart yet.
    pub fn new(user: &FirmwareUser, save_hook: Option<SaveHook>) -> Result<Self, CoreError> {
        let mut save_hook = Box::new(save_hook);
        let cfg = sys::GlCoreConfig {
            user: user.to_sys(),
            on_save: Some(on_save),
            save_ctx: (&mut *save_hook) as *mut Option<SaveHook> as *mut c_void,
        };
        // SAFETY: cfg is valid for the call; the core copies what it keeps.
        let raw = unsafe { sys::gl_core_new(&cfg) };
        Ok(Self {
            raw: NonNull::new(raw).ok_or(CoreError::Create)?,
            save_hook,
            frames: 0,
        })
    }

    fn ptr(&self) -> *mut sys::GlCore {
        self.raw.as_ptr()
    }

    /// Inserts a ROM with its save (None: a blank save) and powers on. Also a power cycle when
    /// called again (BizHawk's `reboot_core`).
    pub fn load_rom(&mut self, rom: &[u8], save: Option<&[u8]>) -> Result<(), CoreError> {
        let rom_len = u32::try_from(rom.len()).map_err(|_| CoreError::TooLarge)?;
        let (save_ptr, save_len) = match save {
            Some(s) => (
                s.as_ptr(),
                u32::try_from(s.len()).map_err(|_| CoreError::TooLarge)?,
            ),
            None => (std::ptr::null(), 0),
        };
        // SAFETY: both buffers are valid for their lengths during the call; the core copies them.
        let status =
            unsafe { sys::gl_core_load_rom(self.ptr(), rom.as_ptr(), rom_len, save_ptr, save_len) };
        self.frames = 0;
        if status == 0 {
            Ok(())
        } else {
            Err(CoreError::BadRom)
        }
    }

    /// Sets the DS's clock.
    pub fn set_rtc(&mut self, at: NaiveDateTime) {
        // SAFETY: plain values.
        unsafe {
            sys::gl_core_set_rtc(
                self.ptr(),
                at.year(),
                at.month() as c_int,
                at.day() as c_int,
                at.hour() as c_int,
                at.minute() as c_int,
                at.second() as c_int,
            )
        };
    }

    /// The DS's clock = the computer's local time (BizHawk's `UseRealTime`).
    pub fn set_rtc_now(&mut self) {
        self.set_rtc(Local::now().naive_local());
    }

    /// Runs one frame (about 1/60 s of DS time). Returns the frames run since power-on.
    pub fn run_frame(&mut self) -> u32 {
        // SAFETY: the core is valid; the save hook may run inside (see on_save).
        self.frames = unsafe { sys::gl_core_run_frame(self.ptr()) };
        self.frames
    }

    pub fn frame_count(&self) -> u32 {
        self.frames
    }

    /// The buttons held and the stylus (`Some((x, y))` on the bottom screen) for the next frames.
    pub fn set_input(&mut self, keys: Keys, touch: Option<(u16, u16)>) {
        // SAFETY: plain values.
        unsafe {
            sys::gl_core_set_keys(self.ptr(), keys.0 & 0xFFF);
            match touch {
                Some((x, y)) => sys::gl_core_set_touch(self.ptr(), 1, x, y),
                None => sys::gl_core_set_touch(self.ptr(), 0, 0, 0),
            }
        }
    }

    /// The 4 MiB main RAM: offset = ARM9 address - 0x02000000.
    pub fn main_ram(&self) -> &[u8] {
        let mut len = 0;
        // SAFETY: the buffer lives as long as the core and is only written during run_frame /
        // load_rom, which take &mut self, so it can't change under this shared borrow.
        unsafe {
            let p = sys::gl_core_main_ram(self.ptr(), &mut len);
            std::slice::from_raw_parts(p, len as usize)
        }
    }

    pub fn main_ram_mut(&mut self) -> &mut [u8] {
        let mut len = 0;
        // SAFETY: as main_ram, and &mut self makes this the only borrow.
        unsafe {
            let p = sys::gl_core_main_ram(self.ptr(), &mut len);
            std::slice::from_raw_parts_mut(p, len as usize)
        }
    }

    /// A read through the ARM9's memory map, without side effects (I/O reads as 0).
    pub fn arm9_peek8(&mut self, addr: u32) -> u8 {
        // SAFETY: the shim bounds every access by melonDS's own memory map.
        unsafe { sys::gl_core_arm9_peek8(self.ptr(), addr) }
    }

    /// The cart's save memory (empty without a cart).
    pub fn save(&self) -> &[u8] {
        let mut len = 0;
        // SAFETY: as main_ram (the save buffer belongs to the cart, replaced only by load_rom).
        unsafe {
            let p = sys::gl_core_save(self.ptr(), &mut len);
            if p.is_null() {
                &[]
            } else {
                std::slice::from_raw_parts(p, len as usize)
            }
        }
    }

    pub fn save_mut(&mut self) -> &mut [u8] {
        let mut len = 0;
        // SAFETY: as save, with the only borrow.
        unsafe {
            let p = sys::gl_core_save(self.ptr(), &mut len);
            if p.is_null() {
                &mut []
            } else {
                std::slice::from_raw_parts_mut(p, len as usize)
            }
        }
    }

    /// A screen's last finished frame (0 top, 1 bottom): 256 x 192 pixels, 0x00RRGGBB.
    pub fn framebuffer(&self, screen: usize) -> &[u32] {
        // SAFETY: melonDS's front buffer holds 256 * 192 u32 and is swapped only in run_frame.
        unsafe {
            let p = sys::gl_core_framebuffer(self.ptr(), screen.min(1) as c_int);
            std::slice::from_raw_parts(p, SCREEN_WIDTH * SCREEN_HEIGHT)
        }
    }

    /// The most the picture can be widened on each side (UC9 §4).
    pub const MAX_WIDE_SIDE: usize = 64;

    /// Widens the 3D view by `side` pixels on each side (0: the DS's own view).
    pub fn set_wide(&mut self, side: usize) {
        // SAFETY: a plain setter on the live core.
        unsafe { sys::gl_core_set_wide(self.ptr(), side.min(Self::MAX_WIDE_SIDE) as c_int) }
    }

    /// A screen `256 + 2 * side` pixels wide (0x00RRGGBB, 192 rows): the screen as shown in the
    /// middle, the 3D layer on the sides.
    pub fn wide_frame(&self, screen: usize, side: usize) -> Vec<u32> {
        let side = side.min(Self::MAX_WIDE_SIDE);
        let mut out = vec![0u32; (SCREEN_WIDTH + 2 * side) * SCREEN_HEIGHT];
        // SAFETY: out holds (256 + 2 * side) * 192 pixels, what the shim writes.
        unsafe {
            sys::gl_core_wide_frame(
                self.ptr(),
                screen.min(1) as c_int,
                side as c_int,
                out.as_mut_ptr(),
            );
        }
        out
    }

    /// Takes up to `out.len() / 2` stereo frames of audio. Returns how many it wrote.
    pub fn audio(&mut self, out: &mut [i16]) -> usize {
        // SAFETY: out holds out.len() / 2 interleaved stereo frames.
        unsafe { sys::gl_core_audio(self.ptr(), out.as_mut_ptr(), (out.len() / 2) as u32) as usize }
    }

    /// The audio output's sample rate in Hz (48,000 until set; 8,000..192,000): the core
    /// resamples to it, so `audio` yields this many stereo frames per emulated second.
    pub fn set_audio_rate(&mut self, rate: f64) {
        // SAFETY: plain call on a live core.
        unsafe { sys::gl_core_set_audio_rate(self.ptr(), rate) }
    }

    /// `cpu` 9 or 7: r0..r15, then CPSR.
    pub fn registers(&self, cpu: u8) -> [u32; 17] {
        let mut out = [0u32; 17];
        // SAFETY: out has the 17 slots the shim fills.
        unsafe {
            sys::gl_core_registers(self.ptr(), if cpu == 7 { 7 } else { 9 }, out.as_mut_ptr())
        };
        out
    }

    /// A savestate of the whole machine (BizHawk's memorysavestate; the bridge's backups).
    pub fn savestate(&mut self) -> Option<Vec<u8>> {
        // SAFETY: the shim keeps the state in the core until the next call; we copy it at once.
        unsafe {
            let len = sys::gl_core_savestate(self.ptr());
            if len == 0 {
                return None;
            }
            Some(
                std::slice::from_raw_parts(sys::gl_core_savestate_data(self.ptr()), len as usize)
                    .to_vec(),
            )
        }
    }

    /// Restores a savestate from [`Nds::savestate`] (same ROM).
    pub fn load_savestate(&mut self, state: &[u8]) -> Result<(), CoreError> {
        let len = u32::try_from(state.len()).map_err(|_| CoreError::TooLarge)?;
        // SAFETY: the buffer is valid for len bytes; the shim copies it.
        let status = unsafe { sys::gl_core_loadstate(self.ptr(), state.as_ptr(), len) };
        if status == 0 {
            Ok(())
        } else {
            Err(CoreError::BadState)
        }
    }

    /// The co-op mailbox's header at main-RAM `offset` (coop_offsets.json `main_ram_offset`),
    /// if the ROM has published it.
    pub fn mailbox(&self, offset: usize) -> Option<MailboxHeader> {
        use goldlink_abi::CoopCommBlock as B;
        use std::mem::offset_of;
        let ram = self.main_ram();
        let at = |field: usize, n: usize| ram.get(offset + field..offset + field + n);
        let u32_at = |field| at(field, 4).map(|b| u32::from_le_bytes(b.try_into().unwrap()));
        let magic = u32_at(offset_of!(B, magic))?;
        if magic != goldlink_abi::COOP_MAGIC {
            return None;
        }
        Some(MailboxHeader {
            version: at(offset_of!(B, version), 2).map(|b| u16::from_le_bytes([b[0], b[1]]))?,
            heartbeat_rom: u32_at(offset_of!(B, heartbeat_rom))?,
            heartbeat_bridge: u32_at(offset_of!(B, heartbeat_bridge))?,
        })
    }
}

impl Drop for Nds {
    fn drop(&mut self) {
        // SAFETY: the pointer came from gl_core_new and is freed once.
        unsafe { sys::gl_core_free(self.ptr()) };
    }
}

/// What the ROM publishes first in the mailbox.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MailboxHeader {
    pub version: u16,
    pub heartbeat_rom: u32,
    pub heartbeat_bridge: u32,
}
