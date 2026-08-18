use hadris_ntfs::NtfsError;
use hadris_ntfs::attr::{
    ATTR_DATA, ATTR_END, AttrIter, DataRun, apply_fixups, decode_data_runs, decode_record_size,
    decode_utf16le,
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
fn open_rejects_record_sizes_beyond_the_image() {
    // Exponent encoding -38 = 2^38 bytes: record buffers are sized from these
    // fields, so a record that cannot fit in the data source must be rejected
    // before any allocation (fuzz-found OOM).
    let mut boot = boot_sector();
    boot[64] = (-38_i8) as u8;

    assert!(matches!(
        NtfsFs::open(std::io::Cursor::new(boot)),
        Err(NtfsError::InvalidVolumeGeometry)
    ));

    // Same for the index record size; pad the image past the $MFT placement
    // so the index record check is reached.
    let mut boot = boot_sector();
    boot[68] = (-38_i8) as u8;
    let mut image = boot;
    image.resize(4 * 4096 + 1024, 0);

    assert!(matches!(
        NtfsFs::open(std::io::Cursor::new(image)),
        Err(NtfsError::InvalidRecordSize)
    ));
}

/// Minimal image: one cluster per sector, $MFT covering records 0..31 at
/// LCN 4, and a $UpCase (record 10) stream with the given sizes.
fn image_with_upcase_stream(data_size: u64, initialized_size: u64, runs: [u8; 8]) -> Vec<u8> {
    let mut image = boot_sector();
    image[13] = 1; // sectors per cluster
    image[40..48].copy_from_slice(&512_u64.to_le_bytes());
    image.resize(16384, 0);

    let mut write_record = |offset: usize, data_size: u64, initialized: u64, runs: [u8; 8]| {
        let record = &mut image[offset..offset + 1024];
        record[0..4].copy_from_slice(b"FILE");
        record[4..6].copy_from_slice(&0x30_u16.to_le_bytes()); // USA offset
        record[6..8].copy_from_slice(&3_u16.to_le_bytes()); // USA count
        record[0x14..0x16].copy_from_slice(&0x38_u16.to_le_bytes()); // first attr
        record[0x16..0x18].copy_from_slice(&1_u16.to_le_bytes()); // in use
        record[0x18..0x1C].copy_from_slice(&0x80_u32.to_le_bytes()); // used size
        record[0x30..0x32].copy_from_slice(&0xAAAA_u16.to_le_bytes()); // USN
        record[510..512].copy_from_slice(&0xAAAA_u16.to_le_bytes());
        record[1022..1024].copy_from_slice(&0xAAAA_u16.to_le_bytes());

        let attr = &mut record[0x38..0x80];
        attr[0x00..0x04].copy_from_slice(&ATTR_DATA.to_le_bytes());
        attr[0x04..0x08].copy_from_slice(&0x48_u32.to_le_bytes());
        attr[0x08] = 1; // non-resident
        attr[0x18..0x20].copy_from_slice(&63_u64.to_le_bytes()); // last VCN
        attr[0x20..0x22].copy_from_slice(&0x40_u16.to_le_bytes()); // runs offset
        attr[0x28..0x30].copy_from_slice(&data_size.to_le_bytes());
        attr[0x30..0x38].copy_from_slice(&data_size.to_le_bytes());
        attr[0x38..0x40].copy_from_slice(&initialized.to_le_bytes());
        attr[0x40..0x48].copy_from_slice(&runs);
    };

    // Record 0: $MFT data, 64 clusters at LCN 4.
    write_record(2048, 32768, 32768, [0x11, 0x40, 0x04, 0x00, 0, 0, 0, 0]);
    // Record 10: $UpCase.
    write_record(12288, data_size, initialized_size, runs);
    image
}

#[test]
fn open_rejects_an_upcase_stream_larger_than_the_volume() {
    // 2^40 claimed bytes in a 256 KiB volume must fail without a matching
    // allocation.
    let image = image_with_upcase_stream(1 << 40, 0, [0x02, 0x00, 0x01, 0x00, 0, 0, 0, 0]);

    assert!(matches!(
        NtfsFs::open(std::io::Cursor::new(image)),
        Err(NtfsError::InvalidUpcaseTable)
    ));
}

#[test]
fn open_rejects_an_oversized_upcase_stream_on_a_huge_claimed_volume() {
    // The volume-capacity guard trusts the claimed total_sectors, so a boot
    // sector claiming a huge volume would let a bogus $UpCase size through to
    // read_to_vec, which then materializes the sparse zeros (fuzz-found OOM).
    // The exact-size gate rejects it before any allocation.
    let mut image = image_with_upcase_stream(1 << 30, 0, [0x02, 0x00, 0x01, 0x00, 0, 0, 0, 0]);
    image[40..48].copy_from_slice(&(1_u64 << 22).to_le_bytes()); // 2 GiB claimed volume

    assert!(matches!(
        NtfsFs::open(std::io::Cursor::new(image)),
        Err(NtfsError::InvalidUpcaseTable)
    ));
}

#[test]
fn open_reads_a_sparse_upcase_stream() {
    // 256 sparse clusters = 128 KiB of zeros: exactly the $UpCase table size.
    let image = image_with_upcase_stream(131072, 0, [0x02, 0x00, 0x01, 0x00, 0, 0, 0, 0]);

    assert!(NtfsFs::open(std::io::Cursor::new(image)).is_ok());
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

fn record_with_one_resident_attribute() -> Vec<u8> {
    let mut record = vec![0_u8; 1024];
    record[0x14..0x16].copy_from_slice(&0x30_u16.to_le_bytes());
    record[0x18..0x1C].copy_from_slice(&0x4C_u32.to_le_bytes());

    record[0x30..0x34].copy_from_slice(&ATTR_DATA.to_le_bytes());
    record[0x34..0x38].copy_from_slice(&0x18_u32.to_le_bytes());
    record[0x44..0x46].copy_from_slice(&0x18_u16.to_le_bytes());
    record
}

#[test]
fn attributes_stop_at_end_marker() {
    let mut record = record_with_one_resident_attribute();
    record[0x48..0x4C].copy_from_slice(&ATTR_END.to_le_bytes());

    let mut attrs = AttrIter::new(&record).unwrap();
    assert_eq!(attrs.next().unwrap().unwrap().attr_type, ATTR_DATA);
    assert!(attrs.next().is_none());
    assert!(attrs.next().is_none());
}

#[test]
fn attributes_reject_a_missing_end_marker() {
    let record = record_with_one_resident_attribute();

    let mut attrs = AttrIter::new(&record).unwrap();
    assert_eq!(attrs.next().unwrap().unwrap().attr_type, ATTR_DATA);
    assert!(matches!(
        attrs.next(),
        Some(Err(NtfsError::InvalidAttribute))
    ));
    assert!(attrs.next().is_none());
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
