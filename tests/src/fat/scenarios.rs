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
        ("nt-case-flags".into(), nt_case_flags()),
        ("case-only-rename".into(), case_only_rename()),
        ("lfn-unit-boundaries".into(), lfn_unit_boundaries()),
        ("non-bmp-names".into(), non_bmp_names()),
        ("numeric-tail-collisions".into(), numeric_tail_collisions()),
        ("slot-fragmentation".into(), slot_fragmentation()),
        ("deep-nesting".into(), deep_nesting()),
        ("directory-attributes".into(), directory_attributes()),
        (
            "cluster-boundary-appends".into(),
            cluster_boundary_appends(),
        ),
        ("stale-cluster-reuse".into(), stale_cluster_reuse()),
        ("short-basename-aliases".into(), short_basename_aliases()),
    ]
}

/// Clusters freed from a directory full of entries and from a file whose
/// contents look like directory entries are reused by new directories,
/// which must start out empty.
fn stale_cluster_reuse() -> Vec<Operation> {
    let mut operations = vec![create_dir("/Stale")];
    operations.extend((0..24).map(|index| {
        create(
            &format!("/Stale/stale entry {index:02}.txt"),
            vec![index as u8],
        )
    }));
    operations.extend((0..24).map(|index| delete(&format!("/Stale/stale entry {index:02}.txt"))));
    operations.extend([
        delete("/Stale"),
        create_dir("/Fresh"),
        create("/Fresh/only.txt", b"only".to_vec()),
        create("/fake entries.bin", fake_directory_entries(8192)),
        delete("/fake entries.bin"),
        create_dir("/Fresh/Reused"),
        create_dir("/Fresh/Reused/Again"),
        create("/refill.bin", fake_directory_entries(4096)),
        Operation::ReplaceFile {
            path: "/refill.bin".into(),
            data: Vec::new(),
        },
        create_dir("/Third"),
    ]);
    operations
}

fn fake_directory_entries(len: usize) -> Vec<u8> {
    let mut data = Vec::with_capacity(len);
    while data.len() < len {
        let mut entry = *b"FAKEFILETXT";
        entry[7] = b'A' + (data.len() / 32 % 26) as u8;
        data.extend_from_slice(&entry);
        data.push(0x20);
        data.extend_from_slice(&[0; 14]);
        data.extend_from_slice(&[0x02, 0x00]);
        data.extend_from_slice(&[0x00; 4]);
    }
    data.truncate(len);
    data
}

/// Basenames shorter than the six characters that precede a numeric tail,
/// whose lossy aliases collide with genuine 8.3 names.
fn short_basename_aliases() -> Vec<Operation> {
    [
        "x.txt",
        "x .txt",
        "x  .txt",
        "a",
        "a b",
        "a.b.c",
        "A.B.C.D",
        "ab.cd",
        "ab .cd",
        "a+b.txt",
        "a+c.txt",
        "a+d.txt",
        "a+e.txt",
        "a+f.txt",
        "a+g.txt",
        "a+h.txt",
        "a+i.txt",
        "a+j.txt",
        "a+k.txt",
        "a+l.txt",
        "\u{3c3}igma.txt",
        "\u{3c3}.txt",
        "..dots",
        ".a.b",
    ]
    .into_iter()
    .enumerate()
    .map(|(index, name)| create(&format!("/{name}"), vec![index as u8]))
    .collect()
}

fn create(path: &str, data: Vec<u8>) -> Operation {
    Operation::CreateFile {
        path: path.into(),
        data,
    }
}

fn create_dir(path: &str) -> Operation {
    Operation::CreateDir { path: path.into() }
}

fn rename(from: &str, to: &str) -> Operation {
    Operation::Rename {
        from: from.into(),
        to: to.into(),
    }
}

fn delete(path: &str) -> Operation {
    Operation::Delete { path: path.into() }
}

fn set_attrs(path: &str, attrs: u8) -> Operation {
    Operation::SetAttrs {
        path: path.into(),
        attrs,
    }
}

fn append(path: &str, data: Vec<u8>) -> Operation {
    Operation::AppendFile {
        path: path.into(),
        data,
    }
}

fn truncate(path: &str, len: usize) -> Operation {
    Operation::TruncateFile {
        path: path.into(),
        len,
    }
}

/// Names that fit 8.3 apart from case must round-trip through the NT
/// reserved-byte case flags or an LFN without changing how they display.
fn nt_case_flags() -> Vec<Operation> {
    [
        "ALPHA.txt",
        "beta.TXT",
        "Gamma.txt",
        "delta.txt",
        "EPSILON.TXT",
        "zeta.TxT",
        "eta",
        "THETA",
        "iota.x",
        "KAPPA.abc",
        "lambda1",
        "mu.1",
        "Nu",
        "xi.Yz",
    ]
    .into_iter()
    .enumerate()
    .map(|(index, name)| create(&format!("/{name}"), vec![index as u8]))
    .collect()
}

