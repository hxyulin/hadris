use std::io::Cursor;

use hadris_iso::read::IsoImage;

const SECTOR_SIZE: usize = 2048;

fn root_record() -> [u8; 34] {
    let mut record = [0_u8; 34];
    record[0] = 34;
    record[2..6].copy_from_slice(&20_u32.to_le_bytes());
    record[6..10].copy_from_slice(&20_u32.to_be_bytes());
    record[10..14].copy_from_slice(&(SECTOR_SIZE as u32).to_le_bytes());
    record[14..18].copy_from_slice(&(SECTOR_SIZE as u32).to_be_bytes());
    record[18..25].copy_from_slice(&[126, 1, 1, 0, 0, 0, 0]);
    record[25] = 2;
    record[28..30].copy_from_slice(&1_u16.to_le_bytes());
    record[30..32].copy_from_slice(&1_u16.to_be_bytes());
    record[32] = 1;
    record
}

fn primary_descriptor() -> [u8; SECTOR_SIZE] {
    let mut pvd = [0_u8; SECTOR_SIZE];
    pvd[0] = 1;
    pvd[1..6].copy_from_slice(b"CD001");
    pvd[6] = 1;
    pvd[40..72].fill(b' ');
    pvd[40..46].copy_from_slice(b"HADRIS");
    pvd[80..84].copy_from_slice(&21_u32.to_le_bytes());
    pvd[84..88].copy_from_slice(&21_u32.to_be_bytes());
    pvd[120..122].copy_from_slice(&1_u16.to_le_bytes());
    pvd[122..124].copy_from_slice(&1_u16.to_be_bytes());
    pvd[124..126].copy_from_slice(&1_u16.to_le_bytes());
    pvd[126..128].copy_from_slice(&1_u16.to_be_bytes());
    pvd[128..130].copy_from_slice(&(SECTOR_SIZE as u16).to_le_bytes());
    pvd[130..132].copy_from_slice(&(SECTOR_SIZE as u16).to_be_bytes());
    pvd[132..136].copy_from_slice(&10_u32.to_le_bytes());
    pvd[136..140].copy_from_slice(&10_u32.to_be_bytes());
    pvd[140..144].copy_from_slice(&19_u32.to_le_bytes());
    pvd[148..152].copy_from_slice(&19_u32.to_be_bytes());
    pvd[156..190].copy_from_slice(&root_record());
    for range in [190..318, 318..446, 446..574, 574..702] {
        pvd[range].fill(b' ');
    }
    pvd[813..830].copy_from_slice(b"2026010100000000\0");
    pvd[830..847].copy_from_slice(b"2026010100000000\0");
    pvd[847..864].copy_from_slice(b"0000000000000000\0");
    pvd[864..881].copy_from_slice(b"2026010100000000\0");
    pvd[881] = 1;
    pvd
}

fn terminator() -> [u8; SECTOR_SIZE] {
    let mut descriptor = [0_u8; SECTOR_SIZE];
    descriptor[0] = 255;
    descriptor[1..6].copy_from_slice(b"CD001");
    descriptor[6] = 1;
    descriptor
}

fn minimal_iso() -> Vec<u8> {
    let mut image = vec![0_u8; 21 * SECTOR_SIZE];
    image[16 * SECTOR_SIZE..17 * SECTOR_SIZE].copy_from_slice(&primary_descriptor());
    image[17 * SECTOR_SIZE..18 * SECTOR_SIZE].copy_from_slice(&terminator());
    let root = root_record();
    image[20 * SECTOR_SIZE..20 * SECTOR_SIZE + 34].copy_from_slice(&root);
    let mut parent = root;
    parent[33] = 1;
    image[20 * SECTOR_SIZE + 34..20 * SECTOR_SIZE + 68].copy_from_slice(&parent);
    image
}

#[test]
fn descriptor_sequence_opens_primary_volume_and_root_directory() {
    let image = IsoImage::open(Cursor::new(minimal_iso())).unwrap();
    let pvd = image.read_pvd().unwrap();
    assert_eq!(pvd.header.standard_identifier.to_str(), "CD001");
    assert_eq!(pvd.volume_identifier.to_str().trim_end(), "HADRIS");
    assert_eq!(
        image.root_dir().iter(&image).read_entries().unwrap().len(),
        2
    );
}

#[test]
fn malformed_primary_descriptor_and_terminator_cases_are_rejected() {
    type Corruption = (&'static str, fn(&mut [u8]));
    let cases: [Corruption; 5] = [
        ("missing primary", |image| image[16 * SECTOR_SIZE] = 0),
        ("invalid identifier", |image| {
            image[16 * SECTOR_SIZE + 1..16 * SECTOR_SIZE + 6].copy_from_slice(b"WRONG")
        }),
        ("missing terminator", |image| {
            image[17 * SECTOR_SIZE..18 * SECTOR_SIZE].fill(0)
        }),
        ("nonzero terminator body", |image| {
            image[17 * SECTOR_SIZE + 7] = 1
        }),
        ("inconsistent endian field", |image| {
            image[16 * SECTOR_SIZE + 84..16 * SECTOR_SIZE + 88]
                .copy_from_slice(&22_u32.to_be_bytes())
        }),
    ];

    for (name, corrupt) in cases {
        let mut image = minimal_iso();
        corrupt(&mut image);
        assert!(IsoImage::open(Cursor::new(image)).is_err(), "{name}");
    }
}

#[test]
fn non_2048_logical_block_size_is_rejected() {
    let mut image = minimal_iso();
    let offset = 16 * SECTOR_SIZE;
    image[offset + 128..offset + 130].copy_from_slice(&1024_u16.to_le_bytes());
    image[offset + 130..offset + 132].copy_from_slice(&1024_u16.to_be_bytes());
    assert!(IsoImage::open(Cursor::new(image)).is_err());
}
