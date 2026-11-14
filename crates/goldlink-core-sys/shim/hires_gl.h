// SPDX-License-Identifier: GPL-3.0-or-later
// GoldLink (UC12): the shim's OpenGL renderer (hires_gl.cpp), for goldlink_core.cpp.
#pragma once
#include <cstdint>

namespace melonDS { class NDS; }

int hires_set(melonDS::NDS &nds, int scale);
int hires_frame(uint32_t *out, int cap, int *w, int *h, int *scale);
