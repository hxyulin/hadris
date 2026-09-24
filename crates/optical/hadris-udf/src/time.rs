//! Conversions between ECMA-167 timestamps and `hadris_fs::DateTime`.

use hadris_fs::{CivilDate, CivilTime, DateTime};

use crate::raw::Timestamp;

/// The instant a timestamp records, or `None` when it is blank or out of
/// range. Times without a zone, and agreement times, read as UTC.
pub(crate) fn to_datetime(ts: &Timestamp) -> Option<DateTime> {
    let raw = ts.type_and_zone.get();
    let kind = raw >> 12;
    let zone = ((raw & 0x0FFF) as i16) << 4 >> 4;
    let offset =
        (kind == 1 && zone != Timestamp::NO_ZONE && (-1440..=1440).contains(&zone)).then_some(zone);
    let year = i32::from(ts.year.get() as i16);
    let date = CivilDate::new(year, ts.month, ts.day).ok()?;
    let time = CivilTime::new(ts.hour, ts.minute, ts.second).ok()?;
    let micros = u32::from(ts.centiseconds.min(99)) * 10_000
        + u32::from(ts.hundreds_of_microseconds.min(99)) * 100
        + u32::from(ts.microseconds.min(99));
    DateTime::from_civil(date, time, offset.filter(|zone| zone.abs() <= 1439))
        .ok()?
        .with_nanoseconds(micros * 1000)
        .ok()
}

/// The timestamp of an instant: local time with its offset when it has
/// one, UTC with a zero offset otherwise. `None` outside years 1 to 9999.
#[cfg(feature = "alloc")]
pub(crate) fn from_datetime(time: DateTime) -> Option<Timestamp> {
    let (date, clock) = time.to_civil();
    if !(1..=9999).contains(&date.year()) {
        return None;
    }
    let offset = time.utc_offset_minutes().unwrap_or(0);
    let nanos = time.nanoseconds();
    Some(Timestamp {
        type_and_zone: crate::raw::U16Le::new(0x1000 | (offset as u16 & 0x0FFF)),
        year: crate::raw::U16Le::new(date.year() as u16),
        month: date.month(),
        day: date.day(),
        hour: clock.hour(),
        minute: clock.minute(),
        second: clock.second(),
        centiseconds: (nanos / 10_000_000) as u8,
        hundreds_of_microseconds: (nanos / 100_000 % 100) as u8,
        microseconds: (nanos / 1_000 % 100) as u8,
    })
}

#[cfg(all(test, feature = "alloc"))]
mod tests {
    use super::*;

    #[test]
    fn timestamps_round_trip() {
        let time = DateTime::new(1_700_000_000, 123_456_000).unwrap();
        let ts = from_datetime(time).unwrap();
        assert_eq!(ts.type_and_zone.get(), 0x1000);
        assert_eq!((ts.year.get(), ts.month, ts.day), (2023, 11, 14));
        assert_eq!(
            (
                ts.centiseconds,
                ts.hundreds_of_microseconds,
                ts.microseconds
            ),
            (12, 34, 56)
        );
        assert_eq!(
            to_datetime(&ts),
            Some(time.with_utc_offset_minutes(Some(0)).unwrap())
        );

        let zoned = time.with_utc_offset_minutes(Some(-330)).unwrap();
        let ts = from_datetime(zoned).unwrap();
        assert_eq!(ts.type_and_zone.get(), 0x1000 | (-330i16 as u16 & 0x0FFF));
        assert_eq!(to_datetime(&ts), Some(zoned));

        let epoch = from_datetime(hadris_fs::NoClock::TIME).unwrap();
        assert_eq!(
            (epoch.year.get(), epoch.month, epoch.day, epoch.hour),
            (1980, 1, 1, 0)
        );

        assert!(to_datetime(&Timestamp::default()).is_none());
        let far = DateTime::from_unix_seconds(400_000_000_000).unwrap();
        assert!(from_datetime(far).is_none());
    }

    #[test]
    fn unspecified_zones_read_as_utc() {
        let mut ts = from_datetime(DateTime::from_unix_seconds(0).unwrap()).unwrap();
        ts.type_and_zone = crate::raw::U16Le::new(0x1000 | (Timestamp::NO_ZONE as u16 & 0x0FFF));
        assert_eq!(to_datetime(&ts).unwrap().utc_offset_minutes(), None);
        assert_eq!(to_datetime(&ts).unwrap().unix_seconds(), 0);
    }
}
