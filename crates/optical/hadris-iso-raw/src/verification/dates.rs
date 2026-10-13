use crate::raw::{digits, parse_digits};
use crate::{DecDateTime, DirDateTime};

fn days_in_month(year: i32, month: u8) -> u8 {
    let leap = year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if leap => 29,
        2 => 28,
        _ => 0,
    }
}

fn valid_civil(year: i32, month: u8, day: u8, hour: u8, minute: u8, second: u8) -> bool {
    day >= 1 && day <= days_in_month(year, month) && hour < 24 && minute < 60 && second < 60
}

fn valid_offset(offset: i8) -> bool {
    (-95..=95).contains(&offset)
}

fn decimal(raw: &[u8]) -> Option<u32> {
    let mut value = 0;
    for digit in raw {
        if !digit.is_ascii_digit() {
            return None;
        }
        value = value * 10 + u32::from(digit - b'0');
    }
    Some(value)
}

#[kani::proof]
fn directory_dates_accept_exactly_valid_fields() {
    let raw: [u8; 7] = kani::any();
    let stored: DirDateTime = bytemuck::cast(raw);
    let offset = raw[6] as i8;
    let valid = raw != [0; 7]
        && valid_civil(
            1900 + i32::from(raw[0]),
            raw[1],
            raw[2],
            raw[3],
            raw[4],
            raw[5],
        )
        && valid_offset(offset);
    let decoded = stored.to_datetime();
    assert_eq!(decoded.is_some(), valid);
    let Some(time) = decoded else { return };
    assert_eq!(time.utc_offset_minutes(), Some(i16::from(offset) * 15));
    assert_eq!(time.nanoseconds(), 0);
    let local = DirDateTime {
        offset: 0,
        ..stored
    }
    .to_datetime()
    .unwrap();
    assert_eq!(
        time.unix_seconds(),
        local.unix_seconds() - i64::from(offset) * 900
    );
    kani::cover!(offset == 52, "largest specified offset");
}

#[kani::proof]
#[kani::unwind(8)]
fn descriptor_dates_accept_exactly_valid_fields() {
    let raw: [u8; 17] = kani::any();
    let stored: DecDateTime = bytemuck::cast(raw);
    let offset = raw[16] as i8;
    let fields =
        [0..4, 4..6, 6..8, 8..10, 10..12, 12..14, 14..16].map(|range| decimal(&raw[range]));
    let decoded = stored.to_datetime();
    let [
        Some(year),
        Some(month),
        Some(day),
        Some(hour),
        Some(minute),
        Some(second),
        Some(hundredths),
    ] = fields
    else {
        assert!(decoded.is_none());
        return;
    };
    let [month, day, hour, minute, second] =
        [month, day, hour, minute, second].map(|field| field as u8);
    let valid = year != 0
        && valid_civil(year as i32, month, day, hour, minute, second)
        && valid_offset(offset);
    assert_eq!(decoded.is_some(), valid);
    let Some(time) = decoded else { return };
    assert_eq!(time.utc_offset_minutes(), Some(i16::from(offset) * 15));
    assert_eq!(time.nanoseconds(), hundredths * 10_000_000);
    let local = DecDateTime {
        offset: 0,
        ..stored
    }
    .to_datetime()
    .unwrap();
    assert_eq!(
        time.unix_seconds(),
        local.unix_seconds() - i64::from(offset) * 900
    );
    kani::cover!(
        offset == -48 && year > 2099,
        "smallest specified offset after 2099"
    );
}

#[kani::proof]
#[kani::unwind(5)]
fn descriptor_date_digits_are_fixed_width_decimal() {
    let places: [u8; 4] = kani::any();
    kani::assume(places.iter().all(|&place| place < 10));
    let value = places
        .iter()
        .fold(0u32, |sum, &place| sum * 10 + u32::from(place));
    let ascii = places.map(|place| b'0' + place);
    assert_eq!(digits::<4>(value), ascii);
    assert_eq!(parse_digits(&ascii), Some(value));
    let low = u32::from(places[2]) * 10 + u32::from(places[3]);
    assert_eq!(digits::<2>(low), [ascii[2], ascii[3]]);
    let raw: [u8; 4] = kani::any();
    let valid = raw.iter().all(u8::is_ascii_digit);
    let number = raw.iter().fold(0u32, |sum, digit| {
        sum * 10 + u32::from(digit.wrapping_sub(b'0'))
    });
    assert_eq!(parse_digits(&raw), valid.then_some(number));
}