/// Renames that only change case: the entry must be replaced in place, not
/// rejected as a collision with itself or duplicated.
fn case_only_rename() -> Vec<Operation> {
    vec![
        create("/lower.txt", b"short alias".to_vec()),
        rename("/lower.txt", "/LOWER.TXT"),
        create("/Mixed Case Name.bin", payload(600, 0x61)),
        rename("/Mixed Case Name.bin", "/MIXED case NAME.bin"),
        create_dir("/Folder"),
        create("/Folder/inner.txt", b"child".to_vec()),
        rename("/Folder", "/FOLDER"),
        rename("/FOLDER", "/folder"),
        create_dir("/Long Directory Name"),
        rename("/Long Directory Name", "/long directory name"),
        rename("/MIXED case NAME.bin", "/mixed case name.BIN"),
    ]
}

/// Long names whose UTF-16 length lands on and around each 13-unit slot
/// boundary, plus the 255-unit maximum.
fn lfn_unit_boundaries() -> Vec<Operation> {
    let mut operations: Vec<Operation> = [9, 12, 13, 14, 25, 26, 27, 38, 39, 40, 254, 255]
        .into_iter()
        .enumerate()
        .map(|(index, len)| create(&format!("/{:0>len$}", index + 1), vec![index as u8]))
        .collect();
    operations.push(create(
        &format!("/{}.txt", "9".repeat(251)),
        b"max with extension".to_vec(),
    ));
    operations.push(create(
        &format!("/\u{1F600}{}", "e".repeat(253)),
        b"surrogate pair at the limit".to_vec(),
    ));
    operations.push(rename(&format!("/{:0>13}", 3), &format!("/{:0>26}", 3)));
    operations
}

/// Non-ASCII and supplementary-plane names, which need surrogate pairs in
/// LFN entries and lossy short aliases.
fn non_bmp_names() -> Vec<Operation> {
    vec![
        create("/emoji \u{1F600} name.txt", b"grinning".to_vec()),
        create(
            "/\u{1D518}\u{1D52B}\u{1D526}\u{1D520}\u{1D52C}\u{1D521}\u{1D522}.txt",
            b"fraktur".to_vec(),
        ),
        create("/caf\u{e9}.txt", b"latin-1 lowercase".to_vec()),
        create(
            "/\u{dc}n\u{ef}c\u{f6}d\u{e9}.txt",
            b"latin-1 mixed".to_vec(),
        ),
        create("/\u{3b1}\u{3b2}\u{3b3}.txt", b"greek".to_vec()),
        create("/\u{3a9}.TXT", b"single greek uppercase".to_vec()),
        create("/\u{1F600}", b"emoji only".to_vec()),
        create_dir("/\u{1F4C1} folder"),
        create(
            "/\u{1F4C1} folder/\u{1F600}\u{1F601}\u{1F602}.bin",
            payload(700, 0x71),
        ),
        rename(
            "/emoji \u{1F600} name.txt",
            "/\u{1F4C1} folder/renamed \u{1F642}.txt",
        ),
        rename("/\u{1F4C1} folder", "/\u{1F4C2} moved folder"),
        delete("/\u{1F600}"),
    ]
}

/// Many long names that share one generated alias prefix, an explicit 8.3
/// name that occupies the first numeric tail, and churn on the freed tails.
fn numeric_tail_collisions() -> Vec<Operation> {
    let mut operations = vec![create("/COLLIS~1.TXT", b"explicit tail".to_vec())];
    operations.extend((0..12).map(|index| {
        create(
            &format!("/collision candidate {index:02}.txt"),
            vec![index as u8],
        )
    }));
    operations.extend([
        delete("/collision candidate 03.txt"),
        delete("/collision candidate 07.txt"),
        create("/collision candidate 12.txt", vec![12]),
        rename(
            "/collision candidate 05.txt",
            "/collision candidate 05 renamed.txt",
        ),
        create_dir("/collision directory"),
        create("/collision directory/collision candidate 00.txt", vec![0]),
        rename(
            "/collision candidate 00.txt",
            "/collision directory/collision candidate 00 moved.txt",
        ),
        create("/collision candidate 03.txt", vec![3]),
    ]);
    operations
}

