//! exFAT Timestamp implementation.
//!
//! exFAT timestamps have the same bit layout as FAT32 timestamps,
//! but with additional fields for:
//! - 10ms precision (0-199)
//! - UTC offset in 15-minute increments

/// exFAT timestamp with UTC offset support.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExFatTimestamp {
    /// Combined date and time value (same layout as FAT)
    timestamp: u32,
    /// 10ms increment (0-199, adds 0-1990ms to the time)
    increment_10ms: u8,
    /// UTC offset in 15-minute increments (-48 to +56, 0x80 = invalid/local)
    utc_offset: i8,
}

impl ExFatTimestamp {
    /// Invalid/unset UTC offset marker
    pub const INVALID_UTC_OFFSET: u8 = 0x80;

    /// Create a new timestamp.
    pub fn new(timestamp: u32, increment_10ms: u8, utc_offset: u8) -> Self {
        Self {
            timestamp,
            increment_10ms: increment_10ms.min(199),
            utc_offset: if utc_offset == Self::INVALID_UTC_OFFSET {
                i8::MIN // Use i8::MIN to represent invalid
            } else {
                utc_offset as i8
            },
        }
    }

    /// Get the raw timestamp value.
    pub fn raw_timestamp(&self) -> u32 {
        self.timestamp
    }

    /// Get the 10ms increment.
    pub fn increment_10ms(&self) -> u8 {
        self.increment_10ms
    }

    /// Check if the UTC offset is valid.
    pub fn has_valid_utc_offset(&self) -> bool {
        self.utc_offset != i8::MIN
    }

    /// Get the UTC offset in minutes.
    ///
    /// Returns `None` if the offset is invalid.
    pub fn utc_offset_minutes(&self) -> Option<i16> {
        if self.utc_offset == i8::MIN {
            None
        } else {
            Some(self.utc_offset as i16 * 15)
        }
    }

    /// Get the year (1980-2107).
    pub fn year(&self) -> u16 {
        ((self.timestamp >> 25) & 0x7F) as u16 + 1980
    }

    /// Get the month (1-12).
    pub fn month(&self) -> u8 {
        ((self.timestamp >> 21) & 0x0F) as u8
    }

    /// Get the day of month (1-31).
    pub fn day(&self) -> u8 {
        ((self.timestamp >> 16) & 0x1F) as u8
    }

    /// Get the hour (0-23).
    pub fn hour(&self) -> u8 {
        ((self.timestamp >> 11) & 0x1F) as u8
    }

    /// Get the minute (0-59).
    pub fn minute(&self) -> u8 {
        ((self.timestamp >> 5) & 0x3F) as u8
    }

    /// Get the second (0-59).
    ///
    /// Note: The base timestamp has 2-second granularity.
    /// Use `second_with_10ms()` for full precision.
    pub fn second(&self) -> u8 {
        ((self.timestamp & 0x1F) * 2) as u8
    }

    /// Get the second with 10ms precision.
    pub fn second_with_10ms(&self) -> (u8, u16) {
        let base_second = self.second();
        let ms = (self.increment_10ms as u16) * 10;
        (base_second, ms)
    }

    /// Create a timestamp from components.
    pub fn from_components(
        year: u16,
        month: u8,
        day: u8,
        hour: u8,
        minute: u8,
        second: u8,
        increment_10ms: u8,
        utc_offset_minutes: Option<i16>,
    ) -> Self {
        let year_val = (year.saturating_sub(1980).min(127) as u32) << 25;
        let month_val = ((month.min(12).max(1)) as u32) << 21;
        let day_val = ((day.min(31).max(1)) as u32) << 16;
        let hour_val = ((hour.min(23)) as u32) << 11;
        let minute_val = ((minute.min(59)) as u32) << 5;
        let second_val = (second.min(59) / 2) as u32;

        let timestamp = year_val | month_val | day_val | hour_val | minute_val | second_val;

        let utc_offset = match utc_offset_minutes {
            Some(minutes) => (minutes / 15).clamp(-48, 56) as i8,
            None => i8::MIN,
        };

        Self {
            timestamp,
            increment_10ms: increment_10ms.min(199),
            utc_offset,
        }
    }

    /// Get the raw UTC offset byte for writing to disk.
    pub fn raw_utc_offset(&self) -> u8 {
        if self.utc_offset == i8::MIN {
            Self::INVALID_UTC_OFFSET
        } else {
            self.utc_offset as u8
        }
    }

    /// Create a timestamp representing the current time.
    #[cfg(feature = "std")]
    pub fn now() -> Self {
        use chrono::{Datelike, Local, Timelike};

        let now = Local::now();
        let offset_minutes = now.offset().local_minus_utc() / 60;

        Self::from_components(
            now.year() as u16,
            now.month() as u8,
            now.day() as u8,
            now.hour() as u8,
            now.minute() as u8,
            now.second() as u8,
            ((now.nanosecond() / 10_000_000) % 200) as u8,
            Some(offset_minutes as i16),
        )
    }

    /// Create a timestamp for DOS epoch (January 1, 1980, 00:00:00).
    pub fn dos_epoch() -> Self {
        Self::from_components(1980, 1, 1, 0, 0, 0, 0, None)
    }
}

impl Default for ExFatTimestamp {
    fn default() -> Self {
        Self::dos_epoch()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_timestamp_components() {
        let ts = ExFatTimestamp::from_components(2024, 6, 15, 14, 30, 45, 50, Some(0));

        assert_eq!(ts.year(), 2024);
        assert_eq!(ts.month(), 6);
        assert_eq!(ts.day(), 15);
        assert_eq!(ts.hour(), 14);
        assert_eq!(ts.minute(), 30);
        assert_eq!(ts.second(), 44); // Rounded down to 2-second boundary
        assert_eq!(ts.increment_10ms(), 50);
        assert_eq!(ts.utc_offset_minutes(), Some(0));
    }

    #[test]
    fn test_utc_offset() {
        // UTC+5:30 (330 minutes)
        let ts = ExFatTimestamp::from_components(2024, 1, 1, 0, 0, 0, 0, Some(330));
        assert_eq!(ts.utc_offset_minutes(), Some(330)); // Rounded to 15-min boundary

        // Invalid offset
        let ts = ExFatTimestamp::from_components(2024, 1, 1, 0, 0, 0, 0, None);
        assert!(!ts.has_valid_utc_offset());
        assert_eq!(ts.utc_offset_minutes(), None);
    }

    #[test]
    fn test_dos_epoch() {
        let ts = ExFatTimestamp::dos_epoch();
        assert_eq!(ts.year(), 1980);
        assert_eq!(ts.month(), 1);
        assert_eq!(ts.day(), 1);
        assert_eq!(ts.hour(), 0);
        assert_eq!(ts.minute(), 0);
        assert_eq!(ts.second(), 0);
    }
}
