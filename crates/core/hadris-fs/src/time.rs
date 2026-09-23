use core::fmt;

use crate::ErrorKind;

const SECS_PER_DAY: i64 = 86_400;
const NANOS_PER_SEC: u32 = 1_000_000_000;
const MAX_OFFSET_MINUTES: i16 = 24 * 60 - 1;
const MIN_YEAR: i32 = -32_768;
const MAX_YEAR: i32 = 32_767;

/// Why a date or time value was rejected.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum DateTimeError {
    /// Nanoseconds are not below one second.
    InvalidNanoseconds,
    /// The UTC offset is not within ±23:59.
    InvalidOffset,
    /// A month, day, hour, minute or second is out of range.
    InvalidCivil,
    /// The instant is outside [`DateTime::MIN`]..=[`DateTime::MAX`].
    OutOfRange,
}

impl DateTimeError {
    /// Returns the matching error kind.
    pub const fn kind(self) -> ErrorKind {
        match self {
            Self::OutOfRange => ErrorKind::LimitExceeded,
            _ => ErrorKind::InvalidInput,
        }
    }
}

impl From<DateTimeError> for ErrorKind {
    fn from(err: DateTimeError) -> Self {
        err.kind()
    }
}

impl fmt::Display for DateTimeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::InvalidNanoseconds => "nanoseconds must be below one second",
            Self::InvalidOffset => "UTC offset must be within 23:59 hours",
            Self::InvalidCivil => "civil date or time field out of range",
            Self::OutOfRange => "date and time out of supported range",
        })
    }
}

impl core::error::Error for DateTimeError {}

const fn is_leap_year(year: i64) -> bool {
    year % 4 == 0 && (year % 100 != 0 || year % 400 == 0)
}

const fn days_in_month(year: i64, month: u8) -> u8 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if is_leap_year(year) => 29,
        2 => 28,
        _ => 0,
    }
}

// Howard Hinnant, "chrono-Compatible Low-Level Date Algorithms", days_from_civil.
const fn days_from_civil(year: i64, month: u8, day: u8) -> i64 {
    let (m, d) = (month as i64, day as i64);
    let y = if m <= 2 { year - 1 } else { year };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = if m > 2 { m - 3 } else { m + 9 };
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

// Howard Hinnant, "chrono-Compatible Low-Level Date Algorithms", civil_from_days.
const fn civil_from_days(days: i64) -> (i64, u8, u8) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as u8;
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u8;
    let year = yoe + era * 400 + if month <= 2 { 1 } else { 0 };
    (year, month, day)
}

/// A proleptic Gregorian calendar date.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct CivilDate {
    year: i32,
    month: u8,
    day: u8,
}

impl CivilDate {
    /// Creates a date. Year 0 is 1 BCE. Months and days start at 1.
    pub const fn new(year: i32, month: u8, day: u8) -> Result<Self, DateTimeError> {
        if month < 1 || month > 12 || day < 1 || day > days_in_month(year as i64, month) {
            return Err(DateTimeError::InvalidCivil);
        }
        Ok(Self { year, month, day })
    }

    /// Returns the year.
    pub const fn year(&self) -> i32 {
        self.year
    }

    /// Returns the month, 1 to 12.
    pub const fn month(&self) -> u8 {
        self.month
    }

    /// Returns the day of the month, starting at 1.
    pub const fn day(&self) -> u8 {
        self.day
    }
}

/// A time of day with whole-second precision.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct CivilTime {
    hour: u8,
    minute: u8,
    second: u8,
}

impl CivilTime {
    /// Midnight.
    pub const MIDNIGHT: Self = Self {
        hour: 0,
        minute: 0,
        second: 0,
    };

    /// Creates a time. Leap seconds are not representable.
    pub const fn new(hour: u8, minute: u8, second: u8) -> Result<Self, DateTimeError> {
        if hour > 23 || minute > 59 || second > 59 {
            return Err(DateTimeError::InvalidCivil);
        }
        Ok(Self {
            hour,
            minute,
            second,
        })
    }

    /// Returns the hour, 0 to 23.
    pub const fn hour(&self) -> u8 {
        self.hour
    }

    /// Returns the minute, 0 to 59.
    pub const fn minute(&self) -> u8 {
        self.minute
    }

