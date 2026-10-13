use crate::{FatKind, Slot, date, exfat, lfn, name, short_name};

#[kani::proof]
#[kani::unwind(12)]
fn lfn_checksum_matches_specification() {
    let short: [u8; 11] = kani::any();
    let mut expected = 0u16;
    for byte in short {
        expected = (((expected & 1) << 7) + (expected >> 1) + u16::from(byte)) & 0xff;
    }
    assert_eq!(lfn::checksum(&short), expected as u8);
}

#[kani::proof]
#[kani::unwind(33)]
fn lfn_fields_preserve_code_units_and_terminator() {
    let units: [u16; 13] = kani::any();
    let (first, second, third) = lfn::pack(&units);
    for (index, unit) in units.iter().enumerate() {
        let (bytes, offset): (&[u8], usize) = if index < 5 {
            (&first, index * 2)
        } else if index < 11 {
            (&second, (index - 5) * 2)
        } else {
            (&third, (index - 11) * 2)
        };
        assert_eq!(bytes[offset], (*unit & 0xff) as u8);
        assert_eq!(bytes[offset + 1], (*unit >> 8) as u8);
    }
    let (decoded, length) = lfn::unpack(&first, &second, &third);
    assert_eq!(decoded, units);
    let mut expected = 13;
    for (index, unit) in units.iter().enumerate() {
        if *unit == 0 || *unit == 0xffff {
            expected = index;
            break;
        }
    }
    assert_eq!(length, expected);
}

#[kani::proof]
fn directory_slots_match_disk_markers() {
    let raw: [u8; 32] = kani::any();
    match Slot::parse(&raw) {
        Slot::End => assert_eq!(raw[0], 0),
        Slot::Free => assert_eq!(raw[0], 0xe5),
        Slot::Long(_) => {
            assert!(raw[0] != 0 && raw[0] != 0xe5);
            assert_eq!(raw[11] & 0x3f, 0x0f);
        }
        Slot::Short(_) => {
            assert!(raw[0] != 0 && raw[0] != 0xe5);
            assert_ne!(raw[11] & 0x3f, 0x0f);
        }
    }
}

#[kani::proof]
fn short_entry_cluster_fields_match_specification() {
    let mut raw: [u8; 32] = kani::any();
    raw[0] = b'A';
    raw[11] = 0x20;
    let Slot::Short(entry) = Slot::parse(&raw) else {
        panic!("short entry required")
    };
    let low = u32::from(raw[26]) + (u32::from(raw[27]) << 8);
    let high = u32::from(raw[20]) + (u32::from(raw[21]) << 8);
    assert_eq!(entry.first_cluster(FatKind::Fat12), low);
    assert_eq!(entry.first_cluster(FatKind::Fat16), low);
    assert_eq!(
        entry.first_cluster(FatKind::Fat32),
        (low + (high << 16)) & 0x0fff_ffff
    );
}

#[kani::proof]
#[kani::unwind(12)]
fn short_name_escape_preserves_other_bytes() {
    let name: [u8; 11] = kani::any();
    let mut stored = name;
    short_name::to_disk(&mut stored);
    assert_eq!(stored[0], if name[0] == 0xe5 { 5 } else { name[0] });
    assert_eq!(stored[1..], name[1..]);
    short_name::from_disk(&mut stored);
    assert_eq!(stored[0], if name[0] == 5 { 0xe5 } else { name[0] });
    assert_eq!(stored[1..], name[1..]);
}

#[kani::proof]
fn ascii_case_fold_matches_specification() {
    let unit: u16 = kani::any();
    let expected = if (97..=122).contains(&unit) {
        unit - 32
    } else {
        unit
    };
    assert_eq!(name::fold_ascii(unit), expected);
    assert_eq!(name::fold_ascii(name::fold_ascii(unit)), expected);
}

#[kani::proof]
fn exfat_name_units_reject_forbidden_characters() {
    let unit: u16 = kani::any();
    let forbidden = unit < 32 || matches!(unit, 34 | 42 | 47 | 58 | 60 | 62 | 63 | 92 | 124);
    assert_eq!(exfat::valid_unit(unit), !forbidden);
}

#[kani::proof]
fn fat_timestamp_packing_matches_bit_fields() {
    let year: u16 = kani::any();
    let month: u8 = kani::any();
    let day: u8 = kani::any();
    let hour: u8 = kani::any();
    let minute: u8 = kani::any();
    let second: u8 = kani::any();
    let (date, time) = date::pack(year, month, day, hour, minute, second);
    assert_eq!(date >> 9, year.clamp(1980, 2107) - 1980);
    assert_eq!((date >> 5) & 15, u16::from(month & 15));
    assert_eq!(date & 31, u16::from(day & 31));
    assert_eq!(time >> 11, u16::from(hour & 31));
    assert_eq!((time >> 5) & 63, u16::from(minute & 63));
    assert_eq!(time & 31, u16::from((second / 2) & 31));
}
