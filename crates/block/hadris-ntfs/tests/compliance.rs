use hadris_ntfs::NtfsError;
use hadris_ntfs::attr::{
    AttrIter, DataRun, apply_fixups, decode_data_runs, decode_record_size, decode_utf16le,
};
use hadris_ntfs::sync::NtfsFs;

fn boot_sector() -> Vec<u8> {
    let mut boot = vec![0_u8; 512];
    boot[3..11].copy_from_slice(b"NTFS    ");
    boot[11..13].copy_from_slice(&512_u16.to_le_bytes());
    boot[13] = 8;
    boot[40..48].copy_from_slice(&2048_u64.to_le_bytes());
    boot[48..56].copy_from_slice(&4_u64.to_le_bytes());
    boot[64] = (-10_i8) as u8;
    boot[68] = (-12_i8) as u8;
    boot[510..512].copy_from_slice(&0xAA55_u16.to_le_bytes());
    boot
}

#[test]
fn record_size_encoding_rejects_zero_and_unrepresentable_exponents() {
    assert!(matches!(
        decode_record_size(0, 4096),
        Err(NtfsError::InvalidRecordSize)
    ));
    assert!(matches!(
        decode_record_size((-128_i8) as u8, 4096),
        Err(NtfsError::InvalidRecordSize)
    ));
}

#[test]
fn open_rejects_invalid_sector_size() {
    let mut boot = boot_sector();
    boot[11..13].copy_from_slice(&1000_u16.to_le_bytes());

    assert!(matches!(
        NtfsFs::open(std::io::Cursor::new(boot)),
        Err(NtfsError::InvalidSectorSize { found: 1000 })
    ));
}

#[test]
fn open_rejects_invalid_cluster_factor() {
    let mut boot = boot_sector();
    boot[13] = 3;

    assert!(matches!(
        NtfsFs::open(std::io::Cursor::new(boot)),
        Err(NtfsError::InvalidSectorsPerCluster { found: 3 })
    ));
}

#[test]
fn open_rejects_mft_outside_volume() {
    let mut boot = boot_sector();
    boot[48..56].copy_from_slice(&256_u64.to_le_bytes());

    assert!(matches!(
        NtfsFs::open(std::io::Cursor::new(boot)),
        Err(NtfsError::InvalidVolumeGeometry)
    ));
}

#[test]
fn fixups_restore_each_sector_trailer() {
    let mut record = vec![0_u8; 1024];
    record[4..6].copy_from_slice(&0x30_u16.to_le_bytes());
    record[6..8].copy_from_slice(&3_u16.to_le_bytes());
    record[0x30..0x32].copy_from_slice(&0xA55A_u16.to_le_bytes());
    record[0x32..0x34].copy_from_slice(&0x2211_u16.to_le_bytes());
    record[0x34..0x36].copy_from_slice(&0x4433_u16.to_le_bytes());
    record[510..512].copy_from_slice(&0xA55A_u16.to_le_bytes());
    record[1022..1024].copy_from_slice(&0xA55A_u16.to_le_bytes());

    apply_fixups(&mut record, 512).unwrap();

    assert_eq!(&record[510..512], &0x2211_u16.to_le_bytes());
    assert_eq!(&record[1022..1024], &0x4433_u16.to_le_bytes());
}

#[test]
fn fixups_reject_a_count_that_does_not_cover_every_sector() {
    let mut record = vec![0_u8; 1024];
    record[4..6].copy_from_slice(&0x30_u16.to_le_bytes());
    record[6..8].copy_from_slice(&2_u16.to_le_bytes());

    assert!(matches!(
        apply_fixups(&mut record, 512),
        Err(NtfsError::InvalidFixup)
    ));
}

#[test]
fn data_runs_decode_relative_and_sparse_extents() {
    let runs = decode_data_runs(&[
        0x11, 0x03, 0x20, // Three clusters at LCN 0x20.
        0x01, 0x02, // Two sparse clusters.
        0x11, 0x01, 0xFE, // One cluster at LCN 0x1e (delta -2).
        0x00,
    ])
    .unwrap();

    assert_eq!(
        runs,
        [
            DataRun {
                lcn: 0x20,
                length: 3,
            },
            DataRun { lcn: -1, length: 2 },
            DataRun {
                lcn: 0x1e,
                length: 1,
            },
        ]
    );
}

#[test]
fn data_runs_reject_malformed_encodings() {
    for invalid in [
        &[0x19, 1, 2, 3, 4, 5, 6, 7, 8, 9, 0][..],
        &[0x11, 0, 1, 0][..],
        &[0x11, 1, 1][..],
        &[0x11, 1, 0xFF, 0][..],
    ] {
        assert!(matches!(
            decode_data_runs(invalid),
            Err(NtfsError::InvalidDataRun)
        ));
    }
}

#[test]
fn attributes_are_bounded_by_the_file_record_used_size() {
    let mut record = vec![0_u8; 1024];
    record[0x14..0x16].copy_from_slice(&0x30_u16.to_le_bytes());
    record[0x18..0x1C].copy_from_slice(&0x38_u32.to_le_bytes());
    record[0x30..0x34].copy_from_slice(&0x80_u32.to_le_bytes());
    record[0x34..0x38].copy_from_slice(&8_u32.to_le_bytes());

    let mut attrs = AttrIter::new(&record).unwrap();
    assert!(matches!(
        attrs.next(),
        Some(Err(NtfsError::InvalidAttribute))
    ));
    assert!(attrs.next().is_none());
}

#[test]
fn attributes_reject_used_sizes_beyond_the_record() {
    let mut record = vec![0_u8; 1024];
    record[0x14..0x16].copy_from_slice(&0x30_u16.to_le_bytes());
    record[0x18..0x1C].copy_from_slice(&2048_u32.to_le_bytes());

    assert!(matches!(
        AttrIter::new(&record),
        Err(NtfsError::InvalidAttribute)
    ));
}

#[test]
fn filenames_decode_utf16_surrogate_pairs() {
    assert_eq!(
        decode_utf16le(&[0x3E, 0xD8, 0x80, 0xDD]).unwrap(),
        "\u{1F980}"
    );
    assert!(matches!(
        decode_utf16le(&[0x3E, 0xD8]),
        Err(NtfsError::InvalidFileName)
    ));
    assert!(matches!(
        decode_utf16le(&[0x41]),
        Err(NtfsError::InvalidFileName)
    ));
}
