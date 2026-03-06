//! Advanced ISO 9660 roundtrip tests:
//! - BFS path table generation and traversal
//! - Filename deduplication for ISO compliance
//! - Directory depth enforcement (8-level limit)
//! - Image size estimation accuracy
//! - RRIP TF timestamp preservation
//! - Subdirectory navigation via open_dir()

use std::io::Cursor;
use std::sync::Arc;

use hadris_iso::directory::FileFlags;
use hadris_iso::read::{IsoImage, PathSeparator};
use hadris_iso::susp::{SystemUseField, SystemUseIter};
use hadris_iso::write::options::{CreationFeatures, FormatOptions};
use hadris_iso::write::{File as IsoFile, InputFiles, IsoImageWriter, estimator};

// ── Helpers ──

fn default_options() -> FormatOptions {
    FormatOptions {
        volume_name: "TEST".to_string(),
        system_id: None,
        volume_set_id: None,
        publisher_id: None,
        preparer_id: None,
        application_id: None,
        sector_size: 2048,
        path_separator: PathSeparator::ForwardSlash,
        features: CreationFeatures::default(),
    }
}

fn rrip_options() -> FormatOptions {
    FormatOptions {
        volume_name: "RRIP_TEST".to_string(),
        system_id: None,
        volume_set_id: None,
        publisher_id: None,
        preparer_id: None,
        application_id: None,
        sector_size: 2048,
        path_separator: PathSeparator::ForwardSlash,
        features: CreationFeatures::with_rock_ridge(),
    }
}

fn joliet_options() -> FormatOptions {
    use hadris_iso::joliet::JolietLevel;
    FormatOptions {
        volume_name: "JOLIET_TEST".to_string(),
        system_id: None,
        volume_set_id: None,
        publisher_id: None,
        preparer_id: None,
        application_id: None,
        sector_size: 2048,
        path_separator: PathSeparator::ForwardSlash,
        features: CreationFeatures::with_joliet(JolietLevel::Level3),
    }
}

fn write_and_open(files: Vec<IsoFile>, options: FormatOptions) -> IsoImage<Cursor<Vec<u8>>> {
    let input = InputFiles {
        path_separator: PathSeparator::ForwardSlash,
        files,
    };
    let mut buffer = Cursor::new(vec![0u8; 8 * 1024 * 1024]);
    IsoImageWriter::format_new(&mut buffer, input, options).expect("Failed to write ISO");
    let data = buffer.into_inner();
    IsoImage::open(Cursor::new(data)).expect("Failed to open ISO")
}

/// Build a chain of N nested directories with a leaf file.
fn nested_dir(depth: usize) -> IsoFile {
    let mut current = IsoFile::File {
        name: Arc::new("deep.txt".to_string()),
        contents: b"deep content".to_vec(),
    };
    for i in (0..depth).rev() {
        current = IsoFile::Directory {
            name: Arc::new(format!("dir{}", i)),
            children: vec![current],
        };
    }
    current
}

/// Collect non-special entry names from a directory.
fn entry_names(
    image: &IsoImage<Cursor<Vec<u8>>>,
    dir_ref: hadris_iso::directory::DirectoryRef,
) -> Vec<String> {
    let dir = image.open_dir(dir_ref);
    dir.entries()
        .filter_map(|e| e.ok())
        .filter(|e| !e.is_special())
        .map(|e| {
            let name = String::from_utf8_lossy(e.name()).to_string();
            if let Some(pos) = name.rfind(';') {
                name[..pos].to_string()
            } else {
                name
            }
        })
        .collect()
}

// ── Tests ──

#[test]
fn test_path_table_contains_all_subdirectories() {
    // Write root/{a/{b/{c/deep.txt}}, x/{y/leaf.txt}}
    let files = vec![
        IsoFile::Directory {
            name: Arc::new("a".to_string()),
            children: vec![IsoFile::Directory {
                name: Arc::new("b".to_string()),
                children: vec![IsoFile::Directory {
                    name: Arc::new("c".to_string()),
                    children: vec![IsoFile::File {
                        name: Arc::new("deep.txt".to_string()),
                        contents: b"deep".to_vec(),
                    }],
                }],
            }],
        },
        IsoFile::Directory {
            name: Arc::new("x".to_string()),
            children: vec![IsoFile::Directory {
                name: Arc::new("y".to_string()),
                children: vec![IsoFile::File {
                    name: Arc::new("leaf.txt".to_string()),
                    contents: b"leaf".to_vec(),
                }],
            }],
        },
    ];

    let image = write_and_open(files, default_options());
    let pt = image.path_table();
    let entries: Vec<_> = pt.entries(&image).collect::<Result<Vec<_>, _>>().unwrap();

    // Should have 6 entries: root + a + b + c + x + y
    assert_eq!(
        entries.len(),
        6,
        "Expected 6 path table entries (root + 5 dirs), got {}",
        entries.len()
    );

    // Root entry should have parent index 1 (itself)
    assert_eq!(entries[0].parent_index, 1, "Root parent should be 1");

    // BFS order: root(1), a(2), x(3), b(4), y(5), c(6)
    // All entries after root should have valid parent indices
    for entry in &entries[1..] {
        assert!(
            entry.parent_index >= 1 && entry.parent_index <= entries.len() as u16,
            "Invalid parent index: {}",
            entry.parent_index
        );
    }
}

