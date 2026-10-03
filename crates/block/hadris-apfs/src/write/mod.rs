//! APFS write support scaffolding.
//!
//! APFS writes require correct checkpoint, object-map, and space-manager updates.
//! The module is intentionally minimal until the read-side metadata walkers can
//! prove allocation state and transaction ordering.

/// Options controlling future APFS write transactions.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct WriteOptions {
    /// Verify checksums before modifying blocks.
    pub verify_before_write: bool,
}