    /// Returns the second, 0 to 59.
    pub const fn second(&self) -> u8 {
        self.second
    }

    const fn seconds_of_day(self) -> i64 {
        self.hour as i64 * 3600 + self.minute as i64 * 60 + self.second as i64
    }
}

/// An instant with nanosecond precision and an optional UTC offset.
///
/// The instant is stored as seconds and nanoseconds since
/// 1970-01-01T00:00:00Z. The offset records the local time zone the value was
/// recorded in, when the format stores one; it does not change the instant.
/// Supported years are -32768 to 32767 in UTC.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct DateTime {
    seconds: i64,
    nanoseconds: u32,
    offset_minutes: Option<i16>,
}

impl DateTime {
    /// 1970-01-01T00:00:00Z.
    pub const UNIX_EPOCH: Self = Self {
        seconds: 0,
        nanoseconds: 0,
        offset_minutes: None,
    };

    /// The earliest supported instant, -32768-01-01T00:00:00Z.
    pub const MIN: Self = Self {
        seconds: days_from_civil(MIN_YEAR as i64, 1, 1) * SECS_PER_DAY,
        nanoseconds: 0,
        offset_minutes: None,
    };

    /// The latest supported instant, 32767-12-31T23:59:59.999999999Z.
    pub const MAX: Self = Self {
        seconds: days_from_civil(MAX_YEAR as i64, 12, 31) * SECS_PER_DAY + SECS_PER_DAY - 1,
        nanoseconds: NANOS_PER_SEC - 1,
        offset_minutes: None,
    };

    /// Creates an instant from seconds and nanoseconds since the Unix epoch.
    pub const fn new(seconds: i64, nanoseconds: u32) -> Result<Self, DateTimeError> {
        if nanoseconds >= NANOS_PER_SEC {
            return Err(DateTimeError::InvalidNanoseconds);
        }
        if seconds < Self::MIN.seconds || seconds > Self::MAX.seconds {
            return Err(DateTimeError::OutOfRange);
        }
        Ok(Self {
            seconds,
            nanoseconds,
            offset_minutes: None,
        })
    }

    /// Creates an instant from whole seconds since the Unix epoch.
    pub const fn from_unix_seconds(seconds: i64) -> Result<Self, DateTimeError> {
        Self::new(seconds, 0)
    }

    /// Creates an instant from a civil date and time.
    ///
    /// The civil fields are local time in `offset_minutes` east of UTC, or UTC
    /// when the offset is `None`.
    pub const fn from_civil(
        date: CivilDate,
        time: CivilTime,
        offset_minutes: Option<i16>,
    ) -> Result<Self, DateTimeError> {
        let offset = match check_offset(offset_minutes) {
            Ok(offset) => offset,
            Err(err) => return Err(err),
        };
        let local = days_from_civil(date.year as i64, date.month, date.day) * SECS_PER_DAY
            + time.seconds_of_day();
        let seconds = local - offset as i64 * 60;
        match Self::new(seconds, 0) {
            Ok(value) => Ok(Self {
                offset_minutes,
                ..value
            }),
            Err(err) => Err(err),
        }
    }

    /// Returns the civil date and time, in local time when an offset is set.
    pub const fn to_civil(&self) -> (CivilDate, CivilTime) {
        let offset = match self.offset_minutes {
            Some(offset) => offset as i64,
            None => 0,
        };
        let local = self.seconds + offset * 60;
        let days = local.div_euclid(SECS_PER_DAY);
        let secs = local.rem_euclid(SECS_PER_DAY);
        let (year, month, day) = civil_from_days(days);
        let date = CivilDate {
            year: year as i32,
            month,
            day,
        };
        let time = CivilTime {
            hour: (secs / 3600) as u8,
            minute: (secs / 60 % 60) as u8,
            second: (secs % 60) as u8,
        };
        (date, time)
    }

    /// Returns the civil date and time in UTC, ignoring any offset.
    pub const fn to_civil_utc(&self) -> (CivilDate, CivilTime) {
        Self {
            offset_minutes: None,
            ..*self
        }
        .to_civil()
    }

    /// Returns whole seconds since the Unix epoch.
    pub const fn unix_seconds(&self) -> i64 {
        self.seconds
    }

