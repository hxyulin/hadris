#![cfg(all(feature = "std", feature = "sync"))]

use std::io::{Cursor, Seek, SeekFrom};

use hadris_cd::{Directory, FileEntry, FileTree, OpticalImageOptions, OpticalImageWriter};
use hadris_iso::sync::read::IsoImage;
use hadris_optical::detect::sync::detect;
use hadris_udf::dir::UdfDirEntry;
use hadris_udf::sync::UdfVolume;

const VOLUME_ID: &str = "BRIDGE_TEST";
const SECTOR_SIZE: usize = 2048;

fn tag_id_at(bytes: &[u8], sector: usize) -> u16 {
    let offset = sector * SECTOR_SIZE;
    u16::from_le_bytes([bytes[offset], bytes[offset + 1]])
}

fn fixture() -> (FileTree, Vec<u8>) {
    let large: Vec<u8> = (0..5000).map(|index| (index % 251) as u8).collect();
    let mut tree = FileTree::new();
    tree.add_file(FileEntry::from_buffer("EMPTY.TXT", Vec::new()));
    let mut docs = Directory::new("DOCS");
    docs.add_file(FileEntry::from_buffer("LARGE.BIN", large.clone()));
    let mut nested = Directory::new("NESTED");
    nested.add_file(FileEntry::from_buffer(
        "NOTE.TXT",
        b"qualified through both namespaces".to_vec(),
    ));
    docs.add_subdir(nested);
    tree.add_dir(docs);
    (tree, large)
}

fn create(options: OpticalImageOptions) -> Vec<u8> {
    let (tree, _) = fixture();
    OpticalImageWriter::create(Cursor::new(vec![0_u8; 4 * 1024 * 1024]), tree, options)
        .unwrap()
        .into_inner()
}

fn udf_entry<'a>(
    mut entries: impl Iterator<Item = &'a UdfDirEntry>,
    name: &str,
) -> &'a UdfDirEntry {
    entries
        .find(|entry| entry.name == name)
        .unwrap_or_else(|| panic!("missing UDF entry {name}"))
}

fn verify_iso(bytes: &[u8], large: &[u8]) {
    let image = IsoImage::open(Cursor::new(bytes)).expect("open ISO namespace");
    let pvd = image.read_pvd().expect("read ISO PVD");
    assert_eq!(pvd.volume_identifier.to_str().trim(), VOLUME_ID);

    let empty = image.find_path("EMPTY.TXT").unwrap().expect("empty file");
    assert_eq!(empty.total_size(), 0);
    assert_eq!(image.read_file(&empty).unwrap(), b"");

    let large_entry = image
        .find_path("DOCS/LARGE.BIN")
        .unwrap()
        .expect("large file");
    assert_eq!(large_entry.total_size(), large.len() as u64);
    assert_eq!(image.read_file(&large_entry).unwrap(), large);

    let note = image
        .find_path("DOCS/NESTED/NOTE.TXT")
        .unwrap()
        .expect("nested file");
    assert_eq!(
        image.read_file(&note).unwrap(),
        b"qualified through both namespaces"
    );
    assert_eq!(image.into_inner().get_ref().len(), bytes.len());
}

fn verify_udf(bytes: &[u8], large: &[u8]) {
    let volume = UdfVolume::open(Cursor::new(bytes)).expect("open UDF namespace");
    assert_eq!(volume.info().volume_id.trim_end_matches('\0'), VOLUME_ID);
    let root = volume.root_dir().expect("read UDF root");

    let empty = udf_entry(root.entries(), "EMPTY.TXT");
    assert_eq!(volume.read_file(empty).unwrap(), b"");

    let docs = udf_entry(root.entries(), "DOCS");
    let docs = volume.read_directory(&docs.icb).expect("read DOCS");
    let large_entry = udf_entry(docs.entries(), "LARGE.BIN");
    assert_eq!(volume.read_file(large_entry).unwrap(), large);

    let nested = udf_entry(docs.entries(), "NESTED");
    let nested = volume.read_directory(&nested.icb).expect("read NESTED");
    let note = udf_entry(nested.entries(), "NOTE.TXT");
    assert_eq!(
        volume.read_file(note).unwrap(),
        b"qualified through both namespaces"
    );
    assert_eq!(volume.into_inner().get_ref().len(), bytes.len());
}

#[test]
fn bridge_reopens_through_iso_and_udf_and_recovers_source() {
    let (tree, large) = fixture();
    let cursor = Cursor::new(vec![0_u8; 4 * 1024 * 1024]);
    let output =
        OpticalImageWriter::new(cursor, OpticalImageOptions::default().volume_id(VOLUME_ID))
            .finish(tree)
            .expect("create bridge");
    let bytes = output.into_inner();

    verify_iso(&bytes, &large);
    verify_udf(&bytes, &large);

    let mut source = Cursor::new(bytes.as_slice());
    source.seek(SeekFrom::Start(1234)).unwrap();
    let formats = detect(&mut source).unwrap().expect("detect bridge");
    assert!(formats.is_bridge());
    assert_eq!(source.stream_position().unwrap(), 1234);
}

