//! FAT directory-entry timestamps.
//!
//! A date packs `(year - 1980) << 9 | month << 5 | day`, a time packs
//! `hour << 11 | minute << 5 | second / 2`, and the optional creation field
//! counts 10 ms units from 0 to 199. FAT stores local time with no zone.

use hadris_fs::{CivilDate, CivilTime, DateTime};

const NANOS_PER_TENTH: u32 = 10_000_000;

/// The earliest encodable instant, 1980-01-01 00:00:00.
pub(crate) const MIN: (u16, u16, u8) = ((1 << 5) | 1, 0, 0);
/// The latest encodable instant, 2107-12-31 23:59:59.99.
pub(crate) const MAX: (u16, u16, u8) = (
    (127 << 9) | (12 << 5) | 31,
    (23 << 11) | (59 << 5) | 29,
    199,
);

/// Packs calendar fields into `(date, time)`. Years outside 1980 to 2107
/// are clamped; other fields are masked, not validated.
pub(crate) const fn pack(
    year: u16,
    month: u8,
    day: u8,
    hour: u8,
    minute: u8,
    second: u8,
) -> (u16, u16) {
    let year_offset = if year < 1980 {
        0
    } else if year - 1980 > 127 {
        127
    } else {
        year - 1980
    };
    let date = (year_offset << 9) | ((month as u16 & 0x0F) << 5) | (day as u16 & 0x1F);
    let time =
        ((hour as u16 & 0x1F) << 11) | ((minute as u16 & 0x3F) << 5) | ((second as u16 / 2) & 0x1F);
    (date, time)
}

/// Decodes a stored timestamp. `None` when the date or time fields are out
/// of range, which includes the all-zero "not set" value. A `tenths` value
/// above 199 is ignored.
pub(crate) fn decode(date: u16, time: u16, tenths: u8) -> Option<DateTime> {
    let civil_date = CivilDate::new(
        1980 + (date >> 9) as i32,
        ((date >> 5) & 0x0F) as u8,
        (date & 0x1F) as u8,
    )
    .ok()?;
    let civil_time = CivilTime::new(
        (time >> 11) as u8,
        ((time >> 5) & 0x3F) as u8,
        ((time & 0x1F) * 2) as u8,
    )
    .ok()?;
    let base = DateTime::from_civil(civil_date, civil_time, None).ok()?;
    let tenths = if tenths > 199 { 0 } else { tenths };
    DateTime::new(
        base.unix_seconds() + (tenths / 100) as i64,
        (tenths % 100) as u32 * NANOS_PER_TENTH,
    )
    .ok()
}

/// Encodes `time` as `(date, time, tenths)` in its recorded local time,
/// clamped to [`MIN`] and [`MAX`].
pub(crate) fn encode(time: DateTime) -> (u16, u16, u8) {
    let (date, clock) = time.to_civil();
    if date.year() < 1980 {
        return MIN;
    }
    if date.year() > 2107 {
        return MAX;
    }
    let (packed_date, packed_time) = pack(
        date.year() as u16,
        date.month(),
        date.day(),
        clock.hour(),
        clock.minute(),
        clock.second(),
    );
    let tenths = (clock.second() % 2) * 100 + (time.nanoseconds() / NANOS_PER_TENTH) as u8;
    (packed_date, packed_time, tenths)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(year: i32, month: u8, day: u8, hour: u8, minute: u8, second: u8) -> DateTime {
        DateTime::from_civil(
            CivilDate::new(year, month, day).unwrap(),
            CivilTime::new(hour, minute, second).unwrap(),
            None,
        )
        .unwrap()
    }

    #[test]
    fn pack_clamps_years() {
        assert_eq!(pack(1979, 1, 1, 0, 0, 0), (MIN.0, 0));
        assert_eq!(pack(2200, 1, 1, 0, 0, 0).0 >> 9, 127);
        assert_eq!(
            pack(2024, 2, 29, 13, 45, 31),
            ((44 << 9) | (2 << 5) | 29, (13 << 11) | (45 << 5) | 15)
        );
    }

    #[test]
    fn round_trips_with_two_second_resolution() {
        let time = at(2024, 2, 29, 13, 45, 31)
            .with_nanoseconds(560_000_000)
            .unwrap();
        let (date, clock, tenths) = encode(time);
        assert_eq!(tenths, 156);
        assert_eq!(decode(date, clock, tenths), Some(time));
        assert_eq!(decode(date, clock, 0), Some(at(2024, 2, 29, 13, 45, 30)));
    }

    #[test]
    fn decode_rejects_invalid_fields() {
        assert_eq!(decode(0, 0, 0), None);
        assert_eq!(decode((1 << 5) | 1, 24 << 11, 0), None);
        assert_eq!(decode((2 << 5) | 30, 0, 0), None);
        assert_eq!(decode(MIN.0, MIN.1, 250), Some(at(1980, 1, 1, 0, 0, 0)));
    }

    #[test]
    fn encode_clamps_to_fat_range() {
        assert_eq!(encode(DateTime::UNIX_EPOCH), MIN);
        assert_eq!(encode(at(2200, 6, 1, 0, 0, 0)), MAX);
        assert_eq!(
            decode(MAX.0, MAX.1, MAX.2).unwrap().unix_seconds(),
            at(2107, 12, 31, 23, 59, 59).unix_seconds()
        );
    }

    #[test]
    fn encode_uses_recorded_local_time() {
        let local = at(2024, 1, 1, 10, 0, 0)
            .with_utc_offset_minutes(Some(120))
            .unwrap();
        let (_, clock, _) = encode(local);
        assert_eq!(clock >> 11, 12);
    }
}
