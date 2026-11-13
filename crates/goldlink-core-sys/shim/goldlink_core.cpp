// SPDX-License-Identifier: GPL-3.0-or-later
//
// GoldLink's C ABI over melonDS 1.1 (goldlink_core.h). The only file, with platform.cpp, that
// includes melonDS's headers: an upstream API change is fixed here.
//
// ABI note: melonDS's classes change layout with JIT_ENABLED, OGLRENDERER_ENABLED and
// GDBSTUB_ENABLED. This file is a target of the same CMake build as the core (CMakeLists.txt next
// to it), all three off, so both sides agree; the #error below fails the build otherwise.

#include "goldlink_core.h"

#include <algorithm>
#include <cstring>
#include <memory>
#include <optional>
#include <vector>

#include "Args.h"
#include "NDS.h"
#include "NDSCart.h"
#include "SPI_Firmware.h"
#include "Savestate.h"
#include "GPU3D_Soft.h"
#include "GPU2D_Soft.h"

#if defined(JIT_ENABLED) || defined(OGLRENDERER_ENABLED) || defined(GDBSTUB_ENABLED)
#error "goldlink-core-sys builds melonDS without the JIT, the OpenGL renderer and the GDB stub"
#endif

struct GlCore
{
    std::unique_ptr<melonDS::NDS> nds;
    GlSaveCallback on_save = nullptr;
    void *save_ctx = nullptr;
    std::vector<uint8_t> state; // the last gl_core_savestate
};

// Called by platform.cpp's Platform::WriteNDSSave (the cart and the NDS carry the GlCore as
// their userdata).
void gl_core_on_save(void *userdata, const uint8_t *save, uint32_t save_len, uint32_t offset, uint32_t len)
{
    auto *core = static_cast<GlCore *>(userdata);
    if (core != nullptr && core->on_save != nullptr)
        core->on_save(core->save_ctx, save, save_len, offset, len);
}

static melonDS::Firmware make_firmware(const GlFirmwareUser &user)
{
    melonDS::Firmware firmware(0); // generated, DS (not DSi)
    auto &data = firmware.GetEffectiveUserData();
    uint32_t nick = std::min<uint32_t>(user.nickname_len, 10);
    if (nick > 0)
    {
        std::memset(data.Nickname, 0, sizeof(data.Nickname));
        std::memcpy(data.Nickname, user.nickname, nick * sizeof(char16_t));
        data.NameLength = nick;
    }
    uint32_t msg = std::min<uint32_t>(user.message_len, 26);
    std::memset(data.Message, 0, sizeof(data.Message));
    std::memcpy(data.Message, user.message, msg * sizeof(char16_t));
    data.MessageLength = msg;
    if (user.birthday_month >= 1 && user.birthday_month <= 12)
        data.BirthdayMonth = user.birthday_month;
    if (user.birthday_day >= 1 && user.birthday_day <= 31)
        data.BirthdayDay = user.birthday_day;
    data.FavoriteColor = user.favorite_color & 0x0F;
    data.Settings = static_cast<uint16_t>((data.Settings & ~0x7u) | (user.language & 0x7u));
    firmware.UpdateChecksums();
    return firmware;
}

