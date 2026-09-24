#![no_main]
//! Fuzz the UDF reader: open an arbitrary image and walk every directory,
//! reading every file and link and the metadata, parent and extents of
//! every node. Arbitrary bytes must never panic, abort or OOM.
//!
//! Self-consistency oracles (failures are tagged `ORACLE:`): every file is
//! read twice and the bytes must match, and listed entries must re-resolve
//! by name through `lookup` to the id the listing gave.

use std::collections::HashSet;

use hadris_fs::{DirCursor, FileType, NameBuf, NodeId};
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

/// Walks the tree. A corrupt directory graph (identifiers pointing at
/// sibling or ancestor directories) has a path count that grows like
/// branching^depth, so a flat work budget bounds the entries processed.
fn walk(fs: &mut Fs) {
    let _ = fs.stats();
    let mut budget: u32 = 200_000;
    let mut stack = vec![(fs.root(), 0u32)];
    'walk: while let Some((dir, depth)) = stack.pop() {
        if depth > 64 {
            continue;
        }
        let _ = fs.parent(dir);
        let mut lookups = 0usize;
        let mut seen: HashSet<Vec<u8>> = HashSet::new();
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
            let Some(child) = name.as_name() else {
                continue;
            };
            let bytes = child.as_bytes().to_vec();
            if seen.insert(bytes.clone()) && lookups < MAX_LOOKUPS_PER_DIR {
                lookups += 1;
                let Ok(found) = fs.lookup(dir, child) else {
                    panic!("ORACLE: lookup({bytes:?}) failed to re-resolve a listed entry");
                };
                assert_eq!(found, node, "ORACLE: lookup({bytes:?}) found another node");
                fs.forget(found);
            }
            let _ = fs.node_metadata(node);
            let _ = fs.extents(node, |_| {});
            match entry.file_type() {
                FileType::Dir => stack.push((node, depth + 1)),
                FileType::Symlink => {
                    let mut target = [0u8; 4096];
                    let _ = fs.read_link(node, &mut target);
                }
                _ => {
                    let first = read_pass(fs, node);
                    let second = read_pass(fs, node);
                    assert_eq!(first.1, second.1, "ORACLE: repeated reads of {bytes:?} disagree on success");
                    assert!(first.0 == second.0, "ORACLE: repeated reads of {bytes:?} returned different bytes");
                }
            }
        }
    }
}

fn drive(data: &[u8]) {
    let mut bytes = data.to_vec();
    bytes.resize(bytes.len().next_multiple_of(512), 0);
    let Ok(mut fs) = UdfFs::open(MemDevice::new(bytes, BlockSize::new(512).unwrap())) else {
        return;
    };
    let _ = (fs.volume_id(), fs.logical_volume_id(), fs.revision(), fs.partitions());
    walk(&mut fs);
}

fuzz_target!(|data: &[u8]| {
    drive(data);
});