    /// Returns the sub-second nanoseconds.
    pub const fn nanoseconds(&self) -> u32 {
        self.nanoseconds
    }

    /// Returns the recorded UTC offset in minutes east of UTC.
    pub const fn utc_offset_minutes(&self) -> Option<i16> {
        self.offset_minutes
    }

    /// Replaces the sub-second nanoseconds.
    pub const fn with_nanoseconds(self, nanoseconds: u32) -> Result<Self, DateTimeError> {
        if nanoseconds >= NANOS_PER_SEC {
            return Err(DateTimeError::InvalidNanoseconds);
        }
        Ok(Self {
            nanoseconds,
            ..self
        })
    }

    /// Replaces the recorded UTC offset. The instant does not change.
    pub const fn with_utc_offset_minutes(
        self,
        offset_minutes: Option<i16>,
    ) -> Result<Self, DateTimeError> {
        match check_offset(offset_minutes) {
            Ok(_) => Ok(Self {
                offset_minutes,
                ..self
            }),
            Err(err) => Err(err),
        }
    }
}

const fn check_offset(offset: Option<i16>) -> Result<i16, DateTimeError> {
    match offset {
        None => Ok(0),
        Some(minutes) if minutes >= -MAX_OFFSET_MINUTES && minutes <= MAX_OFFSET_MINUTES => {
            Ok(minutes)
        }
        Some(_) => Err(DateTimeError::InvalidOffset),
    }
}

/// Creation, modification, access and change times of a node.
///
/// Every field is optional: formats leave out what they do not store, and
/// `None` in a change request means "leave unchanged".
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct FileTimes {
    created: Option<DateTime>,
    modified: Option<DateTime>,
    accessed: Option<DateTime>,
    changed: Option<DateTime>,
}

impl FileTimes {
    /// Creates a value with every time unset.
    pub const fn new() -> Self {
        Self {
            created: None,
            modified: None,
            accessed: None,
            changed: None,
        }
    }

    /// Returns the creation (birth) time.
    pub const fn created(&self) -> Option<DateTime> {
        self.created
    }

    /// Returns the last content modification time.
    pub const fn modified(&self) -> Option<DateTime> {
        self.modified
    }

    /// Returns the last access time.
    pub const fn accessed(&self) -> Option<DateTime> {
        self.accessed
    }

    /// Returns the last metadata change time.
    pub const fn changed(&self) -> Option<DateTime> {
        self.changed
    }

    /// Sets the creation time.
    pub fn with_created(self, time: impl Into<Option<DateTime>>) -> Self {
        Self {
            created: time.into(),
            ..self
        }
    }

    /// Sets the modification time.
    pub fn with_modified(self, time: impl Into<Option<DateTime>>) -> Self {
        Self {
            modified: time.into(),
            ..self
        }
    }

    /// Sets the access time.
    pub fn with_accessed(self, time: impl Into<Option<DateTime>>) -> Self {
        Self {
            accessed: time.into(),
            ..self
        }
    }

    /// Sets the metadata change time.
    pub fn with_changed(self, time: impl Into<Option<DateTime>>) -> Self {
        Self {
            changed: time.into(),
            ..self
        }
    }

    /// Returns whether no time is set.
    pub const fn is_empty(&self) -> bool {
        self.created.is_none()
            && self.modified.is_none()
            && self.accessed.is_none()
            && self.changed.is_none()
    }
}

/// A source of the current time for timestamps a filesystem writes.
pub trait Clock {
    /// Returns the current time.
    fn now(&self) -> DateTime;
}

impl<C: Clock + ?Sized> Clock for &C {
    fn now(&self) -> DateTime {
        (**self).now()
    }
}

/// A clock that always returns 1980-01-01T00:00:00Z.
///
/// This is the earliest time FAT can store, so every format can represent it.
/// Use it where no real clock exists and reproducible output is wanted.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct NoClock;

impl NoClock {
    /// The time this clock returns.
    pub const TIME: DateTime = DateTime {
        seconds: 315_532_800,
        nanoseconds: 0,
        offset_minutes: None,
    };
}

impl Clock for NoClock {
    fn now(&self) -> DateTime {
        Self::TIME
    }
}

