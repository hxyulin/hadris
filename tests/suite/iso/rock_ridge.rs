//! Rock Ridge (SUSP/RRIP) records written by Hadris, checked by Hadris and by
//! xorriso when it is available.

use std::fs;
use std::io::Cursor;
use std::sync::Arc;

use hadris_iso::read::PathSeparator;
use hadris_iso::susp::{SystemUseField, SystemUseIter};
use hadris_iso::write::options::{CreationFeatures, IsoFormatOptions};
use hadris_iso::write::{File as IsoFile, InputFiles, IsoImageWriter};
use hadris_tests::iso::xorriso;
use tempfile::TempDir;

use super::open;

#[test]
fn test_hadris_rockridge_roundtrip() {
    let files = InputFiles {
        path_separator: PathSeparator::ForwardSlash,
        files: vec![
            IsoFile::File {
                name: Arc::new("hello.txt".to_string()),
                contents: b"Hello, Rock Ridge!\n".to_vec(),
            },
            IsoFile::Directory {
                name: Arc::new("subdir".to_string()),
                children: vec![IsoFile::File {
                    name: Arc::new("nested.txt".to_string()),
                    contents: b"Nested content\n".to_vec(),
                }],
            },
        ],
    };
    let format_options = IsoFormatOptions {
        volume_name: "RRIP_TEST".to_string(),
        system_id: None,
        volume_set_id: None,
        publisher_id: None,
        preparer_id: None,
        application_id: None,
        sector_size: 2048,
        path_separator: PathSeparator::ForwardSlash,
        features: CreationFeatures::rock_ridge(),
        strict_charset: false,
    };
    let mut buffer = Cursor::new(vec![0u8; 4 * 1024 * 1024]);
    IsoImageWriter::create(&mut buffer, files, format_options)
        .expect("Failed to create Rock Ridge ISO");
    let iso_data = buffer.into_inner();

    let image = open(iso_data.clone());
    let pvd = image.read_pvd().unwrap();
    assert_eq!(pvd.volume_identifier.to_str().trim(), "RRIP_TEST");

    let root = image.root_dir();
    let dir = root.iter(&image);
    let mut entries = dir.entries();

    let dot_entry = entries.next().unwrap().unwrap();
    assert_eq!(dot_entry.name(), b"\x00", "First entry should be dot");
    let su = dot_entry.system_use();
    assert!(!su.is_empty(), "Dot entry should have system use data");

    let mut found_sp = false;
    let mut found_ce = false;
    let mut found_px = false;
    let mut found_nm = false;
    let mut ce_sector = 0u64;
    let mut ce_offset = 0u64;
    let mut ce_length = 0usize;
    for field in SystemUseIter::new(su, 0) {
        match field {
            SystemUseField::SuspIdentifier(sp) => {
                assert!(sp.is_valid(), "SP check bytes should be 0xBEEF");
                found_sp = true;
            }
            SystemUseField::ContinuationArea(ce) => {
                ce_sector = ce.sector.read() as u64;
                ce_offset = ce.offset.read() as u64;
                ce_length = ce.length.read() as usize;
                found_ce = true;
            }
            SystemUseField::PosixAttributes(_) => found_px = true,
            SystemUseField::AlternateName(_) => found_nm = true,
            _ => {}
        }
    }
    assert!(found_sp, "Root dot should have SP entry");
    assert!(found_ce, "Root dot should have CE entry (for full ER)");
    assert!(found_px, "Root dot should have PX entry");
    assert!(found_nm, "Root dot should have NM entry");

    assert!(ce_length > 0, "CE length should be non-zero");
    let byte_pos = ce_sector * 2048 + ce_offset;
    let mut ce_buf = vec![0u8; ce_length];
    image
        .read_bytes_at(byte_pos, &mut ce_buf)
        .expect("Failed to read CE area");
    let mut found_er = false;
    for field in SystemUseIter::new(&ce_buf, 0) {
        if let SystemUseField::ExtensionReference(er) = field {
            let id_start = 4usize;
            let id_end = id_start + er.identifier_len as usize;
            if id_end <= er.buf.len() && &er.buf[id_start..id_end] == b"RRIP_1991A" {
                found_er = true;
                assert!(
                    er.descriptor_len > 0,
                    "Full ER should have non-empty descriptor"
                );
                assert!(er.source_len > 0, "Full ER should have non-empty source");
            }
        }
    }
    assert!(
        found_er,
        "Continuation area should contain ER with RRIP_1991A identifier"
    );

    let dotdot_entry = entries.next().unwrap().unwrap();
    assert_eq!(
        dotdot_entry.name(),
        b"\x01",
        "Second entry should be dotdot"
    );
    assert!(
        !dotdot_entry.system_use().is_empty(),
        "Dotdot entry should have system use data"
    );

    let mut found_file_with_nm = false;
    for entry_result in entries {
        let entry = entry_result.unwrap();
        if entry.is_special() {
            continue;
        }
        for field in SystemUseIter::new(entry.system_use(), 0) {
            if let SystemUseField::AlternateName(_) = field {
                found_file_with_nm = true;
            }
        }
    }
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
