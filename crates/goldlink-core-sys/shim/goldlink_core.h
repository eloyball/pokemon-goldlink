// SPDX-License-Identifier: GPL-3.0-or-later
//
// GoldLink's C ABI over the melonDS core (unified-client RFC §2.2, UC1_PROPOSAL §3.3). Upstream
// melonDS changes stay inside goldlink_core.cpp and platform.cpp; Rust only sees these functions.
//
// One GlCore is one emulated DS. Several can live in one process (UC4's in-process tests).
// A GlCore is not thread-safe: call it from one thread at a time.

#ifndef GOLDLINK_CORE_H
#define GOLDLINK_CORE_H

#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

typedef struct GlCore GlCore;

// The game wrote its save (melonDS Platform::WriteNDSSave): the whole save memory, and the
// range that changed.
typedef void (*GlSaveCallback)(void *ctx, const uint8_t *save, uint32_t save_len, uint32_t offset, uint32_t len);

// melonDS's log (Platform::Log has no instance, so this is process-wide). level: 0 debug,
// 1 info, 2 warn, 3 error. `message` ends with a newline most of the time.
typedef void (*GlLogCallback)(int level, const char *message);

// The generated firmware's user settings (what the game reads as the DS's owner).
typedef struct GlFirmwareUser {
    uint16_t nickname[10]; // UTF-16
    uint32_t nickname_len;
    uint16_t message[26];  // UTF-16
    uint32_t message_len;
    uint8_t birthday_month;
    uint8_t birthday_day;
    uint8_t favorite_color; // 0-15
    uint8_t language;       // 0 Japanese, 1 English, 2 French, 3 German, 4 Italian, 5 Spanish
} GlFirmwareUser;

typedef struct GlCoreConfig {
    GlFirmwareUser user;
    GlSaveCallback on_save; // may be NULL
    void *save_ctx;
} GlCoreConfig;

// The DS buttons for gl_core_set_keys (a bit set = held).
enum {
    GL_KEY_A = 1 << 0,
    GL_KEY_B = 1 << 1,
    GL_KEY_SELECT = 1 << 2,
    GL_KEY_START = 1 << 3,
    GL_KEY_RIGHT = 1 << 4,
    GL_KEY_LEFT = 1 << 5,
    GL_KEY_UP = 1 << 6,
    GL_KEY_DOWN = 1 << 7,
    GL_KEY_R = 1 << 8,
    GL_KEY_L = 1 << 9,
    GL_KEY_X = 1 << 10,
    GL_KEY_Y = 1 << 11,
};

#define GL_SCREEN_WIDTH 256
#define GL_SCREEN_HEIGHT 192

void gl_set_log_callback(GlLogCallback cb);

// A DS with FreeBIOS and generated firmware, no cart. NULL on failure.
GlCore *gl_core_new(const GlCoreConfig *cfg);
void gl_core_free(GlCore *core);

// Inserts a ROM (copied) with its save memory (copied; NULL / 0 = a blank save), then powers on:
// reset, direct boot (as BizHawk's SkipFirmware), start. Also a power cycle when called again.
// Returns 0, or 1 if melonDS can't parse the ROM.
int gl_core_load_rom(GlCore *core, const uint8_t *rom, uint32_t rom_len, const uint8_t *save, uint32_t save_len);

void gl_core_set_rtc(GlCore *core, int year, int month, int day, int hour, int minute, int second);

// Runs one frame. Returns the number of frames run since power-on.
uint32_t gl_core_run_frame(GlCore *core);

void gl_core_set_keys(GlCore *core, uint32_t held);
// The stylus: touching != 0 at (x, y) in the bottom screen's pixels; touching == 0 lifts it.
void gl_core_set_touch(GlCore *core, int touching, uint16_t x, uint16_t y);

// The 4 MiB main RAM (ARM9 0x02000000, mirrored). Valid until gl_core_free.
uint8_t *gl_core_main_ram(GlCore *core, uint32_t *len);
// A side-effect-free read through the ARM9's memory map ("ARM9 System Bus"): main RAM, VRAM,
// palettes, OAM and the rest the ARM9 maps; I/O registers read as 0.
uint8_t gl_core_arm9_peek8(GlCore *core, uint32_t addr);
// The cart's save memory (NULL / 0 without a cart). Writes go to the live save.
uint8_t *gl_core_save(GlCore *core, uint32_t *len);

// The front buffer of a screen (0 top, 1 bottom): 256 x 192 pixels, 0x00RRGGBB.
const uint32_t *gl_core_framebuffer(GlCore *core, int screen);

// Up to max_frames stereo frames (interleaved L, R) of audio produced so far. Returns how many.
uint32_t gl_core_audio(GlCore *core, int16_t *out, uint32_t max_frames);

// cpu 9 or 7: out[0..16] = r0-r15, out[16] = CPSR.
void gl_core_registers(GlCore *core, int cpu, uint32_t out[17]);

// A savestate of the whole machine, kept inside the core until the next call: returns its
// length (0 on failure); gl_core_savestate_data points to it.
uint32_t gl_core_savestate(GlCore *core);
const uint8_t *gl_core_savestate_data(GlCore *core);
// Restores a savestate made by gl_core_savestate (same ROM). Returns 0, or 1 on failure.
int gl_core_loadstate(GlCore *core, const uint8_t *state, uint32_t len);

#ifdef __cplusplus
}
#endif

#endif
