//! GitWell — surface abandoned work in git repositories.
//!
//! This is the library crate. The `gitwell` CLI binary at `src/main.rs`
//! is a thin wrapper over these modules, and the integration tests in
//! `tests/` exercise them directly via this public API.

pub mod cluster;
pub mod config;
pub mod execute;
pub mod git;
pub mod hook;
pub mod json;
pub mod narrative;
pub mod report;
pub mod report_md;
pub mod scanner;
pub mod trends;
pub mod triage;
pub mod triage_state;
pub mod util;
