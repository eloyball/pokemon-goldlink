// SPDX-License-Identifier: GPL-3.0-or-later
//
// GoldLink (UC12): the 3D at screen resolution. melonDS's OpenGL renderer on an offscreen EGL
// context of its own, made on the emulating thread at the first frame that wants it; it draws the
// polygons the software renderer draws (GPU3D::HiresHook, patch 0025), and the picture is read
// back for the window. The software renderer keeps running: its masks and buffers (which 2D shows
// where, the lighting's depth) are still the picture's.
//
// Linux for now (EGL); elsewhere gl_core_set_hires says no.

#include "goldlink_core.h"
#include "hires_gl.h"

#include <cstring>
#include <memory>
#include <mutex>
#include <vector>

#include "NDS.h"

#if defined(OGLRENDERER_ENABLED) && defined(__linux__)

#include <EGL/egl.h>
#include <EGL/eglext.h>

#include "GPU3D_OpenGL.h"

namespace
{
struct HiresGl
{
    int wanted = 0; // the scale asked for; 0 off
    bool failed = false;
    EGLDisplay display = EGL_NO_DISPLAY;
    EGLContext context = EGL_NO_CONTEXT;
    std::unique_ptr<melonDS::GLRenderer> renderer;
    int width = 0, height = 0, scale = 0;
    // The last one drawn, the one before it (what the frame on screen shows: the 3D drawn at
    // VCount215 is shown in the next frame), and a spare.
    std::vector<uint32_t> last, shown, spare;
    int shown_scale = 0;
    std::mutex lock;
};

HiresGl g_hires;

EGLDisplay open_display()
{
    // A GPU of its own (EGL_EXT_platform_device: no window system needed), else Mesa's
    // surfaceless platform, else the default display.
    auto query = (PFNEGLQUERYDEVICESEXTPROC)eglGetProcAddress("eglQueryDevicesEXT");
    auto platform = (PFNEGLGETPLATFORMDISPLAYEXTPROC)eglGetProcAddress("eglGetPlatformDisplayEXT");
    if (query && platform)
    {
        EGLDeviceEXT devices[8];
        EGLint n = 0;
        if (query(8, devices, &n) && n > 0)
        {
            for (EGLint i = 0; i < n; i++)
            {
                EGLDisplay d = platform(EGL_PLATFORM_DEVICE_EXT, devices[i], nullptr);
                EGLint major = 0, minor = 0;
                if (d != EGL_NO_DISPLAY && eglInitialize(d, &major, &minor))
                    return d;
            }
        }
    }
    if (platform)
    {
        EGLDisplay d = platform(EGL_PLATFORM_SURFACELESS_MESA, EGL_DEFAULT_DISPLAY, nullptr);
        EGLint major = 0, minor = 0;
        if (d != EGL_NO_DISPLAY && eglInitialize(d, &major, &minor))
            return d;
    }
    EGLDisplay d = eglGetDisplay(EGL_DEFAULT_DISPLAY);
    EGLint major = 0, minor = 0;
    if (d != EGL_NO_DISPLAY && eglInitialize(d, &major, &minor))
        return d;
    return EGL_NO_DISPLAY;
}

bool fail(const char *what)
{
    fprintf(stderr, "[goldlink] OpenGL renderer: %s failed (EGL error 0x%x)\n", what, eglGetError());
    return false;
}

bool start(HiresGl &h)
{
    h.display = open_display();
    if (h.display == EGL_NO_DISPLAY)
        return fail("opening a display");
    fprintf(stderr, "[goldlink] OpenGL renderer: EGL %s, %s\n", eglQueryString(h.display, EGL_VENDOR),
            eglQueryString(h.display, EGL_VERSION));
    if (!eglBindAPI(EGL_OPENGL_API))
        return fail("eglBindAPI");
    // An offscreen config (NVIDIA's device platform has none for windows), else none at all
    // (EGL_KHR_no_config_context): nothing is drawn to a surface, only to the renderer's own
    // framebuffers.
    const EGLint config_attrs[] = {EGL_SURFACE_TYPE, EGL_PBUFFER_BIT, EGL_RENDERABLE_TYPE, EGL_OPENGL_BIT,
                                   EGL_NONE};
    EGLConfig config = (EGLConfig)0; // EGL_NO_CONFIG_KHR
    EGLint n = 0;
    if (!eglChooseConfig(h.display, config_attrs, &config, 1, &n) || n < 1)
        config = (EGLConfig)0;
    const EGLint context_attrs[] = {EGL_CONTEXT_MAJOR_VERSION, 3, EGL_CONTEXT_MINOR_VERSION, 2,
                                    EGL_CONTEXT_OPENGL_PROFILE_MASK, EGL_CONTEXT_OPENGL_CORE_PROFILE_BIT,
                                    EGL_NONE};
    h.context = eglCreateContext(h.display, config, EGL_NO_CONTEXT, context_attrs);
    if (h.context == EGL_NO_CONTEXT)
        return fail("eglCreateContext");
    if (!eglMakeCurrent(h.display, EGL_NO_SURFACE, EGL_NO_SURFACE, h.context))
        return fail("eglMakeCurrent");
    if (!gladLoadGLLoader((GLADloadproc)eglGetProcAddress))
        return fail("loading GL");
    fprintf(stderr, "[goldlink] OpenGL renderer: %s, %s\n", (const char *)glGetString(GL_RENDERER),
            (const char *)glGetString(GL_VERSION));
    h.renderer = melonDS::GLRenderer::New();
    if (!h.renderer)
        return fail("melonDS's GLRenderer");
    h.renderer->SetRenderSettings(false, h.wanted);
    h.scale = h.wanted;
    h.width = h.renderer->GoldLinkWidth();
    h.height = h.renderer->GoldLinkHeight();
    return true;
}

void hook(melonDS::GPU &gpu)
{
    HiresGl &h = g_hires;
    if (h.wanted == 0 || h.failed)
        return;
    if (!h.renderer && !start(h))
    {
        h.failed = true;
        fprintf(stderr, "[goldlink] the OpenGL renderer could not start: the 3D stays at the DS's resolution\n");
        return;
    }
    if (h.scale != h.wanted)
    {
        h.renderer->SetRenderSettings(false, h.wanted);
        h.scale = h.wanted;
        h.width = h.renderer->GoldLinkWidth();
        h.height = h.renderer->GoldLinkHeight();
    }
    h.renderer->RenderFrame(gpu);
    h.spare.resize((size_t)h.width * h.height);
    h.renderer->GoldLinkRead(h.spare.data());
    std::lock_guard<std::mutex> guard(h.lock);
    h.shown.swap(h.last);
    h.last.swap(h.spare);
    h.shown_scale = h.scale;
}
} // namespace

int hires_set(melonDS::NDS &, int scale)
{
    g_hires.wanted = scale < 0 ? 0 : scale;
    melonDS::GPU3D::Hires = g_hires.wanted > 0;
    melonDS::GPU3D::HiresHook = g_hires.wanted > 0 ? hook : nullptr;
    return !g_hires.failed;
}

int hires_frame(uint32_t *out, int cap, int *w, int *h, int *scale)
{
    HiresGl &g = g_hires;
    std::lock_guard<std::mutex> guard(g.lock);
    if (g.wanted == 0 || g.failed || g.shown.empty() || g.shown_scale == 0)
        return 0;
    *scale = g.shown_scale;
    *w = (int)(g.shown.size() / (192 * g.shown_scale)) ;
    *h = 192 * g.shown_scale;
    const size_t n = g.shown.size();
    if (out && (size_t)cap >= n)
        memcpy(out, g.shown.data(), n * 4);
    return 1;
}

#else

int hires_set(melonDS::NDS &, int) { return 0; }
int hires_frame(uint32_t *, int, int *, int *, int *) { return 0; }

#endif
