#![no_main]
//! Fuzz the UDF reader: open an arbitrary image and walk every directory,
//! reading every file and link and the metadata, parent and extents of
//! every node. Arbitrary bytes must never panic, abort or OOM.
//!
//! Self-consistency oracles (failures are tagged `ORACLE:`): every file is
//! read twice and the bytes must match, and listed entries must re-resolve
//! by name through `lookup` to the id the listing gave.

use hadris_fs::MountOptions;
use std::collections::HashSet;

use hadris_fs::sync::FileSystem;
use hadris_fs::{DirCursor, FileType, NodeId};
use hadris_storage::{BlockSize, MemDevice};
use hadris_udf::sync::UdfFs;
use libfuzzer_sys::fuzz_target;

type Fs = UdfFs<MemDevice<Vec<u8>>>;

/// `lookup` re-scans a directory from the start, so cap name re-resolution
/// lookups per directory to keep the walk from going quadratic under the
/// flat work budget.
const MAX_LOOKUPS_PER_DIR: usize = 32;
const READ_CAP: usize = 4 * 1024 * 1024;

/// Reads a file in chunks with a byte cap: the size is fuzz-controlled.
/// Returns the bytes and whether the read ended in an error.
fn read_pass(fs: &mut Fs, node: NodeId) -> (Vec<u8>, bool) {
    let mut buf = [0u8; 64 * 1024];
    let mut out = Vec::new();
    loop {
        match fs.read(node, out.len() as u64, &mut buf) {
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

/// Walks the tree. A corrupt directory graph (identifiers pointing at
/// sibling or ancestor directories) has a path count that grows like
/// branching^depth, so a flat work budget bounds the entries processed.
fn walk(fs: &mut Fs) {
    let _ = fs.statfs();
    let _ = fs.label(&mut [0u8; 384]);
    let mut budget: u32 = 200_000;
    let mut stack = vec![(fs.root(), 0u32)];
    'walk: while let Some((dir, depth)) = stack.pop() {
        if depth > 64 {
            continue;
        }
        let _ = fs.parent(dir);
        let mut lookups = 0usize;
        let mut seen: HashSet<Vec<u8>> = HashSet::new();
        let mut cursor = DirCursor::START;
        loop {
            if budget == 0 {
                break 'walk;
            }
            budget -= 1;
            let entry = match fs.readdir(dir, cursor) {
                Ok(Some(entry)) => entry,
                Ok(None) | Err(_) => break,
            };
            cursor = entry.next_cursor();
            let node = entry.node();
            let child = entry.name();
            let bytes = child.as_bytes().to_vec();
            if seen.insert(bytes.clone()) && lookups < MAX_LOOKUPS_PER_DIR {
                lookups += 1;
                let Ok(found) = fs.lookup(dir, child) else {
                    panic!("ORACLE: lookup({bytes:?}) failed to re-resolve a listed entry");
                };
                assert_eq!(found, node, "ORACLE: lookup({bytes:?}) found another node");
                fs.forget(found, 1);
            }
            let _ = fs.stat(node);
            let _ = fs.extents(node, 0, &mut [hadris_fs::Extent::new(0, 0); 4]);
            let _ = fs.records(node, &mut [hadris_fs::Extent::new(0, 0); 1]);
            match entry.file_type() {
                FileType::Dir => stack.push((node, depth + 1)),
                FileType::Symlink => {
                    let mut target = [0u8; 4096];
                    let _ = fs.readlink(node, &mut target);
                }
                _ => {
                    let first = read_pass(fs, node);
                    let second = read_pass(fs, node);
                    assert_eq!(
                        first.1, second.1,
                        "ORACLE: repeated reads of {bytes:?} disagree on success"
                    );
                    assert!(
                        first.0 == second.0,
                        "ORACLE: repeated reads of {bytes:?} returned different bytes"
                    );
                }
            }
        }
    }
}

fn drive(data: &[u8]) {
    let mut bytes = data.to_vec();
    bytes.resize(bytes.len().next_multiple_of(512), 0);
    let Ok(mut fs) = UdfFs::mount(
        MemDevice::new(bytes, BlockSize::new(512).unwrap()),
        MountOptions::new(),
    ) else {
        return;
    };
    let _ = (
        fs.info().id(hadris_udf::UdfId::Volume),
        fs.info().id(hadris_udf::UdfId::VolumeSet),
        fs.info().volume_serial(),
        fs.info().partitions(),
        fs.was_dirty(),
    );
    walk(&mut fs);
}

fuzz_target!(|data: &[u8]| {
    drive(data);
});
