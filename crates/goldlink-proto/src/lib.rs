//! The GoldLink relay wire protocol (packets, trailers, constants), shared by `coop-relay` and
//! the unified client.
//!
//! The source still lives at `coop-relay/src/protocol.rs` and is compiled from there: master
//! keeps editing it (protocol bumps), and release and test scripts read `PROTOCOL_VERSION` from
//! that path. It moves into this crate once the BizHawk path is retired (UC5,
//! dev/docs/unified-client/UC0_PROPOSAL.md §3.2). The relay uses it through
//! `pub use goldlink_proto as protocol;`.

// rustfmt::skip: `cargo fmt` here must not rewrite a file the relay owns.
#[rustfmt::skip]
#[path = "../../../coop-relay/src/protocol.rs"]
mod protocol;

pub use protocol::*;
