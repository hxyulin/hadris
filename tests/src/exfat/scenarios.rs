//! exFAT scenarios on top of the shared FAT traces: names only exFAT allows,
//! Unicode case rules of the up-case table, entry sets that cross clusters
//! and directories that grow, and the operations exFAT must refuse.

use crate::fat::model::Operation;
use crate::fat::scenarios::{RejectionScenario, payload, rejection_scenarios};

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

/// Names whose sets take one to nineteen entries, in a directory that
/// grows past several clusters of the smallest cluster size, with deletes
/// that leave holes the later sets must fit or skip.
fn entry_sets_across_clusters() -> Vec<Operation> {
    let mut operations = vec![create_dir("/Sets")];
    for index in 0..48 {
        let len = 1 + index * 5 % 250;
        operations.push(create(
            &format!("/Sets/{index:02}{}", "s".repeat(len)),
            payload(index * 37, index as u8),
        ));
    }
    for index in (0..48).step_by(3) {
        let len = 1 + index * 5 % 250;
        operations.push(delete(&format!("/Sets/{index:02}{}", "s".repeat(len))));
    }
    for index in 48..64 {
        operations.push(create(
            &format!("/Sets/{index:02} refill {}", "r".repeat(index * 3)),
            vec![index as u8],
        ));
    }
    operations.push(rename(
        &format!("/Sets/01{}", "s".repeat(6)),
        &format!("/Sets/moved and longer {}", "m".repeat(200)),
    ));
    operations
}

/// Characters FAT short names cannot hold but exFAT names can, and names
/// that differ only in case under the up-case table.
fn exfat_names() -> Vec<Operation> {
    vec![
        create("/plus+comma,semi;eq=[br].txt", b"allowed".to_vec()),
        create("/ leading space", b"leading space".to_vec()),
        create("/Stra\u{df}e", b"sharp s has no single uppercase".to_vec()),
        create("/\u{3a3}\u{3b9}\u{3b3}\u{3bc}\u{3b1}", b"greek".to_vec()),
        rename(
            "/\u{3a3}\u{3b9}\u{3b3}\u{3bc}\u{3b1}",
            "/\u{3c3}\u{399}\u{393}\u{39c}\u{391}",
        ),
        create("/\u{430}\u{431}\u{432}", b"cyrillic".to_vec()),
        create("/\u{ff41}\u{ff42}", b"fullwidth".to_vec()),
        create("/\u{1f600} emoji", b"surrogates".to_vec()),
        create_dir("/Caf\u{e9}"),
        rename("/Caf\u{e9}", "/CAF\u{c9}"),
        create("/CAF\u{c9}/inner", b"inside".to_vec()),
    ]
}

/// Files that grow and shrink across cluster boundaries of every case,
/// with gaps that the valid data length must read as zeros.
fn sparse_growth() -> Vec<Operation> {
    vec![
        create("/grow.bin", payload(100, 1)),
        Operation::AppendFile {
            path: "/grow.bin".into(),
            data: payload(40_000, 2),
        },
        Operation::TruncateFile {
            path: "/grow.bin".into(),
            len: 33_000,
        },
        Operation::AppendFile {
            path: "/grow.bin".into(),
            data: payload(512, 3),
        },
        Operation::TruncateFile {
            path: "/grow.bin".into(),
            len: 0,
        },
        Operation::AppendFile {
            path: "/grow.bin".into(),
            data: payload(32_768, 4),
        },
    ]
}

/// The exFAT scenarios, to run beside the shared FAT ones.
pub fn exfat_scenarios() -> Vec<(String, Vec<Operation>)> {
    vec![
        (
            "entry-sets-across-clusters".into(),
            entry_sets_across_clusters(),
        ),
        ("exfat-names".into(), exfat_names()),
        ("sparse-growth".into(), sparse_growth()),
    ]
}

/// The shared rejections plus exFAT's own: names that end in a dot or a
/// space, a backslash, and Unicode case twins.
pub fn exfat_rejection_scenarios() -> Vec<RejectionScenario> {
    let mut scenarios = rejection_scenarios();
    scenarios.extend([
        RejectionScenario {
            name: "trailing-dot".into(),
            setup: Vec::new(),
            rejected: create("/name.", b"dot".to_vec()),
        },
        RejectionScenario {
            name: "trailing-space".into(),
            setup: Vec::new(),
            rejected: create("/name ", b"space".to_vec()),
        },
        RejectionScenario {
            name: "invalid-character-backslash".into(),
            setup: Vec::new(),
            rejected: create("/back\\slash", b"invalid".to_vec()),
        },
        RejectionScenario {
            name: "greek-case-twin".into(),
            setup: vec![create("/\u{3a9}mega", b"upper".to_vec())],
            rejected: create("/\u{3c9}MEGA", b"lower".to_vec()),
        },
        RejectionScenario {
            name: "cyrillic-directory-case-twin".into(),
            setup: vec![create_dir("/\u{414}\u{438}\u{440}")],
            rejected: create_dir("/\u{434}\u{418}\u{420}"),
        },
        RejectionScenario {
            name: "fullwidth-rename-case-twin".into(),
            setup: vec![
                create("/\u{ff21}", b"a".to_vec()),
                create("/other", b"b".to_vec()),
            ],
            rejected: rename("/other", "/\u{ff41}"),
        },
    ]);
    scenarios
}
