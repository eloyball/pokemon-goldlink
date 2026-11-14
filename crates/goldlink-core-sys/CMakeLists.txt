# SPDX-License-Identifier: GPL-3.0-or-later
# GoldLink's core library: melonDS's core plus the C ABI shim. melonDS is the submodule with
# GoldLink's patches (patches/) applied: build.rs applies them to a copy and passes its path as
# MELONDS_DIR (the submodule itself stays at upstream).
# build.rs configures it with the options below forced off and builds the goldlink_core target.
# The shim links melonDS's `core` target, so it is compiled with the core's own include paths
# and public definitions (the class layouts must match: see shim/goldlink_core.cpp).
cmake_minimum_required(VERSION 3.16)
project(goldlink_core C CXX)

set(BUILD_QT_SDL OFF CACHE BOOL "" FORCE)
set(ENABLE_JIT OFF CACHE BOOL "" FORCE)
# UC12: melonDS's OpenGL renderer draws the 3D at screen resolution (Linux: EGL).
if (UNIX AND NOT APPLE)
    set(ENABLE_OGLRENDERER ON CACHE BOOL "" FORCE)
else()
    set(ENABLE_OGLRENDERER OFF CACHE BOOL "" FORCE)
endif()
set(ENABLE_GDBSTUB OFF CACHE BOOL "" FORCE)
set(ENABLE_LTO_RELEASE OFF CACHE BOOL "" FORCE)
set(USE_VCPKG OFF CACHE BOOL "" FORCE)
set(MELONDS_EMBED_BUILD_INFO OFF CACHE BOOL "" FORCE)

set(MELONDS_DIR "${CMAKE_CURRENT_SOURCE_DIR}/melonDS" CACHE PATH "melonDS's source tree")
add_subdirectory(${MELONDS_DIR} melonDS)

add_library(goldlink_core STATIC shim/goldlink_core.cpp shim/platform.cpp shim/hires_gl.cpp)
if (ENABLE_OGLRENDERER)
    # melonDS's GL declarations: its glad (as its Qt frontend sets it), for every target.
    target_compile_definitions(core PUBLIC MELONDS_GL_HEADER=\"frontend/glad/glad.h\")
    target_sources(goldlink_core PRIVATE ${MELONDS_DIR}/src/frontend/glad/glad.c)
endif()
set_target_properties(goldlink_core PROPERTIES CXX_STANDARD 17 CXX_STANDARD_REQUIRED ON)
target_include_directories(goldlink_core PUBLIC "${CMAKE_CURRENT_SOURCE_DIR}/shim")
target_link_libraries(goldlink_core PRIVATE core)
