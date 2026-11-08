// SPDX-License-Identifier: MIT
// GoldLink's own stand-in for hg-engine's types.h, used only by goldlink-abi-gen: the
// co-op headers' fixed-width types for the ARM9 (armv5te-none-eabi). The generator defines
// TYPES_H so hg-engine's own types.h is skipped by its include guard (DECISIONS.md D1 rule 6).
#ifndef GOLDLINK_TYPES_H
#define GOLDLINK_TYPES_H

typedef unsigned char u8;
typedef unsigned short u16;
typedef unsigned int u32;
typedef unsigned long long u64;
typedef signed char s8;
typedef signed short s16;
typedef signed int s32;
typedef signed long long s64;
typedef int BOOL;

#define TRUE 1
#define FALSE 0
#define NULL ((void *)0)
#define LONG_CALL

#endif
