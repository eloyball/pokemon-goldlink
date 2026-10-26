pub mod admission;
pub mod lobby;
pub mod protocol;

#[cfg(test)]
#[path = "tests/lobby_tests.rs"]
mod lobby_tests;

#[cfg(test)]
#[path = "tests/session_tests.rs"]
mod session_tests;
