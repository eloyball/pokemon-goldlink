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

/// `goldlink_core.h`'s `GlShadowPoly` (UC13): a polygon of the picture on screen.
#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct GlShadowPoly {
    pub first_vert: u32,
    pub num_verts: u32,
    pub attr: u32,
    pub tex_param: u32,
    pub tex_palette: u32,
    pub tex: i32,
    pub flags: u32,
    pub object: i32,
}

/// `goldlink_core.h`'s `GlShadowVert`.
#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct GlShadowVert {
    pub clip: [i32; 4],
    pub st: [i16; 2],
    pub screen: [i32; 2],
}

/// `goldlink_core.h`'s `GlShadowTex`.
#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct GlShadowTex {
    pub width: u32,
    pub height: u32,
    pub offset: u32,
}

/// `goldlink_core.h`'s `GlPictures`: where `gl_core_copy_pictures` copies the two pictures.
#[repr(C)]
pub struct GlPictures {
    pub world_top: *mut u32,
    pub world_top_depth: *mut u32,
    pub world_top_attr: *mut u32,
    pub world_bottom: *mut u32,
    pub world_bottom_depth: *mut u32,
    pub world_opaque: *mut u32,
    pub att_top: *mut u32,
    pub att_top_depth: *mut u32,
    pub att_trans_depth: *mut u32,
    pub att_attr: *mut u32,
    pub att_bottom: *mut u32,
    pub att_k: *mut f32,
    pub att_b: *mut f32,
    pub late_early: *mut u32,
    pub late_depth: *mut u32,
    pub late_k: *mut f32,
    pub late_b: *mut f32,
    pub stride: c_int,
    pub disp_cnt: u32,
    pub edge_table: [u16; 8],
    pub left: c_int,
    pub right: c_int,
}

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
    pub fn gl_core_set_steady(core: *mut GlCore, on: c_int);
    pub fn gl_core_steady_shift(core: *mut GlCore, xy: *mut f32) -> c_int;
    pub fn gl_core_fixed_2d(core: *mut GlCore, side: c_int, out: *mut u8) -> c_int;
    pub fn gl_core_attached(
        core: *mut GlCore,
        side: c_int,
        att: *mut u32,
        plain: *mut u8,
        xy: *mut f32,
        at: *mut f64,
    ) -> c_int;
    pub fn gl_pictures_size() -> c_int;
    pub fn gl_core_copy_pictures(core: *mut GlCore, o: *mut GlPictures) -> c_int;
    pub fn gl_composite(
        o: *const GlPictures,
        side: c_int,
        n: c_int,
        world: *const i32,
        att: *const i32,
        dx: f32,
        dy: f32,
        out: *mut u32,
    );
    pub fn gl_core_steady_origin(core: *mut GlCore, xy: *mut f64);
    pub fn gl_core_set_sprite_move(core: *mut GlCore, rects: *const i32, count: c_int);
    pub fn gl_core_wide_frame(
        core: *mut GlCore,
        screen: c_int,
        side: c_int,
        out: *mut u32,
    ) -> c_int;
    pub fn gl_core_light_fades(core: *mut GlCore, out: *mut u8);
    pub fn gl_core_shadow_counts(core: *mut GlCore, counts: *mut u32);
    pub fn gl_core_set_hide_game_shadows(core: *mut GlCore, on: c_int);
    pub fn gl_core_set_hide_blob_shadows(core: *mut GlCore, on: c_int);
    pub fn gl_core_set_blob_shadow_dark(core: *mut GlCore, dark: f32);
    pub fn gl_core_set_soften_game_shadows(core: *mut GlCore, on: c_int);
    pub fn gl_core_shadow_geometry(
        core: *mut GlCore,
        polys: *mut GlShadowPoly,
        verts: *mut GlShadowVert,
        texs: *mut GlShadowTex,
        alpha: *mut u8,
    );
    pub fn gl_core_light_buffers(
        core: *mut GlCore,
        depth: *mut u32,
        layer: *mut u8,
        ids: *mut u8,
        proj: *mut i32,
    ) -> c_int;
    pub fn gl_core_audio(core: *mut GlCore, out: *mut i16, max_frames: u32) -> u32;
    pub fn gl_core_set_audio_rate(core: *mut GlCore, rate: f64);
    pub fn gl_core_registers(core: *mut GlCore, cpu: c_int, out: *mut u32);
    pub fn gl_core_savestate(core: *mut GlCore) -> u32;
    pub fn gl_core_savestate_data(core: *mut GlCore) -> *const u8;
    pub fn gl_core_loadstate(core: *mut GlCore, state: *const u8, len: u32) -> c_int;
}
