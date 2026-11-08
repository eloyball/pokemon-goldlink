//! The co-op bridge in Rust (unified-client RFC §4, UC3): the C# CoopBridge ported to the new
//! client. It reads and writes the ROM's mailbox between frames, talks to the relay, keeps one
//! save per world and the emergency backups. Same files as the BizHawk bridge
//! (`coop_config.json`, `coop_worlds.json`, `coop_saves/`), so an install moves over as is.
//! The virtual link (`CLK1`: link battles, trades in the Union Room, the Colosseum) rides the
//! same packets (`link.rs`, UC4).

pub mod avatars;
pub mod bridge;
pub mod config;
pub mod hook;
pub mod layout;
pub mod link;
pub mod net;
pub mod quic;
pub mod sanitize;
pub mod slots;
pub mod text;
pub mod worlds;

pub use bridge::{Actions, Bridge, LinkState, Machine, View};
pub use hook::{BridgeHook, SharedBridge};
