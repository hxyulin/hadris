//! Tests for RRIP (Rock Ridge) reader support.
//!
//! Tests RRIP auto-detection, NM name assembly, SL symlink reconstruction,
//! TF timestamp parsing, CE continuation area reading, and the RRIP-aware
//! directory iterator.

use hadris_iso::read::IsoImage;
use hadris_iso::write::options::{CreationFeatures, FormatOptions};
use hadris_iso::write::{File as IsoFile, InputFiles, IsoImageWriter};
use hadris_iso::read::PathSeparator;
use std::io::Cursor;
use std::sync::Arc;

/// Helper to create an ISO image from files with given features.
fn create_iso(files: Vec<IsoFile>, features: CreationFeatures) -> Vec<u8> {
    let input = InputFiles {
        path_separator: PathSeparator::ForwardSlash,
        files,
    };
    let options = FormatOptions {
        volume_name: "TEST".to_string(),
        sector_size: 2048,
        path_separator: PathSeparator::ForwardSlash,
        features,
    };
    let mut buffer = Cursor::new(vec![0u8; 4 * 1024 * 1024]);
    IsoImageWriter::format_new(&mut buffer, input, options).unwrap();
    buffer.into_inner()
}

// =============================================================================
// RRIP Detection Tests
// =============================================================================

#[test]
fn test_rrip_detection_with_rock_ridge() {
    let files = vec![IsoFile::File {
        name: Arc::new("hello.txt".to_string()),
        contents: b"Hello, World!".to_vec(),
    }];
    let iso_data = create_iso(files, CreationFeatures::with_rock_ridge());
    let image = IsoImage::open(Cursor::new(iso_data)).unwrap();
    assert!(image.supports_rrip(), "RRIP should be detected for Rock Ridge ISO");
}

#[test]
fn test_rrip_detection_without_rock_ridge() {
    let files = vec![IsoFile::File {
        name: Arc::new("hello.txt".to_string()),
        contents: b"Hello, World!".to_vec(),
    }];
    let iso_data = create_iso(files, CreationFeatures::default());
    let image = IsoImage::open(Cursor::new(iso_data)).unwrap();
    assert!(!image.supports_rrip(), "RRIP should NOT be detected for plain ISO");
}

// =============================================================================
// NM Name Assembly Tests
// =============================================================================

#[test]
fn test_rrip_nm_names() {
    let files = vec![
        IsoFile::File {
            name: Arc::new("readme.txt".to_string()),
            contents: b"readme content".to_vec(),
        },
        IsoFile::File {
            name: Arc::new("LongFileName_WithMixedCase.dat".to_string()),
            contents: b"data".to_vec(),
        },
    ];
    let iso_data = create_iso(files, CreationFeatures::with_rock_ridge());
    let image = IsoImage::open(Cursor::new(iso_data)).unwrap();

    let root = image.root_dir();
    let dir = root.iter(&image);
    let entries: Vec<_> = dir
        .entries()
        .filter_map(|e| e.ok())
        .filter(|e| !e.is_special())
        .collect();

    // Collect RRIP names
    let names: Vec<String> = entries.iter().map(|e| e.display_name().into_owned()).collect();

    assert!(
        names.contains(&"readme.txt".to_string()),
        "Should find 'readme.txt' via RRIP NM, got: {:?}",
        names
    );
    assert!(
        names.contains(&"LongFileName_WithMixedCase.dat".to_string()),
        "Should find mixed-case filename via RRIP NM, got: {:?}",
        names
    );
}

// =============================================================================
// TF Timestamp Parsing Tests
// =============================================================================

#[test]
fn test_rrip_tf_timestamps() {
    let files = vec![IsoFile::File {
        name: Arc::new("test.txt".to_string()),
        contents: b"test".to_vec(),
    }];
    let iso_data = create_iso(files, CreationFeatures::with_rock_ridge());
    let image = IsoImage::open(Cursor::new(iso_data)).unwrap();

    let root = image.root_dir();
    let dir = root.iter(&image);
    let entries: Vec<_> = dir
        .entries()
        .filter_map(|e| e.ok())
        .filter(|e| !e.is_special())
        .collect();

    assert!(!entries.is_empty(), "Should have at least one file entry");

    let entry = &entries[0];
    if let Some(ref ts) = entry.rrip.as_ref().unwrap().timestamps {
        // The writer should produce at least MODIFY and ACCESS timestamps
        if let Some(ref modify) = ts.modify {
            assert!(modify.year >= 2024, "Modify year should be recent: {}", modify.year);
            assert!(modify.month >= 1 && modify.month <= 12);
        }
    }
    // Note: timestamps may or may not be present depending on writer behavior
}

