# SPDX-License-Identifier: GPL-3.0-or-later
# GoldLink's core library: melonDS's core (the submodule, never edited) plus the C ABI shim.
# build.rs configures it with the options below forced off and builds the goldlink_core target.
# The shim links melonDS's `core` target, so it is compiled with the core's own include paths
# and public definitions (the class layouts must match: see shim/goldlink_core.cpp).
cmake_minimum_required(VERSION 3.16)
project(goldlink_core C CXX)

set(BUILD_QT_SDL OFF CACHE BOOL "" FORCE)
set(ENABLE_JIT OFF CACHE BOOL "" FORCE)
set(ENABLE_OGLRENDERER OFF CACHE BOOL "" FORCE)
set(ENABLE_GDBSTUB OFF CACHE BOOL "" FORCE)
set(ENABLE_LTO_RELEASE OFF CACHE BOOL "" FORCE)
set(USE_VCPKG OFF CACHE BOOL "" FORCE)
set(MELONDS_EMBED_BUILD_INFO OFF CACHE BOOL "" FORCE)

add_subdirectory(melonDS)

add_library(goldlink_core STATIC shim/goldlink_core.cpp shim/platform.cpp)
set_target_properties(goldlink_core PROPERTIES CXX_STANDARD 17 CXX_STANDARD_REQUIRED ON)
target_include_directories(goldlink_core PUBLIC "${CMAKE_CURRENT_SOURCE_DIR}/shim")
target_link_libraries(goldlink_core PRIVATE core)
