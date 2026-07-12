//! Compatibility exports for fixed-capacity UTF-16 text.

/// Little-endian, fixed-capacity UTF-16 text.
///
/// New code should use [`hadris_fixed::FixedUtf16Le`] directly.
#[deprecated(since = "2.0.0", note = "use hadris_fixed::FixedUtf16Le")]
pub type FixedUtf16Str<const N: usize> = hadris_fixed::FixedUtf16Le<N>;