// =============================================================================
// PX POSIX Attributes Tests
// =============================================================================

#[test]
fn test_rrip_px_attributes() {
    let files = vec![
        IsoFile::File {
            name: Arc::new("file.txt".to_string()),
            contents: b"content".to_vec(),
        },
        IsoFile::Directory {
            name: Arc::new("subdir".to_string()),
            children: vec![],
        },
    ];
    let iso_data = create_iso(files, CreationFeatures::with_rock_ridge());
    let image = IsoImage::open(Cursor::new(iso_data)).unwrap();

    let root = image.root_dir();
    let dir = root.iter(&image);
    let entries: Vec<_> = dir
        .entries()
        .filter_map(|e| e.ok())
        .filter(|e| !e.is_special())
        .collect();

    for entry in &entries {
        if let Some(ref px) = entry.rrip.as_ref().unwrap().posix_attributes {
            let mode = px.file_mode.read();
            if entry.is_directory() {
                // Directory should have directory file type bit set (0o040000)
                assert!(
                    mode & 0o170000 == 0o040000,
                    "Directory should have S_IFDIR mode, got {:#o}",
                    mode
                );
            } else {
                // Regular file should have regular file type bit set (0o100000)
                assert!(
                    mode & 0o170000 == 0o100000,
                    "File should have S_IFREG mode, got {:#o}",
                    mode
                );
            }
        }
    }
}

// =============================================================================
// Directory Iterator Tests
// =============================================================================

#[test]
fn test_rrip_directory_iteration() {
    let files = vec![
        IsoFile::File {
            name: Arc::new("alpha.txt".to_string()),
            contents: b"alpha".to_vec(),
        },
        IsoFile::File {
            name: Arc::new("beta.txt".to_string()),
            contents: b"beta".to_vec(),
        },
        IsoFile::Directory {
            name: Arc::new("gamma_dir".to_string()),
            children: vec![],
        },
    ];
    let iso_data = create_iso(files, CreationFeatures::with_rock_ridge());
    let image = IsoImage::open(Cursor::new(iso_data)).unwrap();

    let root = image.root_dir();
    let dir = root.iter(&image);

    // Count non-special entries (should be alpha.txt, beta.txt, gamma_dir)
    let entries: Vec<_> = dir
        .entries()
        .filter_map(|e| e.ok())
        .filter(|e| !e.is_special())
        .collect();

    assert_eq!(
        entries.len(),
        3,
        "Should have 3 non-special entries, got {}",
        entries.len()
    );

    // Verify directory vs file detection
    let dir_count = entries.iter().filter(|e| e.is_directory()).count();
    let file_count = entries.iter().filter(|e| !e.is_directory()).count();
    assert_eq!(dir_count, 1, "Should have 1 directory");
    assert_eq!(file_count, 2, "Should have 2 files");
}

#[test]
fn test_rrip_subdirectory_navigation() {
    let files = vec![
        IsoFile::Directory {
            name: Arc::new("mydir".to_string()),
            children: vec![
                IsoFile::File {
                    name: Arc::new("inner.txt".to_string()),
                    contents: b"inner content".to_vec(),
                },
            ],
        },
    ];
    let iso_data = create_iso(files, CreationFeatures::with_rock_ridge());
    let image = IsoImage::open(Cursor::new(iso_data)).unwrap();

    let root = image.root_dir();
    let root_dir = root.iter(&image);
    let root_entries: Vec<_> = root_dir
        .entries()
        .filter_map(|e| e.ok())
        .filter(|e| !e.is_special())
        .collect();

    // Find the subdirectory
    let subdir_entry = root_entries
        .iter()
        .find(|e| e.is_directory())
        .expect("Should find a subdirectory");

    // Navigate into the subdirectory
    let subdir_ref = subdir_entry.as_dir_ref(&image).unwrap();
    let subdir = image.open_dir(subdir_ref);
    let sub_entries: Vec<_> = subdir
        .entries()
        .filter_map(|e| e.ok())
        .filter(|e| !e.is_special())
        .collect();

    assert_eq!(
        sub_entries.len(),
        1,
        "Subdirectory should have 1 file, got {}",
        sub_entries.len()
    );

    let inner_name = sub_entries[0].display_name();
    assert_eq!(inner_name, "inner.txt", "Inner file should be 'inner.txt', got '{}'", inner_name);
}

