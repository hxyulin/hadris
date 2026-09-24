#![no_main]
//! Fuzz the exFAT reader: run `check` on an arbitrary image, mount it with
//! `ExFatFs`, walk every directory and read every file. Arbitrary bytes must
//! never panic, abort or OOM. The main boot checksum is recomputed before
//! mounting, so mutations reach past the boot region.
//!
//! Self-consistency oracles (failures are tagged `ORACLE:`): every file is
//! read twice and the bytes must match, and listed entries must re-resolve
//! by name through `lookup` (guarded against names that the fuzz-controlled
//! up-case table aliases).

use std::collections::HashSet;

use hadris_fat::exfat::sync::{check, ExFatFs};
use hadris_fat::exfat::MountOptions;
use hadris_fs::{DirCursor, FileType, HeapTable, NameBuf, NodeId};
use hadris_storage::{BlockSize, MemDevice};
use libfuzzer_sys::fuzz_target;

type Fs<'a> = ExFatFs<MemDevice<&'a [u8]>, HeapTable>;

/// `lookup` re-scans a directory from the start, so cap name re-resolution
/// lookups per directory to keep the walk from going quadratic under the
/// flat work budget.
const MAX_LOOKUPS_PER_DIR: usize = 32;
const READ_CAP: usize = 4 * 1024 * 1024;
const MAX_BITMAP: u64 = 1 << 20;
/// Inputs are zero-extended to the size their boot sector declares, up to
/// this, so short inputs still mount: `ExFatFs` refuses a volume larger than
/// its device.
const MAX_IMAGE: usize = 8 * 1024 * 1024;

/// The volume size the boot sector declares, in bytes.
fn declared_len(data: &[u8]) -> usize {
    let Some(boot) = data.get(..112) else {
        return 0;
    };
    let sectors = u64::from_le_bytes(boot[72..80].try_into().unwrap());
    let shift = u32::from(boot[108]).min(12);
    usize::try_from(sectors.saturating_mul(1 << shift)).unwrap_or(usize::MAX)
}

/// Stores the checksum of the main boot region, so the mount does not stop
/// at a checksum the fuzzer cannot guess.
fn seal_boot(image: &mut [u8]) {
    let Some(&shift) = image.get(108) else {
        return;
    };
    let sector = 1usize << shift.clamp(9, 12);
    let Some(region) = image.get_mut(..12 * sector) else {
        return;
    };
    let mut sum = 0u32;
    for (at, &byte) in region[..11 * sector].iter().enumerate() {
        if !matches!(at, 106 | 107 | 112) {
            sum = sum.rotate_right(1).wrapping_add(byte as u32);
        }
    }
    for word in region[11 * sector..].chunks_exact_mut(4) {
        word.copy_from_slice(&sum.to_le_bytes());
    }
}

/// Reads a file in chunks with a byte cap: the size is fuzz-controlled and a
/// corrupt FAT can serve the same clusters over and over. Returns the bytes
/// and whether the read ended in an error.
fn read_pass(fs: &mut Fs<'_>, node: NodeId) -> (Vec<u8>, bool) {
    let mut buf = [0u8; 64 * 1024];
    let mut out = Vec::new();
    loop {
        match fs.read_at(node, out.len() as u64, &mut buf) {
            Ok(0) => return (out, false),
            Err(_) => return (out, true),
            Ok(n) => {
                out.extend_from_slice(&buf[..n]);
                if out.len() >= READ_CAP {
                    return (out, false);
                }
            }
        }
    }
}

fn drive(data: &[u8]) {
    let mut image = data.to_vec();
    let len = declared_len(data).min(MAX_IMAGE);
    if image.len() < len {
        image.resize(len, 0);
    }
    image.resize(image.len().next_multiple_of(512), 0);
    seal_boot(&mut image);
    let clusters = (image.len() / 512) as u64;
    let mut scratch = vec![0u8; 1024 + clusters.div_ceil(8).clamp(512, MAX_BITMAP) as usize];
    let mut findings = 0u64;
    let report = check(
        &mut MemDevice::new(&image[..], BlockSize::new(512).unwrap()),
        &mut scratch,
        |f| {
            findings += 1;
            let _ = f.to_string();
        },
    );
    if let Ok(report) = report {
        assert_eq!(
            report.findings(),
            findings,
            "ORACLE: the report counts every finding"
        );
    }
    let dev = MemDevice::new(&image[..], BlockSize::new(512).unwrap());
    let options = MountOptions::new()
        .with_read_only()
        .with_table(HeapTable::new());
    let Ok(mut fs) = ExFatFs::open_with(dev, options) else {
        return;
    };
    let _ = fs.label();
    let _ = fs.stats();

    // Depth-guarded worklist with a flat work budget: a corrupt directory
    // graph has a path count that grows like branching^depth, so bound the
    // total entries processed on any input.
    let mut budget: u32 = 200_000;
    let mut stack = vec![(fs.root(), 0u32)];
    'walk: while let Some((dir, depth)) = stack.pop() {
        if depth > 64 {
            continue;
        }
        let mut lookups = 0usize;
        let mut seen_names: HashSet<String> = HashSet::new();
        let mut cursor = DirCursor::start();
        let mut name = NameBuf::new();
        loop {
            if budget == 0 {
                break 'walk;
            }
            budget -= 1;
            let entry = match fs.read_dir_entry(dir, &mut cursor, &mut name) {
                Ok(Some(entry)) => entry,
                Ok(None) | Err(_) => break,
            };
            let node = entry.node();
            let Some(child_name) = name.as_name() else {
                continue;
            };
            let Ok(text) = child_name.to_str().map(str::to_owned) else {
                continue;
            };
            let is_new_name = seen_names.insert(text.clone());

            // Name re-resolution oracle. Only sound when the name
            // round-trips through UTF-8.
            if lookups < MAX_LOOKUPS_PER_DIR && !text.contains('\u{FFFD}') {
                lookups += 1;
                let Ok(found) = fs.lookup(dir, child_name) else {
                    panic!("ORACLE: lookup({text:?}) failed to re-resolve a listed entry");
                };
                if is_new_name && found == node {
                    assert!(
                        fs.node_metadata(found).is_ok(),
                        "ORACLE: lookup({text:?}) returned a node without metadata"
                    );
                }
                fs.forget(found);
            }

            if entry.file_type() == FileType::Dir {
                let _ = fs.parent(node).map(|parent| fs.forget(parent));
                stack.push((node, depth + 1));
                continue;
            }
            let first = read_pass(&mut fs, node);
            let second = read_pass(&mut fs, node);
            assert_eq!(
                first.1, second.1,
                "ORACLE: repeated reads of {text:?} disagree on success"
            );
            assert!(
                first.0 == second.0,
                "ORACLE: repeated reads of {text:?} returned different bytes"
            );
            let _ = fs.cluster_chain(node, |_| {});
        }
    }
}

fuzz_target!(|data: &[u8]| {
    drive(data);
});
