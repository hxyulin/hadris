//! Multi-extent file conformance and reader coverage.

use std::collections::BTreeMap;

use hadris_tests::harness::tree::EntryData;
use hadris_tests::iso::model::IsoState;
use hadris_tests::iso::{SECTOR_SIZE, VOLUME_ID, hadris, spec};

const FILE_SIZE: usize = 4 * SECTOR_SIZE;
const MULTI_EXTENT: u8 = 0x80;

struct Fixture {
    bytes: Vec<u8>,
    expected: IsoState,
    records: [usize; 3],
}

fn write_both_u32(bytes: &mut [u8], offset: usize, value: u32) {
    bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
    bytes[offset + 4..offset + 8].copy_from_slice(&value.to_be_bytes());
}

fn fixture() -> Fixture {
    let contents = (0..FILE_SIZE).map(|index| (index % 251) as u8).collect();
    let expected = IsoState {
        volume_id: VOLUME_ID.to_string(),
        entries: BTreeMap::from([("/LARGE.BIN".to_string(), EntryData::File(contents))]),
    };
    let mut bytes = hadris::write(&expected).unwrap();
    let root_record = 16 * SECTOR_SIZE + 156;
    let root_extent =
        u32::from_le_bytes(bytes[root_record + 2..root_record + 6].try_into().unwrap()) as usize;
    let mut file_record = root_extent * SECTOR_SIZE;
    file_record += bytes[file_record] as usize;
    file_record += bytes[file_record] as usize;
    let record_len = bytes[file_record] as usize;
    let first_extent =
        u32::from_le_bytes(bytes[file_record + 2..file_record + 6].try_into().unwrap());
    let template = bytes[file_record..file_record + record_len].to_vec();
    let records = [
        file_record,
        file_record + record_len,
        file_record + record_len * 2,
    ];
    let sections = [
        (first_extent, SECTOR_SIZE as u32, true),
        (first_extent + 1, (2 * SECTOR_SIZE) as u32, true),
        (first_extent + 3, SECTOR_SIZE as u32, false),
    ];
    for (record, (extent, length, has_more)) in records.iter().zip(sections) {
        bytes[*record..*record + record_len].copy_from_slice(&template);
        write_both_u32(&mut bytes, *record + 2, extent);
        write_both_u32(&mut bytes, *record + 10, length);
        if has_more {
            bytes[*record + 25] |= MULTI_EXTENT;
        } else {
            bytes[*record + 25] &= !MULTI_EXTENT;
        }
    }
    Fixture {
        bytes,
        expected,
        records,
    }
}

#[test]
fn three_section_file_matches_oracle_and_hadris_reader() {
    let fixture = fixture();
    hadris::verify_image(
        "three-section multi-extent fixture",
        fixture.bytes,
        &fixture.expected,
    )
    .unwrap();
}

#[test]
fn unaligned_non_final_section_matches_oracle_and_hadris_reader() {
    let mut fixture = fixture();
    write_both_u32(
        &mut fixture.bytes,
        fixture.records[0] + 10,
        (SECTOR_SIZE - 1) as u32,
    );
    let EntryData::File(contents) = fixture.expected.entries.get_mut("/LARGE.BIN").unwrap() else {
        unreachable!();
    };
    contents.remove(SECTOR_SIZE - 1);
    hadris::verify_image(
        "unaligned non-final multi-extent section",
        fixture.bytes,
        &fixture.expected,
    )
    .unwrap();
}

#[test]
fn oracle_accepts_per_section_protection_flag() {
    let mut fixture = fixture();
    fixture.bytes[fixture.records[1] + 25] |= 0x10;
    assert_eq!(spec::snapshot(&fixture.bytes).unwrap(), fixture.expected);
}

#[test]
fn oracle_rejects_invalid_multi_extent_chains() {
    type Corrupt = fn(&mut Fixture);

    let cases: [(&str, Corrupt); 2] = [
        ("truncated chain", |fixture| {
            fixture.bytes[fixture.records[2]] = 0;
        }),
        ("changed identifier", |fixture| {
            fixture.bytes[fixture.records[1] + 33] = b'X';
        }),
    ];
    for (name, corrupt) in cases {
        let mut fixture = fixture();
        corrupt(&mut fixture);
        assert!(
            spec::snapshot(&fixture.bytes).is_err(),
            "oracle accepted {name}"
        );
    }
}
