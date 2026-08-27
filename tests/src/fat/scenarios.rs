//! Curated, edge-case, and seeded operation traces shared by every FAT suite.

use super::model::{FsState, Operation};
use super::{ARCHIVE, HIDDEN, READ_ONLY, SYSTEM};
use crate::harness::tree::EntryData;
use crate::harness::{Rng, join_path, path_depth};

/// Set to a `u64` to replay a single generated trace.
pub const SEED_ENV: &str = "HADRIS_TESTS_SEED";
const TRACE_LEN: usize = 64;

pub fn payload(len: usize, salt: u8) -> Vec<u8> {
    (0..len)
        .map(|index| (index as u8).wrapping_mul(31).wrapping_add(salt))
        .collect()
}

pub fn curated_operations() -> Vec<Operation> {
    vec![
        Operation::CreateDir {
            path: "/Empty".into(),
        },
        Operation::CreateDir {
            path: "/Nested".into(),
        },
        Operation::CreateDir {
            path: "/Nested/Deep".into(),
        },
        Operation::CreateFile {
            path: "/README.TXT".into(),
            data: Vec::new(),
        },
        Operation::CreateFile {
            path: "/lower.txt".into(),
            data: b"lowercase short name".to_vec(),
        },
        Operation::CreateFile {
            path: "/.hidden".into(),
            data: b"leading dot filename".to_vec(),
        },
        Operation::CreateFile {
            path: "/Mixed Case Name.bin".into(),
            data: payload(513, 0x11),
        },
        Operation::CreateFile {
            path: "/Nested/boundary.bin".into(),
            data: payload(4097, 0x22),
        },
        Operation::CreateFile {
            path: "/Nested/Deep/日本語.txt".into(),
            data: "Unicode filename contents\n".as_bytes().to_vec(),
        },
        Operation::AppendFile {
            path: "/lower.txt".into(),
            data: b" appended".to_vec(),
        },
        Operation::ReplaceFile {
            path: "/Mixed Case Name.bin".into(),
            data: payload(8193, 0x33),
        },
        Operation::TruncateFile {
            path: "/Nested/boundary.bin".into(),
            len: 512,
        },
        Operation::Rename {
            from: "/Nested/Deep/日本語.txt".into(),
            to: "/Nested/資料 renamed.txt".into(),
        },
        Operation::CreateFile {
            path: "/Delete Me.txt".into(),
            data: payload(31, 0x44),
        },
        Operation::Delete {
            path: "/Delete Me.txt".into(),
        },
        Operation::CreateFile {
            path: "/Slot Reuse Long Name.txt".into(),
            data: vec![0, 1, 0, 2, 0, 3],
        },
        Operation::SetAttrs {
            path: "/lower.txt".into(),
            attrs: ARCHIVE | READ_ONLY | HIDDEN,
        },
        Operation::SetAttrs {
            path: "/Slot Reuse Long Name.txt".into(),
            attrs: ARCHIVE | SYSTEM,
        },
        Operation::CreateDir {
            path: "/Move Source".into(),
        },
        Operation::Rename {
            from: "/Move Source".into(),
            to: "/Nested/Moved Empty Directory".into(),
        },
    ]
}

pub fn dot_entry_operations() -> Vec<Operation> {
    vec![
        Operation::CreateDir {
            path: "/Empty".into(),
        },
        Operation::CreateDir {
            path: "/Nested".into(),
        },
        Operation::CreateDir {
            path: "/Nested/Deep".into(),
        },
    ]
}