#[test]
fn bridge_extends_a_compact_image_to_reserve_the_trailing_anchor() {
    let (tree, large) = fixture();
    let initial_sector_count = 512;
    let cursor = Cursor::new(vec![0_u8; initial_sector_count * SECTOR_SIZE]);
    let bytes =
        OpticalImageWriter::new(cursor, OpticalImageOptions::default().volume_id(VOLUME_ID))
            .finish(tree)
            .expect("create compact bridge")
            .into_inner();
    let sector_count = bytes.len() / SECTOR_SIZE;
    let trailing_anchor = sector_count - 1 - 256;

    assert!(
        sector_count > initial_sector_count,
        "the image must grow to reserve 256 sectors after the trailing anchor"
    );
    assert_eq!(tag_id_at(&bytes, trailing_anchor), 2);
    verify_iso(&bytes, &large);
    verify_udf(&bytes, &large);
}

#[test]
fn bridge_uses_udf_102_anchor_and_vds_layout() {
    let bytes = create(OpticalImageOptions::default().volume_id(VOLUME_ID));
    let sector_count = bytes.len() / SECTOR_SIZE;
    let last_sector = sector_count - 1;

    assert_eq!(tag_id_at(&bytes, 256), 2, "anchor at sector 256");
    assert_eq!(
        tag_id_at(&bytes, last_sector - 256),
        2,
        "second anchor at N-256"
    );
    assert_ne!(
        tag_id_at(&bytes, last_sector),
        2,
        "UDF 1.02 records exactly two candidate anchors"
    );

    let avdp = 256 * SECTOR_SIZE;
    let main_length = u32::from_le_bytes(bytes[avdp + 16..avdp + 20].try_into().unwrap()) as usize;
    let main_location =
        u32::from_le_bytes(bytes[avdp + 20..avdp + 24].try_into().unwrap()) as usize;
    let reserve_length =
        u32::from_le_bytes(bytes[avdp + 24..avdp + 28].try_into().unwrap()) as usize;
    let reserve_location =
        u32::from_le_bytes(bytes[avdp + 28..avdp + 32].try_into().unwrap()) as usize;

    assert_eq!(main_length, 16 * SECTOR_SIZE);
    assert_eq!(reserve_length, 16 * SECTOR_SIZE);
    assert_eq!(main_location, 257);
    assert_eq!(reserve_location, main_location + 16);
}

