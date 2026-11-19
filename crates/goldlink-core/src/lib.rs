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
pub use sys::{GlShadowPoly as ShadowPoly, GlShadowTex as ShadowTex, GlShadowVert as ShadowVert};

/// The polygons of the 3D picture on screen (UC13), for the cast shadows: each vertex's clip
/// coordinates (20.12: the projection places them in 3D), texture coordinates and screen
/// position; each polygon's attributes and kind ([`ShadowPoly::flags`]: 1 attached, 2
/// translucent, 4 and 8 the DS's shadow volumes, 16 facing, 32 a line); the transparency of
/// textures that can have transparent texels (0..31 a texel, `ShadowTex` places each in `alpha`).
#[derive(Debug, Clone, Default)]
pub struct ShadowGeometry {
    pub polys: Vec<ShadowPoly>,
    pub verts: Vec<ShadowVert>,
    pub texs: Vec<ShadowTex>,
    pub alpha: Vec<u8>,
}

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

/// The steady overworld's attached picture for a frame (`Nds::attached`, UC12): what the camera
/// follows (the player, their shadow, a mount, a follower), drawn apart from the world, and the
/// two pictures for the window to put together at the screen's resolution with the attached
/// one where it truly is (`Pictures::composite`). `att` and `plain` are `256 + 2 * side` x 192.
#[derive(Debug, Clone, Default)]
pub struct Attached {
    /// The shift (DS pixels, x then y) the attached picture was drawn with (on the world's
    /// pixels); the world's is the frame's steady shift.
    pub shift: [f32; 2],
    /// Where its reference truly is (the DS screen's columns and rows).
    pub origin: [f64; 2],
    /// The attached picture: 0xAARRGGBB, alpha 0 where it has nothing.
    pub att: Vec<u32>,
    /// 1 where the top screen shows the 3D as drawn (on top, no 2D, no screen-wide fade).
    pub plain: Vec<u8>,
    /// The two pictures as the renderer keeps them.
    pub pictures: Pictures,
    pub side: usize,
}

/// The two pictures of the steady overworld as the renderer keeps them (`goldlink_core.h`'s
/// `GlPictures`), for `Pictures::composite`.
#[derive(Debug, Clone, Default)]
pub struct Pictures {
    u32s: [Vec<u32>; 13],
    f32s: [Vec<f32>; 4],
    stride: i32,
    disp_cnt: u32,
    edge_table: [u16; 8],
    left: i32,
    right: i32,
}

impl Pictures {
    /// One of the u32 arrays, in `GlPictures`' order (world top, its depth, its attributes, world
    /// bottom, its depth, world opaque, attached top, its depth, its translucent depth, its
    /// attributes, attached bottom, late early, late depth), `stride` x 192.
    pub fn u32_plane(&self, i: usize) -> &[u32] {
        &self.u32s[i]
    }

    /// One of the f32 arrays: attached k, attached b (three a pixel), late k, late b (three).
    pub fn f32_plane(&self, i: usize) -> &[f32] {
        &self.f32s[i]
    }

    /// The arrays' row length (the renderer's scanline).
    pub fn stride(&self) -> i32 {
        self.stride
    }

    /// DISP3DCNT, the edge colours, the picture's left and right columns.
    pub fn display(&self) -> (u32, [u16; 8], i32, i32) {
        (self.disp_cnt, self.edge_table, self.left, self.right)
    }

    /// The arrays sized for the renderer's (`gl_pictures_size`).
    fn sized(&mut self) {
        // SAFETY: a size query.
        let n = unsafe { sys::gl_pictures_size() } as usize;
        for v in &mut self.u32s {
            v.resize(n, 0);
        }
        for (i, v) in self.f32s.iter_mut().enumerate() {
            v.resize(if i % 2 == 1 { n * 3 } else { n }, 0.0);
        }
    }

