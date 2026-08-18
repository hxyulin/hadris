//! Regression test (tool feature): the recursive directory walks in
//! `tool/analysis.rs` and `tool/verify.rs` must cap nesting depth and return
//! a graceful error on cyclic directory graphs instead of overflowing the
//! stack.
//!
//! Run: cargo test -p hadris-fat --features tool --test poc_audit_fat_recursion

#![cfg(feature = "tool")]

use std::io::Cursor;

use hadris_fat::{Error, FatAnalysisExt, FatVerifyExt, FatVolume};

/// FAT12 image whose root contains a directory D at cluster 2, and whose
/// cluster 2 in turn contains an entry D also pointing at cluster 2.
fn cyclic_dir_image() -> Vec<u8> {
    const ROOT_ENTRIES: usize = 16;
    const SPF: usize = 9;
    const ROOT_SECTORS: usize = ROOT_ENTRIES * 32 / 512;
    const TOTAL: usize = 1 + SPF + ROOT_SECTORS + 8;
    let root_offset = (1 + SPF) * 512;
    let data_offset = root_offset + ROOT_SECTORS * 512;

    let mut img = vec![0u8; TOTAL * 512];
    img[0..3].copy_from_slice(&[0xEB, 0x3C, 0x90]);
    img[3..11].copy_from_slice(b"MSDOS5.0");
    img[11..13].copy_from_slice(&512u16.to_le_bytes());
    img[13] = 1;
    img[14..16].copy_from_slice(&1u16.to_le_bytes());
    img[16] = 1;
    img[17..19].copy_from_slice(&(ROOT_ENTRIES as u16).to_le_bytes());
    img[19..21].copy_from_slice(&(TOTAL as u16).to_le_bytes());
    img[21] = 0xF8;
    img[22..24].copy_from_slice(&(SPF as u16).to_le_bytes());
    img[38] = 0x29;
    img[43..54].copy_from_slice(b"NO NAME    ");
    img[54..62].copy_from_slice(b"FAT12   ");
    img[510] = 0x55;
    img[511] = 0xAA;
    img[512] = 0xF8;
    img[513] = 0xFF;
    img[514] = 0xFF;
    // FAT[2] = EOC
    img[515] = 0xFF;
    img[516] = 0x4F;

    let mut put = |offset: usize, name: &[u8; 11], attr: u8, cluster: u16| {
        img[offset..offset + 11].copy_from_slice(name);
        img[offset + 11] = attr;
        img[offset + 26..offset + 28].copy_from_slice(&cluster.to_le_bytes());
    };

    // Root: directory D -> cluster 2
    put(root_offset, b"D          ", 0x10, 2);
    // Cluster 2: ".", "..", then D -> cluster 2 (cycle)
    put(data_offset, b".          ", 0x10, 2);
    put(data_offset + 32, b"..         ", 0x10, 0);
    put(data_offset + 64, b"D          ", 0x10, 2);
    img
}

/// Previously this recursed until the stack overflowed (process abort); now
/// the depth cap must surface `CorruptFilesystem`.
#[test]
fn poc_statistics_recurses_forever_on_cyclic_directory() {
    let fs = FatVolume::open(Cursor::new(cyclic_dir_image())).unwrap();
    assert!(matches!(
        fs.statistics(),
        Err(Error::CorruptFilesystem { .. })
    ));
}

#[test]
fn poc_fragmentation_report_depth_capped_on_cyclic_directory() {
    let fs = FatVolume::open(Cursor::new(cyclic_dir_image())).unwrap();
    assert!(matches!(
        fs.fragmentation_report(10),
        Err(Error::CorruptFilesystem { .. })
    ));
}

#[test]
fn poc_verify_depth_capped_on_cyclic_directory() {
    let fs = FatVolume::open(Cursor::new(cyclic_dir_image())).unwrap();
    assert!(matches!(fs.verify(), Err(Error::CorruptFilesystem { .. })));
}
