//! Rock Ridge (SUSP/RRIP) records written by Hadris, checked by Hadris and by
//! xorriso when it is available.

use std::fs;

use hadris_fs::tree::{Content, Tree};
use hadris_iso::raw::{DirectoryRecord, SuspEntries};
use hadris_iso::{IsoOptions, RockRidge, VolumeIdentifiers};
use hadris_tests::iso::hadris::write_tree;
use hadris_tests::iso::xorriso;
use tempfile::TempDir;

use super::{open, volume_id};

fn le32(bytes: &[u8]) -> u32 {
    u32::from_le_bytes(bytes[..4].try_into().unwrap())
}

#[test]
fn test_hadris_rockridge_roundtrip() {
    let mut tree = Tree::new();
    tree.add_file("hello.txt", Content::bytes("Hello, Rock Ridge!\n"))
        .unwrap();
    tree.add_file("subdir/nested.txt", Content::bytes("Nested content\n"))
        .unwrap();
    let options = IsoOptions::default()
        .with_volume(VolumeIdentifiers::new("RRIP_TEST"))
        .with_rock_ridge(RockRidge::default());
    let iso_data = write_tree(&tree, &options).expect("Failed to create Rock Ridge ISO");

    let mut image = open(iso_data.clone());
    assert_eq!(volume_id(&mut image), "RRIP_TEST");
    let pvd = image.primary_descriptor().unwrap();
    let root_start = pvd.root.header.extent.get() as usize * 2048;
    let root_len = pvd.root.header.data_len.get() as usize;
    let root_dir = &iso_data[root_start..root_start + root_len];

    let mut records = Vec::new();
    let mut pos = 0;
    while pos < root_dir.len() {
        match DirectoryRecord::parse(&root_dir[pos..]).expect("valid directory record") {
            Some(record) => {
                pos += record.len();
                records.push(record);
            }
            None => pos = (pos / 2048 + 1) * 2048,
        }
    }

    let dot_entry = &records[0];
    assert_eq!(dot_entry.name(), b"\x00", "First entry should be dot");
    let su = dot_entry.system_use();
    assert!(!su.is_empty(), "Dot entry should have system use data");

    let mut found_sp = false;
    let mut found_ce = false;
    let mut found_px = false;
    let mut ce = (0usize, 0usize, 0usize);
    for entry in SuspEntries::new(su, 0) {
        match &entry.signature {
            b"SP" => {
                assert_eq!(&entry.data[..2], &[0xBE, 0xEF], "SP check bytes");
                found_sp = true;
            }
            b"CE" => {
                ce = (
                    le32(&entry.data[0..]) as usize,
                    le32(&entry.data[8..]) as usize,
                    le32(&entry.data[16..]) as usize,
                );
                found_ce = true;
            }
            b"PX" => found_px = true,
            _ => {}
        }
    }
    assert!(found_sp, "Root dot should have SP entry");
    // RRIP 4.1.4 permits but never requires an NM on the root dot record,
    // so none is asserted; a CE carrying the ER is mkisofs/xorriso practice.
    assert!(found_ce, "Root dot should have CE entry (for full ER)");
    assert!(found_px, "Root dot should have PX entry");

    let (ce_block, ce_offset, ce_length) = ce;
    assert!(ce_length > 0, "CE length should be non-zero");
    let ce_start = ce_block * 2048 + ce_offset;
    let ce_buf = &iso_data[ce_start..ce_start + ce_length];
    let mut found_er = false;
    for entry in SuspEntries::new(ce_buf, 0) {
        if &entry.signature == b"ER" {
            let (id_len, descriptor_len, source_len) = (
                entry.data[0] as usize,
                entry.data[1] as usize,
                entry.data[2] as usize,
            );
            if entry.data.get(4..4 + id_len) == Some(b"RRIP_1991A".as_slice()) {
                found_er = true;
                assert!(
                    descriptor_len > 0,
                    "Full ER should have non-empty descriptor"
                );
                assert!(source_len > 0, "Full ER should have non-empty source");
            }
        }
    }
    assert!(
        found_er,
        "Continuation area should contain ER with RRIP_1991A identifier"
    );

    let dotdot_entry = &records[1];
    assert_eq!(
        dotdot_entry.name(),
        b"\x01",
        "Second entry should be dotdot"
    );
    assert!(
        !dotdot_entry.system_use().is_empty(),
        "Dotdot entry should have system use data"
    );

    let found_file_with_nm = records[2..].iter().any(|record| {
        SuspEntries::new(record.system_use(), 0).any(|entry| &entry.signature == b"NM")
    });
    assert!(
        found_file_with_nm,
        "File/directory entries should have NM entries"
    );

    if xorriso::available() {
        let temp_dir = TempDir::new().unwrap();
        let iso_path = temp_dir.path().join("hadris_rrip.iso");
        fs::write(&iso_path, &iso_data).unwrap();
        let output = xorriso::inspect(&iso_path, &["-report_system_area", "plain", "-pvd_info"]);
        println!(
            "xorriso stdout: {}",
            String::from_utf8_lossy(&output.stdout)
        );
        println!(
            "xorriso stderr: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(
            output.status.success() || output.status.code() == Some(1),
            "xorriso should be able to read hadris RRIP ISO"
        );
        let output = xorriso::inspect(&iso_path, &["-ls", "/"]);
        println!("xorriso ls /: {}", String::from_utf8_lossy(&output.stdout));
    }
}