    /// The C view of the arrays (written through only by `gl_core_copy_pictures`, with `&mut
    /// self`).
    fn view(&self) -> sys::GlPictures {
        let u = |i: usize| self.u32s[i].as_ptr().cast_mut();
        let f = |i: usize| self.f32s[i].as_ptr().cast_mut();
        sys::GlPictures {
            world_top: u(0),
            world_top_depth: u(1),
            world_top_attr: u(2),
            world_bottom: u(3),
            world_bottom_depth: u(4),
            world_opaque: u(5),
            att_top: u(6),
            att_top_depth: u(7),
            att_trans_depth: u(8),
            att_attr: u(9),
            att_bottom: u(10),
            att_k: f(0),
            att_b: f(1),
            late_early: u(11),
            late_depth: u(12),
            late_k: f(2),
            late_b: f(3),
            stride: self.stride,
            disp_cnt: self.disp_cnt,
            edge_table: self.edge_table,
            left: self.left,
            right: self.right,
        }
    }

    /// The two put together (the renderer's `Composite`) at each place: the world's pixel
    /// (`world`, x then y, the picture's columns for `side`), the attached picture's (`att`; out
    /// of the picture: none), `d` the world's point less the attached picture's at the same place
    /// on screen; 0xRRGGBB into `out`, before the lighting.
    pub fn composite(
        &self,
        side: usize,
        world: &[i32],
        att: &[i32],
        d: [f32; 2],
        out: &mut Vec<u32>,
    ) {
        let n = world.len().min(att.len()) / 2;
        out.resize(n, 0);
        if self.u32s[0].is_empty() || n == 0 {
            return;
        }
        let view = self.view();
        // SAFETY: every array holds gl_pictures_size() elements (att_b, late_b three times
        // that), only read; world and att hold 2n, out n.
        unsafe {
            sys::gl_composite(
                &view,
                side as c_int,
                n as c_int,
                world.as_ptr(),
                att.as_ptr(),
                d[0],
                d[1],
                out.as_mut_ptr(),
            )
        };
    }
}

/// The top screen's 3D depth and where the 3D layer shows, for the lighting (UC11). Each is
/// [`LightBuffers::WIDTH`] x 192, the screen's x = 0 at column [`Nds::MAX_WIDE_SIDE`].
#[derive(Debug, Clone, Default)]
pub struct LightBuffers {
    /// 24-bit depth: Z (0..0xFFFFFF over the projection's near..far) or W (`w_depth`).
    pub depth: Vec<u32>,
    /// 1: the 3D scene on top; 2: under a 2D layer (text box, a banner, menus); 0: 2D only.
    pub layer: Vec<u8>,
    /// Each pixel's opaque polygon ID (0..63): the ROM draws a neighbouring area's map with
    /// [`NEIGHBOUR_POLYGON_ID`].
    pub ids: Vec<u8>,
    /// The projection matrix, 20.12, as the DS keeps it (row by row, vectors on the left).
    pub proj: [i32; 16],
    pub w_depth: bool,
    /// Each line's fades (192): see [`Fade`].
    pub fade: Vec<Fade>,
    /// The picture's polygons (UC13).
    pub geometry: ShadowGeometry,
}

/// The polygon ID the ROM draws a neighbouring area's map blocks with (their graphics aren't
/// loaded: GoldLink hazes them in the wider picture). HeartGold's own maps use 0.
pub const NEIGHBOUR_POLYGON_ID: u8 = 62;

/// A line's screen-wide fades as the DS applies them after composing (UC11): the master
/// brightness (`[mode, factor]`: 0 none, 1 toward white, 2 toward black; factor 0..16), then the
/// colour effect's brightness on the 3D layer (where the 3D scene shows on top).
pub type Fade = [u8; 4];

impl LightBuffers {
    pub const WIDTH: usize = SCREEN_WIDTH + 2 * Nds::MAX_WIDE_SIDE;
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

    /// The renderer's picture width (its arrays' columns, `RenderWidth`).
    pub const RENDER_WIDTH: usize = SCREEN_WIDTH + 2 * Self::MAX_WIDE_SIDE;
    /// The most the picture can be widened on each side (UC9 §4).
    pub const MAX_WIDE_SIDE: usize = 64;