#[test]
fn bridge_descriptor_profile_is_visible_in_raw_sectors() {
    let bytes = create(OpticalImageOptions::default().volume_id(VOLUME_ID));

    let vrs_start = (16..256)
        .find(|&sector| {
            let offset = sector * SECTOR_SIZE;
            bytes.get(offset + 1..offset + 6) == Some(b"BEA01")
        })
        .expect("BEA01 after the ECMA-119 descriptor sequence");
    for (index, identifier) in [b"BEA01", b"NSR02", b"TEA01"].iter().enumerate() {
        let offset = (vrs_start + index) * SECTOR_SIZE;
        assert_eq!(&bytes[offset + 1..offset + 6], *identifier);
    }

    let descriptor_ids = [1_u16, 4, 5, 6, 7, 8];
    for (index, expected_id) in descriptor_ids.into_iter().enumerate() {
        let main_offset = (257 + index) * SECTOR_SIZE;
        let reserve_offset = (273 + index) * SECTOR_SIZE;
        assert_eq!(tag_id_at(&bytes, 257 + index), expected_id);
        assert_eq!(tag_id_at(&bytes, 273 + index), expected_id);

        let mut main = bytes[main_offset..main_offset + SECTOR_SIZE].to_vec();
        let mut reserve = bytes[reserve_offset..reserve_offset + SECTOR_SIZE].to_vec();
        main[4] = 0;
        reserve[4] = 0;
        main[8..10].fill(0);
        reserve[8..10].fill(0);
        main[12..16].fill(0);
        reserve[12..16].fill(0);
        if index == 0 {
            // The two PVD calls may sample the clock independently.
            main[376..388].fill(0);
            reserve[376..388].fill(0);
        }
        assert_eq!(main, reserve, "reserve VDS descriptor {index} differs");
    }

    let lvd_offset = 260 * SECTOR_SIZE;
    let integrity_length = u32::from_le_bytes(
        bytes[lvd_offset + 432..lvd_offset + 436]
            .try_into()
            .unwrap(),
    );
    let integrity_location = u32::from_le_bytes(
        bytes[lvd_offset + 436..lvd_offset + 440]
            .try_into()
            .unwrap(),
    ) as usize;
    assert_eq!(integrity_length, SECTOR_SIZE as u32);
    assert_eq!(tag_id_at(&bytes, integrity_location), 9);
    let integrity_offset = integrity_location * SECTOR_SIZE;
    assert_eq!(
        u32::from_le_bytes(
            bytes[integrity_offset + 28..integrity_offset + 32]
                .try_into()
                .unwrap()
        ),
        1,
        "the sole integrity descriptor is closed"
    );

    let partition_descriptor = 259 * SECTOR_SIZE;
    let partition_number = u16::from_le_bytes(
        bytes[partition_descriptor + 22..partition_descriptor + 24]
            .try_into()
            .unwrap(),
    );
    let partition_start = u32::from_le_bytes(
        bytes[partition_descriptor + 188..partition_descriptor + 192]
            .try_into()
            .unwrap(),
    ) as usize;
    let partition_length = u32::from_le_bytes(
        bytes[partition_descriptor + 192..partition_descriptor + 196]
            .try_into()
            .unwrap(),
    ) as usize;
    assert_eq!(partition_number, 0);
    assert_eq!(partition_start, 290);
    assert_eq!(
        partition_start + partition_length,
        bytes.len() / SECTOR_SIZE - 1 - 256,
        "the UDF partition must end before the N-256 anchor"
    );

    let mut file_entries = 0;
    for sector in partition_start..partition_start + partition_length {
        if tag_id_at(&bytes, sector) == 261 {
            file_entries += 1;
            let offset = sector * SECTOR_SIZE;
            let icb_flags = u16::from_le_bytes(bytes[offset + 34..offset + 36].try_into().unwrap());
            assert_eq!(
                icb_flags & 0x7,
                0,
                "File Entries must use short allocation descriptors"
            );
        }
    }
    assert!(file_entries > 0, "fixture should contain File Entries");
}

#[test]
fn bridge_rejects_non_2048_byte_logical_sectors() {
    let mut options = OpticalImageOptions::default().udf_only();
    options.sector_size = 4096;

    let result = OpticalImageWriter::create(
        Cursor::new(vec![0_u8; 4 * 1024 * 1024]),
        FileTree::new(),
        options,
    );
    assert!(
        matches!(result, Err(hadris_cd::Error::InvalidConfig(_))),
        "fixed-size UDF bridge structures require 2048-byte sectors"
    );
}

#[test]
fn udf_parent_fids_reference_the_actual_parent() {
    let bytes = create(OpticalImageOptions::default().volume_id(VOLUME_ID));
    let volume = UdfVolume::open(Cursor::new(bytes)).expect("open UDF namespace");
    let root = volume.root_dir().expect("read root");
    let docs_entry = udf_entry(root.entries(), "DOCS");
    let docs = volume.read_directory(&docs_entry.icb).expect("read DOCS");
    let docs_parent = docs
        .all_entries()
        .find(|entry| entry.is_parent())
        .expect("DOCS parent FID");
    assert_eq!(
        docs_parent.icb.logical_block_num,
        volume
            .root_dir()
            .unwrap()
            .all_entries()
            .find(|entry| entry.is_parent())
            .unwrap()
            .icb
            .logical_block_num
    );

    let nested_entry = udf_entry(docs.entries(), "NESTED");
    let nested = volume
        .read_directory(&nested_entry.icb)
        .expect("read NESTED");
    let nested_parent = nested
        .all_entries()
        .find(|entry| entry.is_parent())
        .expect("NESTED parent FID");
    assert_eq!(
        nested_parent.icb.logical_block_num,
        docs_entry.icb.logical_block_num
    );
}

#[test]
fn detects_and_reopens_iso_only_image() {
    let (_, large) = fixture();
    let bytes = create(
        OpticalImageOptions::default()
            .volume_id(VOLUME_ID)
            .iso_only(),
    );
    let formats = detect(&mut Cursor::new(bytes.as_slice()))
        .unwrap()
        .expect("detect ISO");
    assert!(formats.has_iso9660());
    assert!(formats.udf().is_none());
    verify_iso(&bytes, &large);
}

#[test]
fn detects_and_reopens_udf_only_image() {
    let (_, large) = fixture();
    let bytes = create(
        OpticalImageOptions::default()
            .volume_id(VOLUME_ID)
            .udf_only(),
    );
    let formats = detect(&mut Cursor::new(bytes.as_slice()))
        .unwrap()
        .expect("detect UDF");
    assert!(!formats.has_iso9660());
    assert!(formats.udf().is_some());
    verify_udf(&bytes, &large);
}
