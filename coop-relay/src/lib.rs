pub mod admission;
pub mod lobby;
pub mod quic;
// The wire protocol is the shared goldlink-proto crate (compiled from src/protocol.rs, where it
// still lives; crates/goldlink-proto/src/lib.rs).
pub use goldlink_proto as protocol;

#[cfg(test)]
#[path = "tests/lobby_tests.rs"]
mod lobby_tests;

#[cfg(test)]
#[path = "tests/session_tests.rs"]
mod session_tests;

#[cfg(test)]
#[path = "tests/trade_tests.rs"]
mod trade_tests;

#[cfg(test)]
#[path = "tests/link_tests.rs"]
mod link_tests;

#[cfg(test)]
#[path = "tests/challenge_tests.rs"]
mod challenge_tests;
