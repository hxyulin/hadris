//! The reader on structures the writer does not make: every allocation
//! descriptor form, continuation extents, embedded data, extended file
//! entries, identifiers that cross extents, and prevailing descriptors.

mod common;

use common::Paths;
use common::{image, open, pattern, reseal};
use hadris_fs::{Content, Node, Tree};
use hadris_udf::UdfOptions;
use hadris_udf::raw::{ExtendedFileEntry, FileEntry, Tag, Timestamp, U16Le, U32Le, U64Le, tag};

const PARTITION: u64 = 290;

/// A volume with `f.bin` and free blocks after the allocated ones.
fn volume() -> (Vec<u8>, u64, u64, u64) {
    let mut tree = Tree::new();
    tree.insert("f.bin", Node::file(Content::bytes(pattern(5000, 7))))
        .unwrap();
    for i in 0..80 {
        tree.insert(
            format!("many/file-{i:03}.txt"),
            Node::file(Content::bytes(format!("file {i}"))),
        )
        .unwrap();
    }
    let options = UdfOptions::default().with_min_blocks(1000);
    let report = hadris_udf::plan(&tree, &options).unwrap();
    let bytes = image(&tree, &options);
    let mut udf = open(bytes.clone());
    let f = udf.resolve_path("/f.bin").unwrap();
    let icb = PARTITION + f.get() - 1;
    let data = report.extents("f.bin").map(|e| e[0]).unwrap().offset() / 2048;
    let allocated_end = report
        .files()
        .flat_map(|(_, extents)| extents)
        .map(|extent| extent.end().div_ceil(2048))
        .max()
        .unwrap();
    (bytes, icb, data, allocated_end + 8)
}

fn sector(bytes: &mut [u8], sector: u64) -> &mut [u8] {
    &mut bytes[sector as usize * 2048..(sector as usize + 1) * 2048]
}

/// Rewrites the file entry at `icb` with allocation type `kind`, size
/// `size` and allocation descriptors `ads`.
fn set_ads(bytes: &mut [u8], icb: u64, kind: u16, size: u64, ads: &[u8]) {
    let fe = sector(bytes, icb);
    let flags = u16::from_le_bytes([fe[34], fe[35]]) & !7 | kind;
    fe[34..36].copy_from_slice(&flags.to_le_bytes());
    fe[56..64].copy_from_slice(&size.to_le_bytes());
    fe[172..176].copy_from_slice(&(ads.len() as u32).to_le_bytes());
    fe[176..].fill(0);
    fe[176..176 + ads.len()].copy_from_slice(ads);
    reseal(bytes, icb, 160 + ads.len());
}

fn ad(len: u32, kind: u32, block: u64) -> [u8; 8] {
    let mut out = [0u8; 8];
    out[..4].copy_from_slice(&(len | kind << 30).to_le_bytes());
    out[4..].copy_from_slice(&((block - PARTITION) as u32).to_le_bytes());
    out
}

#[test]
fn allocation_descriptors_of_every_form_read_back() {
    let (base, icb, data, free) = volume();
    let expected = pattern(5000, 7);

    let mut bytes = base.clone();
    let mut long = [0u8; 16];
    long[..4].copy_from_slice(&5000u32.to_le_bytes());
    long[4..8].copy_from_slice(&((data - PARTITION) as u32).to_le_bytes());
    set_ads(&mut bytes, icb, 1, 5000, &long);
    assert_eq!(open(bytes).read_to_vec("/f.bin").unwrap(), expected);

    let mut bytes = base.clone();
    let mut ext = [0u8; 20];
    for at in [0, 4, 8] {
        ext[at..at + 4].copy_from_slice(&5000u32.to_le_bytes());
    }
    ext[12..16].copy_from_slice(&((data - PARTITION) as u32).to_le_bytes());
    set_ads(&mut bytes, icb, 2, 5000, &ext);
    assert_eq!(open(bytes).read_to_vec("/f.bin").unwrap(), expected);

    let mut bytes = base.clone();
    let (first, aed, rest) = (free, free + 1, free + 2);
    sector(&mut bytes, first).copy_from_slice(&expected[..2048]);
    let at = rest as usize * 2048;
    bytes[at..at + 2952].copy_from_slice(&expected[2048..]);
    let descriptors = [ad(2048, 1, PARTITION), ad(2952, 0, rest)].concat();
    let block = sector(&mut bytes, aed);
    block.fill(0);
    block[20..24].copy_from_slice(&(descriptors.len() as u32).to_le_bytes());
    block[24..24 + descriptors.len()].copy_from_slice(&descriptors);
    Tag::seal(
        block,
        tag::ALLOCATION_EXTENT,
        2,
        (aed - PARTITION) as u32,
        8 + descriptors.len(),
    );
    set_ads(
        &mut bytes,
        icb,
        0,
        7048,
        &[ad(2048, 0, first), ad(2048, 3, aed)].concat(),
    );
    let read = open(bytes).read_to_vec("/f.bin").unwrap();
    assert_eq!(&read[..2048], &expected[..2048]);
    assert!(read[2048..4096].iter().all(|&b| b == 0));
    assert_eq!(&read[4096..], &expected[2048..]);

    let mut bytes = base.clone();
    set_ads(&mut bytes, icb, 3, 100, &expected[..100]);
    let mut udf = open(bytes);
    assert_eq!(udf.read_to_vec("/f.bin").unwrap(), &expected[..100]);
    let f = udf.resolve_path("/f.bin").unwrap();
    let mut out = [hadris_fs::Extent::new(0, 0); 4];
    assert_eq!(udf.extents(f, 0, &mut out).unwrap(), 1);
    let mut embedded = [0u8; 100];
    udf.read_raw(out[0].offset(), &mut embedded).unwrap();
    assert_eq!(embedded, expected[..100]);

    let mut bytes = base;
    let looped = sector(&mut bytes, aed);
    looped.fill(0);
    let own = ad(2048, 3, aed);
    looped[20..24].copy_from_slice(&8u32.to_le_bytes());
    looped[24..32].copy_from_slice(&own);
    Tag::seal(
        looped,
        tag::ALLOCATION_EXTENT,
        2,
        (aed - PARTITION) as u32,
        16,
    );
    set_ads(&mut bytes, icb, 0, 7048, &ad(2048, 3, aed));
    let err = open(bytes).read_to_vec("/f.bin").unwrap_err();
    assert_eq!(err.kind(), hadris_fs::ErrorKind::Corrupt);
}