/// Deletes leave holes of assorted widths inside a subdirectory; later names
/// need runs wider than any hole, including one spanning cluster boundaries.
fn slot_fragmentation() -> Vec<Operation> {
    let mut operations = vec![create_dir("/Frag")];
    operations.extend((0..40).map(|index| {
        create(
            &format!("/Frag/three slot name {index:02}.txt"),
            vec![index as u8],
        )
    }));
    operations.extend(
        (0..40)
            .step_by(2)
            .map(|index| delete(&format!("/Frag/three slot name {index:02}.txt"))),
    );
    operations.extend((0..20).map(|index| {
        create(
            &format!("/Frag/five slot name that is much longer {index:02}.txt"),
            vec![index as u8],
        )
    }));
    operations.extend(
        (1..40)
            .step_by(2)
            .map(|index| delete(&format!("/Frag/three slot name {index:02}.txt"))),
    );
    operations.push(create(
        &format!("/Frag/{}", "z".repeat(255)),
        b"twenty slots".to_vec(),
    ));
    operations.push(create("/Frag/short.txt", b"one slot".to_vec()));
    operations.push(delete(&format!("/Frag/{}", "z".repeat(255))));
    operations.push(create(
        &format!("/Frag/{}", "y".repeat(255)),
        b"twenty slots again".to_vec(),
    ));
    operations
}

/// Twelve levels of long directory names, a file at the bottom, and moves
/// of both the top and the deepest directory.
fn deep_nesting() -> Vec<Operation> {
    let mut path = String::new();
    let mut operations = Vec::new();
    for level in 0..12 {
        path.push_str(&format!("/Level {level:02} Directory"));
        operations.push(create_dir(&path));
    }
    operations.push(create(
        &format!("{path}/bottom file.txt"),
        payload(513, 0x81),
    ));
    let parent = path.rsplit_once('/').unwrap().0.to_string();
    operations.push(rename(&path, &format!("{parent}/Deepest Renamed")));
    operations.push(rename(
        &format!("{parent}/Deepest Renamed"),
        "/Deepest Moved To Root",
    ));
    operations.push(rename("/Level 00 Directory", "/Top Level Renamed"));
    operations.push(delete("/Deepest Moved To Root/bottom file.txt"));
    operations.push(delete("/Deepest Moved To Root"));
    operations
}

/// Attributes on directories and read-only files, which must survive
/// renames, moves, and later attribute changes.
fn directory_attributes() -> Vec<Operation> {
    vec![
        create_dir("/Hidden Dir"),
        set_attrs("/Hidden Dir", HIDDEN | SYSTEM),
        create("/Hidden Dir/inner.txt", b"inside hidden".to_vec()),
        rename("/Hidden Dir", "/Hidden Dir Renamed"),
        create_dir("/Hidden Dir Renamed/Child"),
        rename("/Hidden Dir Renamed/Child", "/Child Moved Out"),
        create("/locked.txt", b"read only".to_vec()),
        set_attrs("/locked.txt", ARCHIVE | READ_ONLY),
        rename("/locked.txt", "/Hidden Dir Renamed/locked renamed.txt"),
        set_attrs("/Hidden Dir Renamed/locked renamed.txt", ARCHIVE),
        set_attrs("/Hidden Dir Renamed", 0),
        delete("/Hidden Dir Renamed/locked renamed.txt"),
        create_dir("/Plain"),
        set_attrs("/Plain", READ_ONLY | HIDDEN | SYSTEM | ARCHIVE),
    ]
}

/// Appends and truncations landing exactly on multiples of every cluster
/// size the suite formats with.
fn cluster_boundary_appends() -> Vec<Operation> {
    vec![
        create("/exact.bin", payload(4096, 0x91)),
        append("/exact.bin", vec![0xaa]),
        create("/almost.bin", payload(4095, 0x92)),
        append("/almost.bin", vec![0xbb]),
        truncate("/exact.bin", 4096),
        append("/exact.bin", payload(4096, 0x93)),
        truncate("/exact.bin", 0),
        append("/exact.bin", payload(4097, 0x94)),
        Operation::ReplaceFile {
            path: "/almost.bin".into(),
            data: Vec::new(),
        },
        append("/almost.bin", vec![0xcc]),
        Operation::ReplaceFile {
            path: "/almost.bin".into(),
            data: payload(8192, 0x95),
        },
        truncate("/almost.bin", 4097),
        truncate("/almost.bin", 4095),
        append("/almost.bin", payload(2, 0x96)),
    ]
}

/// An operation every implementation must refuse, with the setup that makes
/// it invalid. The image must be unchanged and spec-valid afterwards.
pub struct RejectionScenario {
    pub name: String,
    pub setup: Vec<Operation>,
    pub rejected: Operation,
}

