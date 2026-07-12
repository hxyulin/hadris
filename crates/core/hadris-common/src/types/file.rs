//! Compatibility exports for fixed-capacity filesystem names.

/// A fixed-capacity byte buffer traditionally used for filesystem names.
///
/// New code should use [`hadris_fixed::FixedBytes`] directly. Filesystem names
/// are byte sequences and are not guaranteed to contain valid UTF-8.
#[deprecated(since = "2.0.0", note = "use hadris_fixed::FixedBytes")]
pub type FixedFilename<const N: usize> = hadris_fixed::FixedBytes<N>;
