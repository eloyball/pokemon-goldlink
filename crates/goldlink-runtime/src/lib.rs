//! GoldLink's game runtime, without a UI (unified-client RFC §4.3–4.4, UC2): one [`Emulator`]
//! per game (the DS, the world's save file, emergency backups), the emulation thread that keeps
//! the DS's 59.8261 Hz ([`runner`]), sound out ([`audio`]) and the player's settings
//! ([`config`]). The window app (`goldlink`) and the dev binary (`goldlink-dev`) build on it.

pub mod audio;
pub mod backups;
pub mod config;
pub mod emulator;
mod files;
pub mod runner;
pub mod saves;

pub use emulator::{Emulator, Event, GameSetup, Input, screen_hash};
pub use files::write_atomic;
pub use runner::{Command, FrameHook, Runner, Shared};