pub fn rejection_scenarios() -> Vec<RejectionScenario> {
    let mut scenarios = vec![
        RejectionScenario {
            name: "duplicate-short-name-case".into(),
            setup: vec![create("/README.TXT", b"upper".to_vec())],
            rejected: create("/readme.txt", b"lower".to_vec()),
        },
        RejectionScenario {
            name: "duplicate-long-name-case".into(),
            setup: vec![create("/Mixed Case Name.bin", b"first".to_vec())],
            rejected: create("/MIXED CASE NAME.BIN", b"second".to_vec()),
        },
        RejectionScenario {
            name: "duplicate-long-directory-case".into(),
            setup: vec![create_dir("/Long Directory Name")],
            rejected: create_dir("/long directory name"),
        },
        RejectionScenario {
            name: "file-over-directory".into(),
            setup: vec![create_dir("/Shared")],
            rejected: create("/shared", b"clash".to_vec()),
        },
        RejectionScenario {
            name: "directory-over-file".into(),
            setup: vec![create("/shared.txt", b"clash".to_vec())],
            rejected: create_dir("/SHARED.TXT"),
        },
        RejectionScenario {
            name: "missing-parent".into(),
            setup: Vec::new(),
            rejected: create("/Missing/file.txt", b"orphan".to_vec()),
        },
        RejectionScenario {
            name: "parent-is-a-file".into(),
            setup: vec![create("/file.txt", b"not a dir".to_vec())],
            rejected: create("/file.txt/child.txt", b"orphan".to_vec()),
        },
        RejectionScenario {
            name: "delete-non-empty-directory".into(),
            setup: vec![create_dir("/Full"), create("/Full/a.txt", b"a".to_vec())],
            rejected: delete("/Full"),
        },
        RejectionScenario {
            name: "delete-non-empty-nested-directory".into(),
            setup: vec![create_dir("/Outer"), create_dir("/Outer/Inner")],
            rejected: delete("/Outer"),
        },
        RejectionScenario {
            name: "rename-onto-existing-file".into(),
            setup: vec![
                create("/a.txt", b"a".to_vec()),
                create("/b.txt", b"b".to_vec()),
            ],
            rejected: rename("/a.txt", "/b.txt"),
        },
        RejectionScenario {
            name: "rename-onto-existing-case".into(),
            setup: vec![
                create("/Alpha Name.txt", b"a".to_vec()),
                create("/beta name.txt", b"b".to_vec()),
            ],
            rejected: rename("/Alpha Name.txt", "/BETA NAME.TXT"),
        },
        RejectionScenario {
            name: "rename-onto-existing-directory".into(),
            setup: vec![create_dir("/Source"), create_dir("/Target")],
            rejected: rename("/Source", "/Target"),
        },
        RejectionScenario {
            name: "rename-into-own-subtree".into(),
            setup: vec![create_dir("/A"), create_dir("/A/B")],
            rejected: rename("/A", "/A/B/A"),
        },
        RejectionScenario {
            name: "rename-into-itself".into(),
            setup: vec![create_dir("/A")],
            rejected: rename("/A", "/A/A"),
        },
        RejectionScenario {
            name: "rename-missing-source".into(),
            setup: vec![create_dir("/Dest")],
            rejected: rename("/nothing.txt", "/Dest/nothing.txt"),
        },
        RejectionScenario {
            name: "name-too-long".into(),
            setup: Vec::new(),
            rejected: create(&format!("/{}.txt", "a".repeat(252)), b"256".to_vec()),
        },
        RejectionScenario {
            name: "name-too-long-surrogate".into(),
            setup: Vec::new(),
            rejected: create(
                &format!("/\u{1F600}{}", "e".repeat(254)),
                b"256 units".to_vec(),
            ),
        },
        RejectionScenario {
            name: "dot-file-in-root".into(),
            setup: Vec::new(),
            rejected: create("/.", b"dot".to_vec()),
        },
        RejectionScenario {
            name: "dotdot-file-in-root".into(),
            setup: Vec::new(),
            rejected: create("/..", b"dotdot".to_vec()),
        },
        RejectionScenario {
            name: "dot-file-in-subdirectory".into(),
            setup: vec![create_dir("/Nested")],
            rejected: create("/Nested/.", b"dot".to_vec()),
        },
        RejectionScenario {
            name: "dotdot-directory-in-subdirectory".into(),
            setup: vec![create_dir("/Nested")],
            rejected: create_dir("/Nested/.."),
        },
    ];
    for (label, character) in [
        ("colon", ':'),
        ("asterisk", '*'),
        ("question", '?'),
        ("quote", '"'),
        ("less-than", '<'),
        ("greater-than", '>'),
        ("pipe", '|'),
        ("control", '\u{1}'),
    ] {
        scenarios.push(RejectionScenario {
            name: format!("invalid-character-{label}"),
            setup: Vec::new(),
            rejected: create(&format!("/bad{character}name.txt"), b"invalid".to_vec()),
        });
    }
    scenarios
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