#[test]
fn test_name_deduplication_produces_unique_names() {
    // Write 3 files that all map to README.TXT;1 at L1
    let files = vec![
        IsoFile::File {
            name: Arc::new("readme.txt".to_string()),
            contents: b"a".to_vec(),
        },
        IsoFile::File {
            name: Arc::new("README.txt".to_string()),
            contents: b"b".to_vec(),
        },
        IsoFile::File {
            name: Arc::new("ReadMe.txt".to_string()),
            contents: b"c".to_vec(),
        },
    ];

    let image = write_and_open(files, default_options());
    let root = image.root_dir();
    let names = entry_names(&image, root.dir_ref());

    assert_eq!(names.len(), 3, "Should have 3 entries, got: {:?}", names);

    // All names should be distinct
    let mut unique = names.clone();
    unique.sort();
    unique.dedup();
    assert_eq!(unique.len(), 3, "All names should be distinct: {:?}", names);
}

#[test]
fn test_dedup_with_no_extension() {
    let files = vec![
        IsoFile::File {
            name: Arc::new("README".to_string()),
            contents: b"a".to_vec(),
        },
        IsoFile::File {
            name: Arc::new("readme".to_string()),
            contents: b"b".to_vec(),
        },
    ];

    let image = write_and_open(files, default_options());
    let root = image.root_dir();
    let names = entry_names(&image, root.dir_ref());

    assert_eq!(names.len(), 2, "Should have 2 entries, got: {:?}", names);

    let mut unique = names.clone();
    unique.sort();
    unique.dedup();
    assert_eq!(unique.len(), 2, "Names should be unique: {:?}", names);
}

#[test]
fn test_depth_8_succeeds() {
    // 7 nested dirs + root = 8 levels (the maximum)
    let files = vec![nested_dir(7)];
    let input = InputFiles {
        path_separator: PathSeparator::ForwardSlash,
        files,
    };
    let mut buffer = Cursor::new(vec![0u8; 8 * 1024 * 1024]);
    let result = IsoImageWriter::format_new(&mut buffer, input, default_options());
    assert!(result.is_ok(), "Depth 8 should succeed: {:?}", result.err());
}

#[test]
fn test_depth_9_fails() {
    // 8 nested dirs + root = 9 levels (exceeds the limit)
    let files = vec![nested_dir(8)];
    let input = InputFiles {
        path_separator: PathSeparator::ForwardSlash,
        files,
    };
    let mut buffer = Cursor::new(vec![0u8; 8 * 1024 * 1024]);
    let result = IsoImageWriter::format_new(&mut buffer, input, default_options());
    assert!(result.is_err(), "Depth 9 should fail");
}

#[test]
fn test_size_estimate_is_conservative() {
    let configs: Vec<(&str, Vec<IsoFile>, FormatOptions)> = vec![
        (
            "simple",
            vec![IsoFile::File {
                name: Arc::new("test.txt".to_string()),
                contents: b"hello".to_vec(),
            }],
            default_options(),
        ),
        (
            "nested",
            vec![IsoFile::Directory {
                name: Arc::new("sub".to_string()),
                children: vec![IsoFile::File {
                    name: Arc::new("inner.txt".to_string()),
                    contents: vec![0u8; 4096],
                }],
            }],
            default_options(),
        ),
        (
            "rrip",
            vec![IsoFile::File {
                name: Arc::new("rrip_file.txt".to_string()),
                contents: b"rrip content".to_vec(),
            }],
            rrip_options(),
        ),
        (
            "joliet",
            vec![IsoFile::File {
                name: Arc::new("joliet_file.txt".to_string()),
                contents: b"joliet content".to_vec(),
            }],
            joliet_options(),
        ),
    ];

    for (label, files, options) in configs {
        let input = InputFiles {
            path_separator: PathSeparator::ForwardSlash,
            files: files.clone(),
        };
        let estimate = estimator::estimate(&input, &options);

        let input2 = InputFiles {
            path_separator: PathSeparator::ForwardSlash,
            files,
        };
        let mut buffer = Cursor::new(vec![
            0u8;
            estimate.minimum_bytes() as usize + 4 * 1024 * 1024
        ]);
        IsoImageWriter::format_new(&mut buffer, input2, options).expect("Failed to write ISO");

        use std::io::Seek;
        let actual_pos = buffer.stream_position().unwrap();
        assert!(
            estimate.minimum_bytes() >= actual_pos,
            "[{}] Estimate {} should be >= actual position {}",
            label,
            estimate.minimum_bytes(),
            actual_pos
        );
    }
}