pub fn edge_case_scenarios() -> Vec<(String, Vec<Operation>)> {
    let deleted_slot_lfn_expansion = vec![
        Operation::CreateDir {
            path: "/D000".into(),
        },
        Operation::CreateDir {
            path: "/TEMP".into(),
        },
        Operation::Delete {
            path: "/TEMP".into(),
        },
        Operation::Rename {
            from: "/D000".into(),
            to: "/Renamed Directory 0009".into(),
        },
    ];

    let mut subdirectory_entry_boundary = vec![Operation::CreateDir {
        path: "/Entries".into(),
    }];
    subdirectory_entry_boundary.extend((0..15).map(|index| Operation::CreateFile {
        path: format!("/Entries/F{index:02}.TXT"),
        data: vec![index as u8],
    }));
    subdirectory_entry_boundary.extend([
        Operation::Delete {
            path: "/Entries/F04.TXT".into(),
        },
        Operation::Delete {
            path: "/Entries/F05.TXT".into(),
        },
        Operation::CreateFile {
            path: "/Entries/Long Name Reusing Adjacent Slots.txt".into(),
            data: payload(513, 0x41),
        },
    ]);

    let short_alias_collisions = ["x+.txt", "x,.txt", "x=.txt", "x;.txt", "x'.txt", "x].txt"]
        .into_iter()
        .enumerate()
        .map(|(index, name)| Operation::CreateFile {
            path: format!("/{name}"),
            data: vec![index as u8],
        })
        .collect();

    let directory_moves = vec![
        Operation::CreateDir {
            path: "/Parent A".into(),
        },
        Operation::CreateDir {
            path: "/Parent B".into(),
        },
        Operation::CreateDir {
            path: "/Parent A/Child".into(),
        },
        Operation::CreateDir {
            path: "/Parent A/Child/Deep".into(),
        },
        Operation::Rename {
            from: "/Parent A/Child".into(),
            to: "/Parent B/Moved Child".into(),
        },
        Operation::Rename {
            from: "/Parent B/Moved Child".into(),
            to: "/Moved Again".into(),
        },
    ];

    let truncate_reallocate = vec![
        Operation::CreateFile {
            path: "/CHAIN.BIN".into(),
            data: payload(16_385, 0x52),
        },
        Operation::TruncateFile {
            path: "/CHAIN.BIN".into(),
            len: 8_192,
        },
        Operation::TruncateFile {
            path: "/CHAIN.BIN".into(),
            len: 4_097,
        },
        Operation::TruncateFile {
            path: "/CHAIN.BIN".into(),
            len: 4_096,
        },
        Operation::TruncateFile {
            path: "/CHAIN.BIN".into(),
            len: 0,
        },
        Operation::AppendFile {
            path: "/CHAIN.BIN".into(),
            data: payload(513, 0x53),
        },
    ];

    let lfn_boundaries = [
        "123456789.txt",
        "1234567890.txt",
        "1234567890123456789012.txt",
        "12345678901234567890123.txt",
        "日本語.txt",
    ]
    .into_iter()
    .enumerate()
    .map(|(index, name)| Operation::CreateFile {
        path: format!("/{name}"),
        data: vec![index as u8],
    })
    .collect();

    vec![
        (
            "deleted-slot-lfn-expansion".into(),
            deleted_slot_lfn_expansion,
        ),
        (
            "subdirectory-entry-boundary".into(),
            subdirectory_entry_boundary,
        ),
        ("short-alias-collisions".into(), short_alias_collisions),
        ("directory-moves".into(), directory_moves),
        ("truncate-reallocate".into(), truncate_reallocate),
        ("lfn-boundaries".into(), lfn_boundaries),
    ]
}

pub fn generate_trace(seed: u64) -> Vec<Operation> {
    let mut rng = Rng::new(seed);
    let mut model = FsState::empty();
    let mut operations = Vec::with_capacity(TRACE_LEN);
    for index in 0..TRACE_LEN {
        let dirs = model.directories();
        let writable_files = model.files(true);
        let empty_dirs = model.empty_directories();
        let can_create = model.entries.len() < 48;
        let roll = rng.index(100);
        let operation = if can_create && (model.entries.len() < 4 || roll < 22) {
            let parents: Vec<_> = dirs
                .iter()
                .filter(|path| path_depth(path) < 4)
                .cloned()
                .collect();
            let parent = &parents[rng.index(parents.len())];
            Operation::CreateDir {
                path: join_path(parent, &generated_name(&mut rng, index, true)),
            }
        } else if can_create && (writable_files.is_empty() || roll < 45) {
            let parents: Vec<_> = dirs
                .iter()
                .filter(|path| path_depth(path) < 4)
                .cloned()
                .collect();
            let parent = &parents[rng.index(parents.len())];
            Operation::CreateFile {
                path: join_path(parent, &generated_name(&mut rng, index, false)),
                data: generated_payload(&mut rng),
            }
        } else if !writable_files.is_empty() && roll < 58 {
            Operation::ReplaceFile {
                path: writable_files[rng.index(writable_files.len())].clone(),
                data: generated_payload(&mut rng),
            }
        } else if !writable_files.is_empty() && roll < 68 {
            Operation::AppendFile {
                path: writable_files[rng.index(writable_files.len())].clone(),
                data: payload([1, 31, 512, 513][rng.index(4)], rng.next_u64() as u8),
            }
        } else if !writable_files.is_empty() && roll < 76 {
            let path = writable_files[rng.index(writable_files.len())].clone();
            let len = match &model.entries[&path].data {
                EntryData::File(data) => rng.index(data.len() + 1),
                EntryData::Directory => unreachable!(),
            };
            Operation::TruncateFile { path, len }
        } else if !model.entries.is_empty() && roll < 87 {
            generate_rename(&mut rng, &model, index).unwrap_or_else(|| {
                let parent = &dirs[rng.index(dirs.len())];
                Operation::CreateFile {
                    path: join_path(parent, &generated_name(&mut rng, index, false)),
                    data: generated_payload(&mut rng),
                }
            })
        } else if !writable_files.is_empty() && roll < 94 {
            Operation::SetAttrs {
                path: writable_files[rng.index(writable_files.len())].clone(),
                attrs: ARCHIVE
                    | if rng.next_u64() & 1 == 0 { HIDDEN } else { 0 }
                    | if rng.next_u64() & 1 == 0 { SYSTEM } else { 0 }
                    | if rng.next_u64() % 5 == 0 {
                        READ_ONLY
                    } else {
                        0
                    },
            }
        } else if !writable_files.is_empty() {
            Operation::Delete {
                path: writable_files[rng.index(writable_files.len())].clone(),
            }
        } else if !empty_dirs.is_empty() {
            Operation::Delete {
                path: empty_dirs[rng.index(empty_dirs.len())].clone(),
            }
        } else {
            let parent = &dirs[rng.index(dirs.len())];
            Operation::CreateFile {
                path: join_path(parent, &generated_name(&mut rng, index, false)),
                data: generated_payload(&mut rng),
            }
        };
        model
            .apply(&operation)
            .expect("generated operation is valid");
        operations.push(operation);
    }
    operations
}

