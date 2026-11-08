//! The ROM's co-op interface as Rust types: the mailbox (`gCoopCommBlock`), the trade
//! exchange, the virtual link block, the challenge sync block and their constants, generated
//! by `goldlink-abi-gen` from `hg-engine/include/coop_sync.h` and `coop_challenge.h`.
//!
//! Layouts are the ARM9's: pointers in ROM structs are `u32` (ARM9 addresses), and the sizes
//! and offsets are asserted at compile time against what clang computed for the ROM's target.
//! The mailbox address is runtime data (`coop_offsets.json`, later `release.json`), not here.
//!
//! Enums are modules of integer constants (`CoopActivity::COOP_ACTIVITY_EVENT`): a value read
//! from RAM is untrusted and may be out of range.

#![no_std]
#![allow(non_camel_case_types, non_snake_case, non_upper_case_globals)]

#[cfg(target_endian = "big")]
compile_error!(
    "goldlink-abi reads the ARM9's little-endian layouts in place: little-endian hosts only"
);

#[rustfmt::skip]
mod generated;

pub use generated::*;