#[test]
fn test_rrip_tf_timestamps_present() {
    let files = vec![IsoFile::File {
        name: Arc::new("timestamped.txt".to_string()),
        contents: b"hello".to_vec(),
    }];

    let image = write_and_open(files, rrip_options());
    let root = image.root_dir();
    let dir = root.iter(&image);

    let mut found_tf = false;
    for entry in dir.entries() {
        let entry = entry.unwrap();
        if entry.is_special() {
            continue;
        }
        let su = entry.system_use();
        if su.is_empty() {
            continue;
        }
        for field in SystemUseIter::new(su, 0) {
            if let SystemUseField::Timestamps(_) = field {
                found_tf = true;
            }
        }
    }

    assert!(
        found_tf,
        "TF timestamp entry should be present in RRIP file entries"
    );
}

#[test]
fn test_roundtrip_multi_level_directories() {
    let files = vec![
        IsoFile::File {
            name: Arc::new("file1.txt".to_string()),
            contents: b"root file".to_vec(),
        },
        IsoFile::Directory {
            name: Arc::new("dir_a".to_string()),
            children: vec![
                IsoFile::File {
                    name: Arc::new("file2.txt".to_string()),
                    contents: b"level 2 file".to_vec(),
                },
                IsoFile::Directory {
                    name: Arc::new("dir_b".to_string()),
                    children: vec![IsoFile::File {
                        name: Arc::new("file3.txt".to_string()),
                        contents: b"level 3 file".to_vec(),
                    }],
                },
            ],
        },
    ];

    let image = write_and_open(files, default_options());
    let root = image.root_dir();

    // Root should have file1.txt and dir_a
    let root_names = entry_names(&image, root.dir_ref());
    assert!(
        root_names.iter().any(|n| n.contains("FILE1")),
        "Root should contain FILE1.TXT: {:?}",
        root_names
    );
    assert!(
        root_names.iter().any(|n| n.contains("DIR_A")),
        "Root should contain DIR_A: {:?}",
        root_names
    );

    // Navigate into dir_a
    let dir_a_ref = {
        let dir = image.open_dir(root.dir_ref());
        dir.entries()
            .filter_map(|e| e.ok())
            .find(|e| {
                let name = String::from_utf8_lossy(e.name());
                name.contains("DIR_A") && e.is_directory()
            })
            .expect("Should find DIR_A")
            .as_dir_ref(&image)
            .unwrap()
    };

    let dir_a_names = entry_names(&image, dir_a_ref);
    assert!(
        dir_a_names.iter().any(|n| n.contains("FILE2")),
        "DIR_A should contain FILE2.TXT: {:?}",
        dir_a_names
    );
    assert!(
        dir_a_names.iter().any(|n| n.contains("DIR_B")),
        "DIR_A should contain DIR_B: {:?}",
        dir_a_names
    );

    // Navigate into dir_b
    let dir_b_ref = {
        let dir = image.open_dir(dir_a_ref);
        dir.entries()
            .filter_map(|e| e.ok())
            .find(|e| {
                let name = String::from_utf8_lossy(e.name());
                name.contains("DIR_B") && e.is_directory()
            })
            .expect("Should find DIR_B")
            .as_dir_ref(&image)
            .unwrap()
    };

    let dir_b_names = entry_names(&image, dir_b_ref);
    assert!(
        dir_b_names.iter().any(|n| n.contains("FILE3")),
        "DIR_B should contain FILE3.TXT: {:?}",
        dir_b_names
    );
}

#[test]
fn test_empty_directory_roundtrip() {
    let files = vec![IsoFile::Directory {
        name: Arc::new("empty".to_string()),
        children: vec![],
    }];

    let image = write_and_open(files, default_options());
    let root = image.root_dir();

    // Find the empty directory
    let empty_ref = {
        let dir = image.open_dir(root.dir_ref());
        dir.entries()
            .filter_map(|e| e.ok())
            .find(|e| {
                let name = String::from_utf8_lossy(e.name());
                name.contains("EMPTY") && e.is_directory()
            })
            .expect("Should find EMPTY directory")
            .as_dir_ref(&image)
            .unwrap()
    };

    // Empty directory should only have dot and dotdot
    let names = entry_names(&image, empty_ref);
    assert!(
        names.is_empty(),
        "Empty directory should have no non-special entries: {:?}",
        names
    );
}