/// A clock that reads the host system time in UTC.
///
/// Times outside the supported range are clamped to [`DateTime::MIN`] or
/// [`DateTime::MAX`].
#[cfg(feature = "std")]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct SystemClock;

#[cfg(feature = "std")]
impl Clock for SystemClock {
    fn now(&self) -> DateTime {
        use std::time::{SystemTime, UNIX_EPOCH};

        let (seconds, nanoseconds) = match SystemTime::now().duration_since(UNIX_EPOCH) {
            Ok(after) => (
                i64::try_from(after.as_secs()).unwrap_or(i64::MAX),
                after.subsec_nanos(),
            ),
            Err(err) => {
                let before = err.duration();
                let secs = i64::try_from(before.as_secs()).unwrap_or(i64::MAX);
                match before.subsec_nanos() {
                    0 => (secs.saturating_neg(), 0),
                    n => (secs.saturating_neg().saturating_sub(1), NANOS_PER_SEC - n),
                }
            }
        };
        if seconds < DateTime::MIN.seconds {
            DateTime::MIN
        } else if seconds > DateTime::MAX.seconds {
            DateTime::MAX
        } else {
            DateTime {
                seconds,
                nanoseconds,
                offset_minutes: None,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn civil(y: i32, mo: u8, d: u8, h: u8, mi: u8, s: u8) -> (CivilDate, CivilTime) {
        (
            CivilDate::new(y, mo, d).unwrap(),
            CivilTime::new(h, mi, s).unwrap(),
        )
    }

    fn round_trip(y: i32, mo: u8, d: u8, h: u8, mi: u8, s: u8, expected: i64) {
        let (date, time) = civil(y, mo, d, h, mi, s);
        let dt = DateTime::from_civil(date, time, None).unwrap();
        assert_eq!(dt.unix_seconds(), expected, "{y}-{mo}-{d}");
        assert_eq!(dt.to_civil(), (date, time));
    }

    #[test]
    fn civil_round_trips() {
        round_trip(1970, 1, 1, 0, 0, 0, 0);
        round_trip(1980, 1, 1, 0, 0, 0, 315_532_800);
        round_trip(2038, 1, 19, 3, 14, 7, 2_147_483_647);
        round_trip(2038, 1, 19, 3, 14, 8, 2_147_483_648);
        round_trip(2107, 12, 31, 23, 59, 58, 4_354_819_198);
        round_trip(2000, 2, 29, 12, 0, 0, 951_825_600);
        round_trip(2024, 2, 29, 0, 0, 0, 1_709_164_800);
        round_trip(1969, 12, 31, 23, 59, 59, -1);
        round_trip(1601, 1, 1, 0, 0, 0, -11_644_473_600);
        round_trip(0, 3, 1, 0, 0, 0, -62_162_035_200);
        round_trip(0, 2, 29, 0, 0, 0, -62_162_121_600);
        round_trip(-1, 12, 31, 23, 59, 59, -62_167_219_201);
        round_trip(-4713, 11, 24, 12, 0, 0, -210_866_760_000);
    }

    #[test]
    fn every_day_round_trips() {
        let start = days_from_civil(1899, 1, 1);
        let end = days_from_civil(2401, 1, 1);
        let mut expected = (1899, 1, 1);
        for days in start..end {
            let (y, m, d) = civil_from_days(days);
            assert_eq!((y, m, d), expected);
            assert_eq!(days_from_civil(y, m, d), days);
            expected = if d < days_in_month(y, m) {
                (y, m, d + 1)
            } else if m < 12 {
                (y, m + 1, 1)
            } else {
                (y + 1, 1, 1)
            };
        }
    }

    #[test]
    fn leap_days() {
        assert!(CivilDate::new(2000, 2, 29).is_ok());
        assert!(CivilDate::new(2024, 2, 29).is_ok());
        assert!(CivilDate::new(0, 2, 29).is_ok());
        assert!(CivilDate::new(-4, 2, 29).is_ok());
        assert_eq!(
            CivilDate::new(1900, 2, 29),
            Err(DateTimeError::InvalidCivil)
        );
        assert_eq!(
            CivilDate::new(2100, 2, 29),
            Err(DateTimeError::InvalidCivil)
        );
        assert_eq!(CivilDate::new(-1, 2, 29), Err(DateTimeError::InvalidCivil));
    }

    #[test]
    fn rejects_invalid_fields() {
        assert!(CivilDate::new(2020, 0, 1).is_err());
        assert!(CivilDate::new(2020, 13, 1).is_err());
        assert!(CivilDate::new(2020, 4, 31).is_err());
        assert!(CivilDate::new(2020, 1, 0).is_err());
        assert!(CivilTime::new(24, 0, 0).is_err());
        assert!(CivilTime::new(0, 60, 0).is_err());
        assert!(CivilTime::new(0, 0, 60).is_err());
        assert_eq!(
            DateTime::new(0, 1_000_000_000),
            Err(DateTimeError::InvalidNanoseconds)
        );
        assert_eq!(DateTime::new(i64::MAX, 0), Err(DateTimeError::OutOfRange));
        assert_eq!(DateTime::new(i64::MIN, 0), Err(DateTimeError::OutOfRange));
        assert_eq!(
            DateTime::UNIX_EPOCH.with_utc_offset_minutes(Some(1440)),
            Err(DateTimeError::InvalidOffset)
        );
        let (date, time) = civil(i32::MAX, 1, 1, 0, 0, 0);
        assert_eq!(
            DateTime::from_civil(date, time, None),
            Err(DateTimeError::OutOfRange)
        );
        let (date, time) = civil(i32::MIN, 1, 1, 0, 0, 0);
        assert_eq!(
            DateTime::from_civil(date, time, None),
            Err(DateTimeError::OutOfRange)
        );
    }

    #[test]
    fn range_limits() {
        assert_eq!(DateTime::MIN.to_civil(), civil(-32768, 1, 1, 0, 0, 0));
        assert_eq!(DateTime::MAX.to_civil(), civil(32767, 12, 31, 23, 59, 59));
        assert_eq!(
            DateTime::new(DateTime::MAX.unix_seconds(), 999_999_999),
            Ok(DateTime::MAX)
        );
        let late = DateTime::MAX.with_utc_offset_minutes(Some(60)).unwrap();
        assert_eq!(late.to_civil().0.year(), 32768);
    }

    #[test]
    fn offsets_shift_civil_time_only() {
        let (date, time) = civil(2024, 1, 1, 9, 30, 0);
        let tokyo = DateTime::from_civil(date, time, Some(9 * 60)).unwrap();
        assert_eq!(tokyo.unix_seconds(), 1_704_069_000);
        assert_eq!(tokyo.to_civil(), (date, time));
        assert_eq!(tokyo.to_civil_utc(), civil(2024, 1, 1, 0, 30, 0));
        let (date, time) = civil(2023, 12, 31, 19, 30, 0);
        let new_york = DateTime::from_civil(date, time, Some(-5 * 60)).unwrap();
        assert_eq!(new_york.unix_seconds(), tokyo.unix_seconds());
        assert_eq!(new_york.utc_offset_minutes(), Some(-300));
    }

    #[test]
    fn with_setters() {
        let dt = DateTime::from_unix_seconds(10)
            .unwrap()
            .with_nanoseconds(5)
            .unwrap();
        assert_eq!((dt.unix_seconds(), dt.nanoseconds()), (10, 5));
        assert!(dt.with_nanoseconds(NANOS_PER_SEC).is_err());
        let times = FileTimes::new().with_modified(dt).with_accessed(None);
        assert_eq!(times.modified(), Some(dt));
        assert_eq!(times.accessed(), None);
        assert!(!times.is_empty());
        assert!(FileTimes::default().is_empty());
    }

    #[test]
    fn no_clock_is_fat_epoch() {
        let (date, time) = NoClock.now().to_civil();
        assert_eq!((date, time), civil(1980, 1, 1, 0, 0, 0));
        assert_eq!(<&NoClock as Clock>::now(&&NoClock), NoClock::TIME);
    }

    #[cfg(feature = "std")]
    #[test]
    #[cfg_attr(miri, ignore)]
    fn system_clock_is_after_2020() {
        assert!(SystemClock.now().unix_seconds() > 1_577_836_800);
    }
}