#[test]
fn extended_file_entries_read() {
    let (mut bytes, icb, _, _) = volume();
    let fe: FileEntry = bytemuck::pod_read_unaligned(&sector(&mut bytes, icb)[..176]);
    let ads = sector(&mut bytes, icb)[176..184].to_vec();
    let created = Timestamp {
        type_and_zone: U16Le::new(0x1000),
        year: U16Le::new(2001),
        month: 2,
        day: 3,
        hour: 4,
        minute: 5,
        second: 6,
        ..Default::default()
    };
    let efe = ExtendedFileEntry {
        tag: fe.tag,
        icb_tag: fe.icb_tag,
        uid: fe.uid,
        gid: fe.gid,
        permissions: fe.permissions,
        link_count: fe.link_count,
        record_format: 0,
        record_display_attributes: 0,
        record_length: U32Le::new(0),
        information_length: fe.information_length,
        object_size: fe.information_length,
        blocks_recorded: U64Le::new(3),
        accessed: fe.accessed,
        modified: fe.modified,
        created,
        attributes_changed: fe.attributes_changed,
        checkpoint: fe.checkpoint,
        reserved: U32Le::new(0),
        extended_attribute_icb: fe.extended_attribute_icb,
        stream_directory_icb: Default::default(),
        implementation: fe.implementation,
        unique_id: fe.unique_id,
        extended_attributes_length: U32Le::new(0),
        allocation_descriptors_length: U32Le::new(8),
    };
    let block = sector(&mut bytes, icb);
    block.fill(0);
    block[..216].copy_from_slice(bytemuck::bytes_of(&efe));
    block[216..224].copy_from_slice(&ads);
    Tag::seal(
        block,
        tag::EXTENDED_FILE_ENTRY,
        2,
        efe.tag.location.get(),
        208,
    );
    let mut udf = open(bytes);
    assert_eq!(udf.read_to_vec("/f.bin").unwrap(), pattern(5000, 7));
    let created = udf.metadata("/f.bin").unwrap().created().unwrap();
    assert_eq!(created.unix_seconds(), 981_173_106);
}

#[test]
fn identifiers_cross_extent_boundaries() {
    let (mut bytes, _, _, free) = volume();
    let mut udf = open(bytes.clone());
    let many = udf.resolve_path("/many").unwrap();
    let icb = PARTITION + many.get() - 1;
    let fe = sector(&mut bytes, icb).to_vec();
    let len = u32::from_le_bytes(fe[176..180].try_into().unwrap());
    let start = PARTITION + u64::from(u32::from_le_bytes(fe[180..184].try_into().unwrap()));
    assert!(len > 4096);
    for i in 1..3 {
        let moved = sector(&mut bytes, start + i).to_vec();
        sector(&mut bytes, free + i - 1).copy_from_slice(&moved);
        sector(&mut bytes, start + i).fill(0xEE);
    }
    set_ads(
        &mut bytes,
        icb,
        0,
        u64::from(len),
        &[ad(2048, 0, start), ad(len - 2048, 0, free)].concat(),
    );
    let mut udf = open(bytes);
    let names = udf.names("/many").unwrap();
    assert_eq!(names.len(), 80);
    assert_eq!(names[79], "file-079.txt");
    assert_eq!(udf.read_to_vec("/many/file-050.txt").unwrap(), b"file 50");
}

#[test]
fn prevailing_descriptors_win() {
    let (mut bytes, ..) = volume();
    let pvd = sector(&mut bytes, 257).to_vec();
    let newer = sector(&mut bytes, 262);
    newer.copy_from_slice(&pvd);
    newer[16..20].copy_from_slice(&7u32.to_le_bytes());
    newer[24..56].fill(0);
    newer[24] = 8;
    newer[25..30].copy_from_slice(b"NEWER");
    newer[55] = 6;
    Tag::seal(newer, tag::PRIMARY_VOLUME, 2, 262, 496);
    let terminator = sector(&mut bytes, 263);
    terminator.fill(0);
    Tag::seal(terminator, tag::TERMINATING, 2, 263, 0);
    let udf = open(bytes);
    assert_eq!(udf.info().id(hadris_udf::UdfId::Volume), "NEWER");
    assert_eq!(
        udf.info().id(hadris_udf::UdfId::LogicalVolume),
        "UDF_VOLUME"
    );
}
