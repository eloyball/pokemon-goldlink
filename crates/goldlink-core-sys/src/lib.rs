//! Raw bindings to GoldLink's C ABI over melonDS (`shim/goldlink_core.h`). Use `goldlink-core`.

#![allow(non_camel_case_types)]

use std::ffi::{c_char, c_int, c_void};

#[repr(C)]
pub struct GlCore {
    _private: [u8; 0],
}

pub type GlSaveCallback = Option<
    unsafe extern "C" fn(ctx: *mut c_void, save: *const u8, save_len: u32, offset: u32, len: u32),
>;
pub type GlLogCallback = Option<unsafe extern "C" fn(level: c_int, message: *const c_char)>;

#[repr(C)]
#[derive(Clone, Copy)]
pub struct GlFirmwareUser {
    pub nickname: [u16; 10],
    pub nickname_len: u32,
    pub message: [u16; 26],
    pub message_len: u32,
    pub birthday_month: u8,
    pub birthday_day: u8,
    pub favorite_color: u8,
    pub language: u8,
}

#[repr(C)]
pub struct GlCoreConfig {
    pub user: GlFirmwareUser,
    pub on_save: GlSaveCallback,
    pub save_ctx: *mut c_void,
}

pub const GL_SCREEN_WIDTH: usize = 256;
pub const GL_SCREEN_HEIGHT: usize = 192;

unsafe extern "C" {
    pub fn gl_set_log_callback(cb: GlLogCallback);
    pub fn gl_core_new(cfg: *const GlCoreConfig) -> *mut GlCore;
    pub fn gl_core_free(core: *mut GlCore);
    pub fn gl_core_load_rom(
        core: *mut GlCore,
        rom: *const u8,
        rom_len: u32,
        save: *const u8,
        save_len: u32,
    ) -> c_int;
    pub fn gl_core_set_rtc(
        core: *mut GlCore,
        year: c_int,
        month: c_int,
        day: c_int,
        hour: c_int,
        minute: c_int,
        second: c_int,
    );
    pub fn gl_core_run_frame(core: *mut GlCore) -> u32;
    pub fn gl_core_set_keys(core: *mut GlCore, held: u32);
    pub fn gl_core_set_touch(core: *mut GlCore, touching: c_int, x: u16, y: u16);
    pub fn gl_core_main_ram(core: *mut GlCore, len: *mut u32) -> *mut u8;
    pub fn gl_core_arm9_peek8(core: *mut GlCore, addr: u32) -> u8;
    pub fn gl_core_save(core: *mut GlCore, len: *mut u32) -> *mut u8;
    pub fn gl_core_framebuffer(core: *mut GlCore, screen: c_int) -> *const u32;
    pub fn gl_core_set_wide(core: *mut GlCore, side: c_int);
    pub fn gl_core_set_sprite_move(core: *mut GlCore, rects: *const i32, count: c_int);
    pub fn gl_core_wide_frame(
        core: *mut GlCore,
        screen: c_int,
        side: c_int,
        out: *mut u32,
    ) -> c_int;
    pub fn gl_core_audio(core: *mut GlCore, out: *mut i16, max_frames: u32) -> u32;
    pub fn gl_core_set_audio_rate(core: *mut GlCore, rate: f64);
    pub fn gl_core_registers(core: *mut GlCore, cpu: c_int, out: *mut u32);
    pub fn gl_core_savestate(core: *mut GlCore) -> u32;
    pub fn gl_core_savestate_data(core: *mut GlCore) -> *const u8;
    pub fn gl_core_loadstate(core: *mut GlCore, state: *const u8, len: u32) -> c_int;
}