    /// Widens the 3D view by `side` pixels on each side (0: the DS's own view).
    pub fn set_wide(&mut self, side: usize) {
        // SAFETY: a plain setter on the live core.
        unsafe { sys::gl_core_set_wide(self.ptr(), side.min(Self::MAX_WIDE_SIDE) as c_int) }
    }

    /// The steady overworld (UC11): field scenes drawn with an orthographic camera locked to the
    /// pixels. GoldLink turns it on only while the game is in the overworld.
    pub fn set_steady(&mut self, on: bool) {
        // SAFETY: a plain setter on the live core.
        unsafe { sys::gl_core_set_steady(self.ptr(), c_int::from(on)) }
    }

    /// Where the steady world's origin truly is in the picture on screen (the DS screen's columns
    /// and rows; patch 0023): the window lands it on a whole pixel of its own.
    pub fn steady_origin(&self) -> [f64; 2] {
        let mut xy = [0f64; 2];
        // SAFETY: the shim writes two doubles.
        unsafe { sys::gl_core_steady_origin(self.ptr(), xy.as_mut_ptr()) };
        xy
    }

    /// When the picture on screen was drawn steady: the shift (pixels, x then y) it was drawn
    /// with, to be taken back at the screen's resolution.
    pub fn steady_shift(&self) -> Option<[f32; 2]> {
        let mut xy = [0f32; 2];
        // SAFETY: the shim writes two floats.
        let on = unsafe { sys::gl_core_steady_shift(self.ptr(), xy.as_mut_ptr()) };
        (on == 1).then_some(xy)
    }

    /// The steady overworld's attached layer (patch 0022): what the camera follows (the player,
    /// their shadow, a mount) on the top screen (as wide as [`Nds::fixed_2d`]'s), slid back by
    /// its own shift alone. None when there's none this frame.
    pub fn attached(&self, side: usize) -> Option<Attached> {
        let side = side.min(Self::MAX_WIDE_SIDE);
        let n = (SCREEN_WIDTH + 2 * side) * SCREEN_HEIGHT;
        let mut a = Attached {
            att: vec![0; n],
            plain: vec![0; n],
            side,
            ..Default::default()
        };
        // SAFETY: att and plain hold (256 + 2 * side) * 192 elements, what the shim writes; shift
        // and origin two each.
        let found = unsafe {
            sys::gl_core_attached(
                self.ptr(),
                side as c_int,
                a.att.as_mut_ptr(),
                a.plain.as_mut_ptr(),
                a.shift.as_mut_ptr(),
                a.origin.as_mut_ptr(),
            )
        };
        if found == 0 {
            return None;
        }
        a.pictures.sized();
        let mut view = a.pictures.view();
        // SAFETY: the arrays were just sized to gl_pictures_size() (att_b, late_b three times
        // that); a.pictures is borrowed by nothing else.
        let copied = unsafe { sys::gl_core_copy_pictures(self.ptr(), &mut view) };
        a.pictures.stride = view.stride;
        a.pictures.disp_cnt = view.disp_cnt;
        a.pictures.edge_table = view.edge_table;
        a.pictures.left = view.left;
        a.pictures.right = view.right;
        (copied != 0).then_some(a)
    }

