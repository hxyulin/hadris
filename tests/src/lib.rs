//! Harness library for the Hadris conformance and interoperability suite.
//!
//! The crate is split into three layers:
//!
//! - [`harness`]: format-agnostic building blocks (temporary workspaces,
//!   external command execution, native mounts, scorecards, tree diffing).
//! - [`fat`] and [`iso`]: per-format semantic models, specification oracles,
//!   scenario generators, and adapters for every implementation under test.
//! - `suite/`: the test binary that combines the two into executable checks.
//!
//! Hadris is one adapter among peers. The oracles are the ground truth; a
//! Hadris-to-Hadris round trip is never used as evidence on its own.

pub mod fat;
pub mod harness;
pub mod iso;