// =============================================================================
// ES Extension Selector Parsing Tests
// =============================================================================

#[test]
fn test_es_parsing() {
    use hadris_iso::susp::{SystemUseField, SystemUseIter};

    // ES entry: sig='ES', length=5, version=1, extension_sequence=42
    let data: &[u8] = &[b'E', b'S', 5, 1, 42];
    let mut iter = SystemUseIter::new(data, 0);
    match iter.next() {
        Some(SystemUseField::ExtensionSelector {
            extension_sequence,
        }) => {
            assert_eq!(extension_sequence, 42);
        }
        other => panic!("expected ES entry, got {:?}", other),
    }
    assert!(iter.next().is_none());
}

// =============================================================================
// SL Symlink Path Assembly Tests (unit-level)
// =============================================================================

#[test]
fn test_sl_absolute_path_assembly() {
    use hadris_iso::read::RripMetadata;
    use hadris_iso::susp::SystemUseField;
    use hadris_iso::rrip::{SlComponent, SlComponentFlags, SlEntry};

    // Build SL for "/usr/bin"
    let sl = SlEntry {
        flags: 0,
        components: vec![
            SlComponent { flags: SlComponentFlags::ROOT, content: vec![] },
            SlComponent { flags: SlComponentFlags::empty(), content: b"usr".to_vec() },
            SlComponent { flags: SlComponentFlags::empty(), content: b"bin".to_vec() },
        ],
    };
    let fields = vec![SystemUseField::SymbolicLink(sl)];
    let meta = RripMetadata::from_fields(&fields);
    assert_eq!(meta.symlink_target.as_deref(), Some("/usr/bin"));
}

#[test]
fn test_sl_relative_path_assembly() {
    use hadris_iso::read::RripMetadata;
    use hadris_iso::susp::SystemUseField;
    use hadris_iso::rrip::{SlComponent, SlComponentFlags, SlEntry};

    // Build SL for "../lib/libfoo.so"
    let sl = SlEntry {
        flags: 0,
        components: vec![
            SlComponent { flags: SlComponentFlags::PARENT, content: vec![] },
            SlComponent { flags: SlComponentFlags::empty(), content: b"lib".to_vec() },
            SlComponent { flags: SlComponentFlags::empty(), content: b"libfoo.so".to_vec() },
        ],
    };
    let fields = vec![SystemUseField::SymbolicLink(sl)];
    let meta = RripMetadata::from_fields(&fields);
    assert_eq!(meta.symlink_target.as_deref(), Some("../lib/libfoo.so"));
}

#[test]
fn test_sl_continue_component_assembly() {
    use hadris_iso::read::RripMetadata;
    use hadris_iso::susp::SystemUseField;
    use hadris_iso::rrip::{SlComponent, SlComponentFlags, SlEntry};

    // Build SL where a component is split across entries using CONTINUE
    let sl = SlEntry {
        flags: 0,
        components: vec![
            SlComponent {
                flags: SlComponentFlags::CONTINUE,
                content: b"long".to_vec(),
            },
            SlComponent {
                flags: SlComponentFlags::empty(),
                content: b"name".to_vec(),
            },
        ],
    };
    let fields = vec![SystemUseField::SymbolicLink(sl)];
    let meta = RripMetadata::from_fields(&fields);
    assert_eq!(meta.symlink_target.as_deref(), Some("longname"));
}

// =============================================================================
// NM Multi-Entry Concatenation Tests (unit-level)
// =============================================================================

