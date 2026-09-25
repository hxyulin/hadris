//! Rock Ridge (SUSP/RRIP) records written by Hadris, checked by Hadris and by
//! xorriso when it is available.

use std::fs;

use hadris_fs::Resolve;
use hadris_fs::sync::{FileSystem, Volume, read_tree};
use hadris_fs::{Content, Node, Tree};
use hadris_iso::raw::{DirectoryRecord, SuspEntries};
use hadris_iso::{IsoOptions, Namespace, RockRidge, VolumeIdentifiers};
use hadris_tests::harness::files::read_path;
use hadris_tests::iso::hadris::write_tree;
use hadris_tests::iso::xorriso;
use tempfile::TempDir;

use super::{open, open_ns, volume_id};

fn le32(bytes: &[u8]) -> u32 {
    u32::from_le_bytes(bytes[..4].try_into().unwrap())
}

#[test]
fn test_hadris_rockridge_roundtrip() {
    let mut tree = Tree::new();
    tree.insert(
        "hello.txt",
        Node::file(Content::bytes("Hello, Rock Ridge!\n")),
    )
    .unwrap();
    tree.insert(
        "subdir/nested.txt",
        Node::file(Content::bytes("Nested content\n")),
    )
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

/// The names of a Rock Ridge hard link share one node id, so importing
/// the image keeps them linked (integrate-2). Checked on an image Hadris
/// writes (shared `PX` serial) and one xorriso writes with `--hardlinks`.
#[test]
fn hard_links_share_a_node_id() {
    let mut tree = Tree::new();
    tree.insert("a.txt", Node::file(Content::bytes("data\n")))
        .unwrap();
    tree.insert("other.txt", Node::file(Content::bytes("other\n")))
        .unwrap();
    tree.link("a.txt", "sub/b.txt").unwrap();
    let options = IsoOptions::default().with_rock_ridge(RockRidge::default());
    let mut images = vec![("hadris", write_tree(&tree, &options).unwrap())];
    let temp = TempDir::new().unwrap();
    if xorriso::require() {
        let source = temp.path().join("source");
        fs::create_dir_all(source.join("sub")).unwrap();
        fs::write(source.join("a.txt"), "data\n").unwrap();
        fs::write(source.join("other.txt"), "other\n").unwrap();
        fs::hard_link(source.join("a.txt"), source.join("sub/b.txt")).unwrap();
        let image = temp.path().join("xorriso.iso");
        xorriso::mkisofs(&source, &image, "LINKS", &["-R", "--hardlinks"]).unwrap();
        images.push(("xorriso", fs::read(image).unwrap()));
    }
    for (producer, bytes) in images {
        let mut view = open_ns(bytes.clone(), Namespace::RockRidge);
        let a = view.resolve(b"/a.txt", Resolve::Lexical).unwrap();
        let b = view.resolve(b"/sub/b.txt", Resolve::Lexical).unwrap();
        let other = view.resolve(b"/other.txt", Resolve::Lexical).unwrap();
        assert_eq!(a, b, "{producer}");
        assert_ne!(a, other, "{producer}");
        assert_eq!(view.stat(b).unwrap().nlink(), 2);
        assert_eq!(read_path(&mut view, "/sub/b.txt").unwrap(), b"data\n");
        let sub = view.resolve(b"/sub", Resolve::Lexical).unwrap();
        let entry = view
            .readdir(sub, hadris_fs::DirCursor::START)
            .unwrap()
            .unwrap();
        assert_eq!(entry.node(), a, "{producer}");
        for node in [a, b, other, sub] {
            view.forget(node, 1);
        }

        let vol = Volume::new(open_ns(bytes, Namespace::RockRidge));
        let imported = read_tree(&vol, "/").unwrap();
        assert_eq!(
            imported.entry("sub/b.txt").unwrap().links(),
            2,
            "{producer}"
        );
        assert_eq!(
            imported.entry("a.txt").unwrap().id(),
            imported.entry("sub/b.txt").unwrap().id(),
            "{producer}"
        );
    }
}
