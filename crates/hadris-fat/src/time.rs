//! Time / date types and the [`TimeProvider`] trait.
//!
//! FAT directory entries store creation, last-access, and last-modified
//! timestamps in a packed format with 2-second resolution and dates from
//! 1980-01-01 to 2107-12-31. [`FatDateTime`] is the in-memory representation;
//! [`TimeProvider`] is the pluggable clock used by the writer.

/// FAT date/time representation for directory entries.
///
/// Stored on disk as two `u16`s (date + time) plus an optional 10-ms-units
/// field used for the creation timestamp. Date covers 1980-2107 with
/// 1-day granularity; time covers a single day with 2-second granularity.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct FatDateTime {
    /// Date: `(year-1980)<<9 | month<<5 | day`.
    pub date: u16,
    /// Time: `hour<<11 | minute<<5 | (second/2)`.
    pub time: u16,
    /// 10-ms units (0-199) for sub-2-second creation precision.
    pub time_tenth: u8,
}

impl FatDateTime {
    /// The FAT epoch: 1980-01-01 00:00:00.
    pub const EPOCH: Self = Self {
        date: (1u16 << 5) | 1,
        time: 0,
        time_tenth: 0,
    };

    /// Build a `FatDateTime` from broken-down calendar components.
    ///
    /// Out-of-range values are silently clamped: years before 1980 become 1980,
    /// years after 2107 become 2107. No calendar validation is performed
    /// (e.g. Feb 30 is accepted as-is — FAT directory entries store these
    /// fields verbatim).
    pub fn new(year: u16, month: u8, day: u8, hour: u8, minute: u8, second: u8) -> Self {
        let year_offset = year.saturating_sub(1980).min(127);
        let date = (year_offset << 9) | ((month as u16 & 0x0F) << 5) | (day as u16 & 0x1F);
        let time = ((hour as u16 & 0x1F) << 11)
            | ((minute as u16 & 0x3F) << 5)
            | ((second as u16 / 2) & 0x1F);
        Self {
            date,
            time,
            time_tenth: 0,
        }
    }

    /// Convert to the raw on-disk triple `(date, time, time_tenth)`.
    pub fn to_raw(&self) -> (u16, u16, u8) {
        (self.date, self.time, self.time_tenth)
    }

    /// Reconstruct from a raw on-disk triple.
    pub fn from_raw(date: u16, time: u16, time_tenth: u8) -> Self {
        Self {
            date,
            time,
            time_tenth,
        }
    }

    /// Current wall-clock time.
    ///
    /// Uses `chrono::Local::now()` when the `std` feature is on; otherwise
    /// returns [`FatDateTime::EPOCH`]. Equivalent to
    /// `ChronoTimeProvider.now()` / `EpochTimeProvider.now()` and provided
    /// as a shortcut so most callers don't need to wire a [`TimeProvider`]
    /// explicitly.
    #[cfg(feature = "std")]
    pub fn now() -> Self {
        ChronoTimeProvider.now()
    }

    /// No-std fallback: returns [`FatDateTime::EPOCH`].
    #[cfg(not(feature = "std"))]
    pub fn now() -> Self {
        Self::EPOCH
    }
}

impl Default for FatDateTime {
    fn default() -> Self {
        Self::now()
    }
}

/// Pluggable clock used to stamp newly-created or modified directory entries.
///
/// Implementations only need to provide a single [`now`](Self::now) method.
/// The type is intentionally trait-object friendly: the writer holds it as
/// `&dyn TimeProvider` so callers can switch providers per-`FatFs` instance
/// without leaking generics through the public API.
pub trait TimeProvider: core::fmt::Debug {
    /// Return the current FAT-encoded date/time.
    fn now(&self) -> FatDateTime;
}

/// Time provider backed by `chrono::Local::now()`.
///
/// Available only with the `std` feature (chrono's `clock` feature is enabled
/// transitively). Captures sub-second precision into [`FatDateTime::time_tenth`].
#[cfg(feature = "std")]
#[derive(Debug, Default, Clone, Copy)]
pub struct ChronoTimeProvider;

#[cfg(feature = "std")]
impl TimeProvider for ChronoTimeProvider {
    fn now(&self) -> FatDateTime {
        use chrono::{Datelike, Local, Timelike};
        let now = Local::now();
        let mut dt = FatDateTime::new(
            now.year() as u16,
            now.month() as u8,
            now.day() as u8,
            now.hour() as u8,
            now.minute() as u8,
            now.second() as u8,
        );
        let millis = now.timestamp_subsec_millis();
        dt.time_tenth = ((now.second() % 2) as u32 * 100 + millis / 10).min(199) as u8;
        dt
    }
}

/// Time provider that always returns [`FatDateTime::EPOCH`] (1980-01-01).
///
/// The right default for embedded targets without an RTC: produces stable,
/// reproducible timestamps without dragging in `chrono`.
#[derive(Debug, Default, Clone, Copy)]
pub struct EpochTimeProvider;

impl TimeProvider for EpochTimeProvider {
    fn now(&self) -> FatDateTime {
        FatDateTime::EPOCH
    }
}

/// Default provider used when none is set on a `FatFs`.
///
/// Resolves to [`ChronoTimeProvider`] under `std`, [`EpochTimeProvider`]
/// otherwise, so consumers don't have to think about the cfg matrix.
#[cfg(feature = "std")]
pub static DEFAULT_TIME_PROVIDER: ChronoTimeProvider = ChronoTimeProvider;
#[cfg(not(feature = "std"))]
pub static DEFAULT_TIME_PROVIDER: EpochTimeProvider = EpochTimeProvider;

/// Time provider that always returns a fixed [`FatDateTime`].
///
/// Useful for deterministic tests and image-reproducibility builds.
#[derive(Debug, Clone, Copy)]
pub struct StaticTimeProvider(pub FatDateTime);

impl TimeProvider for StaticTimeProvider {
    fn now(&self) -> FatDateTime {
        self.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn epoch_round_trips() {
        let raw = FatDateTime::EPOCH.to_raw();
        let back = FatDateTime::from_raw(raw.0, raw.1, raw.2);
        assert_eq!(back, FatDateTime::EPOCH);
    }

    #[test]
    fn epoch_provider_returns_epoch() {
        assert_eq!(EpochTimeProvider.now(), FatDateTime::EPOCH);
    }

    #[test]
    fn static_provider_returns_constructed_value() {
        let dt = FatDateTime::new(2030, 6, 15, 12, 0, 0);
        assert_eq!(StaticTimeProvider(dt).now(), dt);
    }

    #[test]
    fn new_clamps_years_outside_fat_range() {
        let pre = FatDateTime::new(1979, 1, 1, 0, 0, 0);
        assert_eq!(pre.date >> 9, 0); // year offset clamped to 0 (1980)
        let post = FatDateTime::new(2200, 1, 1, 0, 0, 0);
        assert_eq!(post.date >> 9, 127); // year offset clamped to 127 (2107)
    }

    #[cfg(feature = "std")]
    #[test]
    fn chrono_provider_produces_year_in_fat_range() {
        let dt = ChronoTimeProvider.now();
        let year_offset = dt.date >> 9;
        assert!(year_offset > 0 && year_offset <= 127);
    }
}