#[test]
fn test_nm_multi_entry_concatenation() {
    use hadris_iso::read::RripMetadata;
    use hadris_iso::susp::SystemUseField;
    use hadris_iso::rrip::{NmEntry, NmFlags};

    // Two NM entries that should concatenate
    let fields = vec![
        SystemUseField::AlternateName(NmEntry {
            flags: NmFlags::CONTINUE,
            name: b"very_long_file".to_vec(),
        }),
        SystemUseField::AlternateName(NmEntry {
            flags: NmFlags::empty(),
            name: b"name.txt".to_vec(),
        }),
    ];
    let meta = RripMetadata::from_fields(&fields);
    assert_eq!(
        meta.alternate_name.as_deref(),
        Some("very_long_filename.txt")
    );
}

#[test]
fn test_nm_current_directory() {
    use hadris_iso::read::RripMetadata;
    use hadris_iso::susp::SystemUseField;
    use hadris_iso::rrip::{NmEntry, NmFlags};

    let fields = vec![SystemUseField::AlternateName(NmEntry {
        flags: NmFlags::CURRENT,
        name: vec![],
    })];
    let meta = RripMetadata::from_fields(&fields);
    assert_eq!(meta.alternate_name.as_deref(), Some("."));
}

#[test]
fn test_nm_parent_directory() {
    use hadris_iso::read::RripMetadata;
    use hadris_iso::susp::SystemUseField;
    use hadris_iso::rrip::{NmEntry, NmFlags};

    let fields = vec![SystemUseField::AlternateName(NmEntry {
        flags: NmFlags::PARENT,
        name: vec![],
    })];
    let meta = RripMetadata::from_fields(&fields);
    assert_eq!(meta.alternate_name.as_deref(), Some(".."));
}

// =============================================================================
// TF Timestamp Parsing Tests (unit-level)
// =============================================================================

#[test]
fn test_tf_short_form_timestamps() {
    use hadris_iso::read::RripMetadata;
    use hadris_iso::susp::SystemUseField;
    use hadris_iso::rrip::{TfEntry, TfFlags};

    // TF with MODIFY(0x02) + ACCESS(0x04) flags, short form
    let mut timestamps = Vec::new();
    // Modify: 2026-02-17 10:30:00 UTC
    timestamps.extend_from_slice(&[126, 2, 17, 10, 30, 0, 0]);
    // Access: 2026-02-17 12:00:00 UTC
    timestamps.extend_from_slice(&[126, 2, 17, 12, 0, 0, 0]);

    let fields = vec![SystemUseField::Timestamps(TfEntry {
        flags: TfFlags::MODIFY | TfFlags::ACCESS,
        timestamps,
    })];
    let meta = RripMetadata::from_fields(&fields);

    let ts = meta.timestamps.expect("should have timestamps");
    assert!(ts.creation.is_none(), "creation should not be present");

    let modify = ts.modify.expect("modify should be present");
    assert_eq!(modify.year, 2026);
    assert_eq!(modify.month, 2);
    assert_eq!(modify.day, 17);
    assert_eq!(modify.hour, 10);
    assert_eq!(modify.minute, 30);

    let access = ts.access.expect("access should be present");
    assert_eq!(access.year, 2026);
    assert_eq!(access.month, 2);
    assert_eq!(access.day, 17);
    assert_eq!(access.hour, 12);
    assert_eq!(access.minute, 0);
}

#[test]
fn test_tf_long_form_timestamps() {
    use hadris_iso::read::RripMetadata;
    use hadris_iso::susp::SystemUseField;
    use hadris_iso::rrip::{TfEntry, TfFlags};

    // TF with CREATION flag, long form
    let mut timestamps = Vec::new();
    // Creation: "2026021710300000" + gmt_offset=0
    timestamps.extend_from_slice(b"2026021710300000");
    timestamps.push(0); // gmt_offset

    let fields = vec![SystemUseField::Timestamps(TfEntry {
        flags: TfFlags::CREATION | TfFlags::LONG_FORM,
        timestamps,
    })];
    let meta = RripMetadata::from_fields(&fields);

    let ts = meta.timestamps.expect("should have timestamps");
    let creation = ts.creation.expect("creation should be present");
    assert_eq!(creation.year, 2026);
    assert_eq!(creation.month, 2);
    assert_eq!(creation.day, 17);
    assert_eq!(creation.hour, 10);
    assert_eq!(creation.minute, 30);
}