extern "C" {

GlCore *gl_core_new(const GlCoreConfig *cfg)
{
    if (cfg == nullptr)
        return nullptr;
    auto core = std::make_unique<GlCore>();
    core->on_save = cfg->on_save;
    core->save_ctx = cfg->save_ctx;

    melonDS::NDSArgs args {}; // FreeBIOS images and the software 3D renderer by default
    args.Firmware = make_firmware(cfg->user);
    args.JIT = std::nullopt;
    args.GDB = std::nullopt;
    core->nds = std::make_unique<melonDS::NDS>(std::move(args), core.get());
    return core.release();
}

void gl_core_free(GlCore *core)
{
    delete core;
}

int gl_core_load_rom(GlCore *core, const uint8_t *rom, uint32_t rom_len, const uint8_t *save, uint32_t save_len)
{
    auto romdata = std::make_unique<uint8_t[]>(rom_len);
    std::memcpy(romdata.get(), rom, rom_len);
    melonDS::NDSCart::NDSCartArgs cartargs {};
    if (save != nullptr && save_len > 0)
    {
        cartargs.SRAM = std::make_unique<uint8_t[]>(save_len);
        std::memcpy(cartargs.SRAM.get(), save, save_len);
        cartargs.SRAMLength = save_len;
    }
    auto cart = melonDS::NDSCart::ParseROM(std::move(romdata), rom_len, core, std::move(cartargs));
    if (!cart)
        return 1;

    melonDS::NDS &nds = *core->nds;
    nds.SetNDSCart(std::move(cart));
    nds.Reset();
    nds.SetupDirectBoot("goldlink.nds"); // FreeBIOS needs it; BizHawk's SkipFirmware does the same
    nds.Start();
    return 0;
}

void gl_core_set_rtc(GlCore *core, int year, int month, int day, int hour, int minute, int second)
{
    core->nds->RTC.SetDateTime(year, month, day, hour, minute, second);
}

uint32_t gl_core_run_frame(GlCore *core)
{
    core->nds->RunFrame();
    return core->nds->NumFrames;
}

void gl_core_set_keys(GlCore *core, uint32_t held)
{
    core->nds->SetKeyMask(~held & 0xFFF); // melonDS: a set bit = released
}

void gl_core_set_touch(GlCore *core, int touching, uint16_t x, uint16_t y)
{
    if (touching)
        core->nds->TouchScreen(std::min<uint16_t>(x, GL_SCREEN_WIDTH - 1), std::min<uint16_t>(y, GL_SCREEN_HEIGHT - 1));
    else
        core->nds->ReleaseScreen();
}

uint8_t *gl_core_main_ram(GlCore *core, uint32_t *len)
{
    if (len != nullptr)
        *len = core->nds->MainRAMMask + 1;
    return core->nds->MainRAM;
}

uint8_t gl_core_arm9_peek8(GlCore *core, uint32_t addr)
{
    melonDS::NDS &nds = *core->nds;
    switch (addr >> 24)
    {
    case 0x02: return nds.MainRAM[addr & nds.MainRAMMask];
    case 0x04: return 0; // I/O: reads can have side effects
    default: return nds.ARM9Read8(addr);
    }
}

uint8_t *gl_core_save(GlCore *core, uint32_t *len)
{
    if (len != nullptr)
        *len = core->nds->GetNDSSaveLength();
    return core->nds->GetNDSSave();
}

const uint32_t *gl_core_framebuffer(GlCore *core, int screen)
{
    melonDS::GPU &gpu = core->nds->GPU;
    return gpu.Framebuffer[gpu.FrontBuffer][screen == 0 ? 0 : 1].get();
}

void gl_core_set_wide(GlCore *core, int side)
{
    core->nds->GPU.GPU3D.WideSide = std::clamp(side, 0, melonDS::GPU3D::MaxWideSide);
}

void gl_core_set_steady(GlCore *core, int on)
{
    core->nds->GPU.GPU3D.OrthoWanted = on != 0;
}

void gl_core_set_field(GlCore *core, int on)
{
    core->nds->GPU.GPU3D.WideField = on != 0;
}

int gl_core_steady_shift(GlCore *core, float *xy)
{
    melonDS::GPU &gpu = core->nds->GPU;
    const auto &gpu3d = gpu.GPU3D;
    xy[0] = gpu3d.ShownOrthoShift[0];
    xy[1] = gpu3d.ShownOrthoShift[1];
    if (!gpu3d.ShownOrtho)
        return 0;
    // 2D drawn over the scene on the top screen (a text box, a town's banner): it mustn't move
    // with the scene, so the picture isn't slid.
    auto &soft2d = static_cast<melonDS::GPU2D::SoftRenderer &>(gpu.GetRenderer2D());
    int over = 0;
    for (int y = 0; y < 192; y++)
        for (int x = 0; x < 256; x++)
            over += soft2d.Layer3D[y][melonDS::GPU3D::MaxWideSide + x] == 2;
    return over > 128 ? 2 : 1;
}

int gl_core_wide_frame(GlCore *core, int screen, int side, uint32_t *out)
{
    side = std::clamp(side, 0, melonDS::GPU3D::MaxWideSide);
    const int width = 256 + 2 * side;
    melonDS::GPU &gpu = core->nds->GPU;
    const uint32_t *fb = gpu.Framebuffer[gpu.FrontBuffer][screen == 0 ? 0 : 1].get();
    auto &soft = static_cast<melonDS::SoftRenderer &>(gpu.GPU3D.GetCurrentRenderer());
    // The 2D engine's side strips for the engine that drew this screen, when it drew them.
    auto &soft2d = static_cast<melonDS::GPU2D::SoftRenderer &>(gpu.GetRenderer2D());
    int unit = -1;
    for (int u = 0; u < 2; u++)
        if (soft2d.WideOwner[u] == fb && soft2d.WideDrawn[u] >= side)
            unit = u;
    // In the overworld, side pixels where the 3D scene shows nothing (past the map, ground the
    // game never loaded and patch 0006 skips) would show the 2D backdrop or the clear colour: a
    // small gap takes the nearest scene pixel in its row, toward the screen (up to 16 pixels
    // away); anything wider is black.
    const bool field = screen == 0 && gpu.GPU3D.WideField;
    // Only the overworld's own outdoor camera (its perspective, or the steady overworld's flat
    // view of it) is widened: rooms and special cameras (a cave's entrance, cutscenes) show
    // geometry past the DS's view that the game never finished or culls, so their sides are
    // black, the DS's own view.
    const int32_t *proj = gpu.GPU3D.ShownProjMatrix;
    const bool outdoor = proj[11] == -0x1000
        || proj[15] == (int32_t)(melonDS::GPU3D::OrthoW * 4096);
    const bool blank = field && !outdoor;
    for (int y = 0; y < 192; y++)
    {
        const uint32_t *wide = soft.GetWideLine(y) + (melonDS::GPU3D::MaxWideSide - side);
        // Where the 3D scene shows in the composed strips (or, without them, in the 3D line).
        const uint8_t *shows = soft2d.Layer3D[y] + (melonDS::GPU3D::MaxWideSide - side);
        auto scene_at = [&](int nx) { return unit >= 0 ? shows[nx] != 0 : (wide[nx] >> 24) != 0; };
        uint32_t *row = out + y * width;
        for (int x = 0; x < width; x++)
        {
            int sx = x - side;
            if (sx >= 0 && sx < 256)
            {
                row[x] = fb[y * 256 + sx];
                continue;
            }
            if (blank)
            {
                row[x] = 0xFF000000;
                continue;
            }
            if (field && !scene_at(x))
            {
                row[x] = 0xFF000000;
                const int step = sx < 0 ? 1 : -1;
                for (int k = 1; k <= 16; k++)
                {
                    const int nx = x + step * k;
                    if (nx < 0 || nx >= width)
                        break;
                    const int nsx = nx - side;
                    if (nsx >= 0 && nsx < 256)
                    {
                        row[x] = fb[y * 256 + nsx];
                        break;
                    }
                    if (scene_at(nx))
                    {
                        const uint32_t *strips = unit >= 0 ? soft2d.WideSides[unit][y] : nullptr;
                        if (strips)
                            row[x] = nsx < 0 ? strips[nx] : strips[melonDS::GPU2D::SoftRenderer::MaxSide + (nsx - 256)];
                        else
                        {
                            uint32_t c = wide[nx];
                            auto ch = [](uint32_t v) { v &= 0x3F; return (v << 2) | (v >> 4); };
                            row[x] = 0xFF000000 | (ch(c) << 16) | (ch(c >> 8) << 8) | ch(c >> 16);
                        }
                        break;
                    }
                }
                continue;
            }
            if (unit >= 0)
            {
                const uint32_t *strips = soft2d.WideSides[unit][y];
                row[x] = sx < 0 ? strips[x] : strips[melonDS::GPU2D::SoftRenderer::MaxSide + (sx - 256)];
                continue;
            }
            uint32_t c = wide[x];
            if ((c >> 24) == 0)
            {
                row[x] = 0;
                continue;
            }
            auto ch = [](uint32_t v) { v &= 0x3F; return (v << 2) | (v >> 4); };
            row[x] = 0xFF000000 | (ch(c) << 16) | (ch(c >> 8) << 8) | ch(c >> 16);
        }
    }
    return width;
}

int gl_core_light_buffers(GlCore *core, uint32_t *depth, uint8_t *layer, int32_t proj[16])
{
    static_assert(melonDS::GPU3D::MaxWideSide == melonDS::GPU2D::SoftRenderer::MaxSide);
    constexpr int side = melonDS::GPU3D::MaxWideSide;
    constexpr int width = 256 + 2 * side;
    melonDS::GPU &gpu = core->nds->GPU;
    const uint32_t cnt = gpu.GPU2D_A.DispCnt;
    if (!(core->nds->PowerControl9 & (1 << 15)) || ((cnt >> 16) & 3) != 1 || !(cnt & (1 << 3)))
        return 0;
    auto &soft = static_cast<melonDS::SoftRenderer &>(gpu.GPU3D.GetCurrentRenderer());
    auto &soft2d = static_cast<melonDS::GPU2D::SoftRenderer &>(gpu.GetRenderer2D());
    // The side strips' mask, when the 2D engine composed them for this picture; otherwise the
    // sides show the 3D layer alone (gl_core_wide_frame).
    const uint32_t *top = gpu.Framebuffer[gpu.FrontBuffer][0].get();
    const bool strips = soft2d.WideOwner[0] == top && soft2d.WideDrawn[0] > 0;
    for (int y = 0; y < 192; y++)
    {
        // The depth of the picture on screen (the buffer itself is already the next frame's).
        std::memcpy(depth + y * width, soft.GetShownDepthLine(y), width * 4);
        uint8_t *row = layer + y * width;
        std::memcpy(row, soft2d.Layer3D[y], width);
        if (!strips)
        {
            const uint32_t *colour = soft.GetWideLine(y);
            for (int x = 0; x < width; x++)
                if (x < side || x >= side + 256)
                    row[x] = (colour[x] >> 24) != 0 ? 1 : 0;
        }
    }
    std::memcpy(proj, gpu.GPU3D.ShownProjMatrix, 16 * 4);
    return gpu.GPU3D.ShownWBuffer ? 2 : 1;
}

void gl_core_set_sprite_move(GlCore *core, const int32_t *rects, int count)
{
    auto &soft2d = static_cast<melonDS::GPU2D::SoftRenderer &>(core->nds->GPU.GetRenderer2D());
    count = std::clamp(count, 0, melonDS::GPU2D::SoftRenderer::MaxObjMove);
    // Only while the main engine draws the top screen (POWCNT1 bit 15): a screen the battle opens
    // on the touch screen with the main engine (the nickname keyboard after a catch) keeps its
    // sprites where they are.
    if (!(core->nds->PowerControl9 & (1 << 15)))
        count = 0;
    for (int i = 0; i < count; i++)
    {
        const int32_t *r = rects + i * 5;
        soft2d.ObjMove[i] = {r[0], r[1], r[2], r[3], r[4]};
    }
    soft2d.ObjMoveCount = count;
}

uint32_t gl_core_audio(GlCore *core, int16_t *out, uint32_t max_frames)
{
    int n = core->nds->SPU.ReadOutput(out, static_cast<int>(max_frames));
    return n > 0 ? static_cast<uint32_t>(n) : 0;
}

void gl_core_set_audio_rate(GlCore *core, double rate)
{
    if (rate >= 8000.0 && rate <= 192000.0)
        core->nds->SPU.SetOutputSampleRate(rate);
}

void gl_core_registers(GlCore *core, int cpu, uint32_t out[17])
{
    melonDS::NDS &nds = *core->nds;
    const uint32_t *r = cpu == 7 ? nds.ARM7.R : nds.ARM9.R;
    std::memcpy(out, r, 16 * sizeof(uint32_t));
    out[16] = cpu == 7 ? nds.ARM7.CPSR : nds.ARM9.CPSR;
}

uint32_t gl_core_savestate(GlCore *core)
{
    melonDS::Savestate state;
    if (state.Error || !core->nds->DoSavestate(&state))
        return 0;
    state.Finish();
    if (state.Error)
        return 0;
    const auto *p = static_cast<const uint8_t *>(state.Buffer());
    core->state.assign(p, p + state.Length());
    return static_cast<uint32_t>(core->state.size());
}

const uint8_t *gl_core_savestate_data(GlCore *core)
{
    return core->state.data();
}

int gl_core_loadstate(GlCore *core, const uint8_t *data, uint32_t len)
{
    std::vector<uint8_t> copy(data, data + len); // melonDS's reader takes a mutable buffer
    melonDS::Savestate state(copy.data(), len, false);
    if (state.Error || !core->nds->DoSavestate(&state) || state.Error)
        return 1;
    return 0;
}

} // extern "C"
