#![no_main]
//! Fuzz the exFAT reader: mount an arbitrary image with `ExFatFs`, walk every
//! directory, read every file and run `check_with`. Arbitrary bytes must
//! never panic, abort or OOM.
//!
//! Self-consistency oracles (failures are tagged `ORACLE:`): every file is
//! read twice and the bytes must match, and listed entries must re-resolve
//! by name through `lookup` (guarded against names that the fuzz-controlled
//! up-case table aliases).

use std::collections::HashSet;

use hadris_fat::exfat::sync::{check_with, ExFatFs};
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

    let clusters = fs.stats().map(|stats| stats.total_blocks()).unwrap_or(0) + 2;
    let mut bitmap = vec![0u8; clusters.div_ceil(8).clamp(1, MAX_BITMAP) as usize];
    let _ = check_with(&mut fs, &mut bitmap, |_| {});
}

fuzz_target!(|data: &[u8]| {
    drive(data);
});