#[test]
fn test_zero_size_file_roundtrip() {
    let files = vec![IsoFile::File {
        name: Arc::new("empty.txt".to_string()),
        contents: vec![],
    }];

    let image = write_and_open(files, default_options());
    let root = image.root_dir();

    let dir = image.open_dir(root.dir_ref());
    let empty_file = dir
        .entries()
        .filter_map(|e| e.ok())
        .find(|e| {
            let name = String::from_utf8_lossy(e.name());
            name.contains("EMPTY") && !e.is_directory()
        })
        .expect("Should find EMPTY.TXT");

    let header = empty_file.header();
    assert_eq!(
        header.data_len.read(),
        0,
        "Zero-size file should have size 0"
    );
    assert_eq!(
        header.extent.read(),
        0,
        "Zero-size file should have extent 0"
    );
}

#[test]
fn test_joliet_subdirectory_roundtrip() {
    let files = vec![IsoFile::Directory {
        name: Arc::new("subdir".to_string()),
        children: vec![IsoFile::File {
            name: Arc::new("inner.txt".to_string()),
            contents: b"joliet inner".to_vec(),
        }],
    }];

    let image = write_and_open(files, joliet_options());

    // Use the best root (which should be Joliet if available)
    let root = image.root_dir();
    let root_ref = root.dir_ref();

    // Check root has entries
    let dir = image.open_dir(root_ref);
    let entries: Vec<_> = dir
        .entries()
        .filter_map(|e| e.ok())
        .filter(|e| !e.is_special())
        .collect();
    assert!(!entries.is_empty(), "Root should have entries");

    // Find a directory entry and navigate into it
    let subdir_entry = entries.iter().find(|e| e.is_directory());
    assert!(subdir_entry.is_some(), "Should find a subdirectory entry");

    let subdir_ref = subdir_entry.unwrap().as_dir_ref(&image).unwrap();
    let inner_dir = image.open_dir(subdir_ref);
    let inner_entries: Vec<_> = inner_dir
        .entries()
        .filter_map(|e| e.ok())
        .filter(|e| !e.is_special())
        .collect();
    assert!(
        !inner_entries.is_empty(),
        "Subdirectory should contain the inner file"
    );
}

#[test]
fn test_rrip_typed_fields_roundtrip() {
    let files = vec![
        IsoFile::Directory {
            name: Arc::new("subdir".to_string()),
            children: vec![IsoFile::File {
                name: Arc::new("inner.txt".to_string()),
                contents: b"rrip typed test".to_vec(),
            }],
        },
        IsoFile::File {
            name: Arc::new("readme.txt".to_string()),
            contents: b"hello rrip".to_vec(),
        },
    ];

    let image = write_and_open(files, rrip_options());
    let root = image.root_dir();
    let dir = root.iter(&image);

    let mut found_px = false;
    let mut found_nm = false;
    let mut found_tf = false;

    for entry in dir.entries() {
        let entry = entry.unwrap();
        let su = entry.system_use();
        if su.is_empty() {
            continue;
        }

        for field in SystemUseIter::new(su, 0) {
            match &field {
                SystemUseField::PosixAttributes(px) => {
                    found_px = true;
                    // Mode should be set (file or directory)
                    let mode = px.file_mode.read();
                    assert!(mode != 0, "PX mode should be non-zero");
                    // Check that file type bits make sense
                    let is_dir_entry = entry.name() == b"\x00"
                        || entry.name() == b"\x01"
                        || FileFlags::from_bits_truncate(entry.header().flags)
                            .contains(FileFlags::DIRECTORY);
                    if is_dir_entry {
                        assert_eq!(mode & 0o170000, 0o040000, "Directory should have dir mode");
                    } else {
                        assert_eq!(mode & 0o170000, 0o100000, "File should have regular mode");
                    }
                }
                SystemUseField::AlternateName(nm) => {
                    found_nm = true;
                    // Name should be non-empty (unless it's . or .. special)
                    if nm.flags.is_empty() {
                        assert!(
                            !nm.name.is_empty(),
                            "NM name should be non-empty for normal entries"
                        );
                    }
                }
                SystemUseField::Timestamps(tf) => {
                    found_tf = true;
                    assert!(
                        !tf.timestamps.is_empty(),
                        "TF timestamps should be non-empty"
                    );
                }
                // Ensure PX/NM/TF are NOT returned as Unknown
                SystemUseField::Unknown(header, _) => {
                    assert!(
                        &header.sig != b"PX" && &header.sig != b"NM" && &header.sig != b"TF",
                        "RRIP entry {:?} should not be Unknown",
                        core::str::from_utf8(&header.sig)
                    );
                }
                _ => {}
            }
        }
    }

    assert!(found_px, "Should find typed PosixAttributes entries");
    assert!(found_nm, "Should find typed AlternateName entries");
    assert!(found_tf, "Should find typed Timestamps entries");
}