fn generate_rename(rng: &mut Rng, model: &FsState, index: usize) -> Option<Operation> {
    let sources: Vec<_> = model
        .entries
        .iter()
        .filter(|(_, entry)| entry.attrs & READ_ONLY == 0)
        .map(|(path, _)| path.clone())
        .collect();
    if sources.is_empty() {
        return None;
    }
    let from = sources[rng.index(sources.len())].clone();
    let source_is_dir = model.entries[&from].data.is_directory();
    let prefix = format!("{from}/");
    let destinations: Vec<_> = model
        .directories()
        .into_iter()
        .filter(|path| path != &from && !path.starts_with(&prefix) && path_depth(path) < 4)
        .collect();
    if destinations.is_empty() {
        return None;
    }
    let parent = &destinations[rng.index(destinations.len())];
    Some(Operation::Rename {
        from,
        to: join_path(
            parent,
            &if source_is_dir {
                format!("Renamed Directory {index:04}")
            } else {
                format!("Renamed File {index:04}.bin")
            },
        ),
    })
}

fn generated_name(rng: &mut Rng, index: usize, directory: bool) -> String {
    let suffix = if directory { "" } else { ".bin" };
    match rng.index(4) {
        0 => format!("{}{:03}{suffix}", if directory { "D" } else { "F" }, index),
        1 => format!(
            "{}{:03}{suffix}",
            if directory { "dir" } else { "file" },
            index
        ),
        2 => format!(
            "{} {index:03} Name{suffix}",
            if directory { "Directory" } else { "Mixed" }
        ),
        _ => format!("資料{index:03}{suffix}"),
    }
}

fn generated_payload(rng: &mut Rng) -> Vec<u8> {
    let lengths = [0, 1, 31, 511, 512, 513, 4095, 4096, 4097, 8193, 32769];
    payload(lengths[rng.index(lengths.len())], rng.next_u64() as u8)
}

pub fn selected_seeds() -> Vec<u64> {
    match std::env::var(SEED_ENV) {
        Ok(seed) => vec![seed.parse::<u64>().expect("seed must be a u64")],
        Err(_) => vec![
            0x0000_0000_0000_0001,
            0x243f_6a88_85a3_08d3,
            0x1319_8a2e_0370_7344,
            0xa409_3822_299f_31d0,
            0x082e_fa98_ec4e_6c89,
            0x4528_21e6_38d0_1377,
            0xbe54_66cf_34e9_0c6c,
            0xc0ac_29b7_c97c_50dd,
            0x3f84_d5b5_b547_0917,
            0x9216_d5d9_8979_fb1b,
            0xd131_0ba6_98df_b5ac,
            0x2ffd_72db_d01a_dfb7,
            0xb8e1_afed_6a26_7e96,
            0xba7c_9045_f12c_7f99,
            0x24a1_9947_b391_6cf7,
            0x0801_f2e2_858e_fc16,
        ],
    }
}

/// Short, isolated scenarios used to score peers so one failure does not
/// mask the rest of a long trace.
pub fn interoperability_scenarios() -> Vec<(String, Vec<Operation>)> {
    let mut scenarios = vec![("curated".to_string(), curated_operations())];
    scenarios.extend(edge_case_scenarios());
    scenarios
}

/// The interoperability scenarios plus the seeded generated traces.
pub fn specification_scenarios() -> Vec<(String, Vec<Operation>)> {
    let mut scenarios = interoperability_scenarios();
    scenarios.extend(
        selected_seeds()
            .into_iter()
            .map(|seed| (format!("seed-{seed:016x}"), generate_trace(seed))),
    );
    scenarios
}
