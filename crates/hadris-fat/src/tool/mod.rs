//! Analysis and diagnostic tools for FAT filesystems.
//!
//! This module provides utilities for examining and verifying FAT filesystems,
//! including statistics gathering, fragmentation analysis, and integrity checking.

pub mod analysis;
pub mod verify;

pub use analysis::{ClusterState, FatStatistics, FileFragmentInfo, FragmentationReport};
pub use verify::{VerificationIssue, VerificationReport};