    /// The top screen's pixels (`256 + 2 * side` across, 192 rows; `side` 0 for the DS's screen)
    /// where 2D shows over the 3D scene unblended (a text box, a banner): 1 there, else 0. Empty
    /// when there's none. A steady picture's slide leaves them where they are.
    pub fn fixed_2d(&self, side: usize) -> Vec<u8> {
        let side = side.min(Self::MAX_WIDE_SIDE);
        let mut out = vec![0u8; (SCREEN_WIDTH + 2 * side) * SCREEN_HEIGHT];
        // SAFETY: out holds (256 + 2 * side) * 192 bytes, what the shim writes.
        let n = unsafe { sys::gl_core_fixed_2d(self.ptr(), side as c_int, out.as_mut_ptr()) };
        if n == 0 {
            out.clear();
        }
        out
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

    /// Leaves the game's painted shadows (a building's, a sign's) out of the 3D picture from the
    /// next render, while GoldLink casts its own (UC13); the blobs under people stay (patch
    /// 0031).
    pub fn set_hide_game_shadows(&mut self, on: bool) {
        // SAFETY: the core is live; the shim only sets a flag.
        unsafe { sys::gl_core_set_hide_game_shadows(self.ptr(), c_int::from(on)) }
    }

    /// With the game's shadows hidden, the blobs under people left out too (patch 0032): while
    /// GoldLink's sun casts (the user: the game's blobs by night only, as moonlight).
    pub fn set_hide_blob_shadows(&mut self, on: bool) {
        // SAFETY: the core is live; the shim only sets a flag.
        unsafe { sys::gl_core_set_hide_blob_shadows(self.ptr(), c_int::from(on)) }
    }

    /// The blobs under people darken at most `dark` (patch 0033; 0: as the game draws them): by
    /// night, faint as the moon's light.
    pub fn set_blob_shadow_dark(&mut self, dark: f32) {
        // SAFETY: the core is live; the shim only sets a value.
        unsafe { sys::gl_core_set_blob_shadow_dark(self.ptr(), dark) }
    }

    /// What the lighting needs from the top screen (UC11) into `out`; false when the top screen
    /// can't show the 3D scene (then `out` is left as it was).
    pub fn light_buffers(&self, out: &mut LightBuffers) -> bool {
        let n = LightBuffers::WIDTH * SCREEN_HEIGHT;
        out.depth.resize(n, 0);
        out.layer.resize(n, 0);
        out.ids.resize(n, 0);
        // SAFETY: depth, layer and ids hold 384 * 192 each, what the shim writes; proj holds 16.
        let kind = unsafe {
            sys::gl_core_light_buffers(
                self.ptr(),
                out.depth.as_mut_ptr(),
                out.layer.as_mut_ptr(),
                out.ids.as_mut_ptr(),
                out.proj.as_mut_ptr(),
            )
        };
        out.w_depth = kind == 2;
        out.fade.resize(SCREEN_HEIGHT, [0; 4]);
        // SAFETY: fade holds 192 four-byte lines, what the shim writes.
        unsafe { sys::gl_core_light_fades(self.ptr(), out.fade.as_mut_ptr().cast()) };
        let mut counts = [0u32; 4];
        // SAFETY: counts holds 4, what the shim writes.
        unsafe { sys::gl_core_shadow_counts(self.ptr(), counts.as_mut_ptr()) };
        let g = &mut out.geometry;
        g.polys.resize(counts[0] as usize, ShadowPoly::default());
        g.verts.resize(counts[1] as usize, ShadowVert::default());
        g.texs.resize(counts[2] as usize, ShadowTex::default());
        g.alpha.resize(counts[3] as usize, 0);
        // SAFETY: each array is as long as the counts just given for the same frame.
        unsafe {
            sys::gl_core_shadow_geometry(
                self.ptr(),
                g.polys.as_mut_ptr(),
                g.verts.as_mut_ptr(),
                g.texs.as_mut_ptr(),
                g.alpha.as_mut_ptr(),
            )
        };
        kind != 0
    }

    /// Rectangles `[x0, y0, x1, y1, dx]` (screen pixels, end exclusive) whose main-engine sprite
    /// pieces (by their top-left corner) are drawn moved by `dx`, from the next frame: the
    /// one-screen layout moves the battle's HP boxes toward the wider picture's edges. At most 8
    /// are used; empty clears them.
    pub fn set_sprite_move(&mut self, rects: &[[i32; 5]]) {
        let flat: Vec<i32> = rects.iter().take(8).flatten().copied().collect();
        // SAFETY: flat holds count * 5 ints, what the shim reads.
        unsafe {
            sys::gl_core_set_sprite_move(self.ptr(), flat.as_ptr(), (flat.len() / 5) as c_int);
        }
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
