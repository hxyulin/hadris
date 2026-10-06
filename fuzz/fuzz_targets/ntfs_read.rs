#![no_main]
//! Fuzz the NTFS reader: open an arbitrary image and walk every directory,
//! reading every file, its named streams and the metadata and parent of
//! every node. Arbitrary bytes must never panic, abort or OOM.
//!
//! Self-consistency oracles (failures are tagged `ORACLE:`): every file is
//! read twice and the bytes must match, and listed entries must re-resolve
//! by name through `lookup` to the id the listing gave.

use hadris_fs::MountOptions;
use std::collections::HashSet;

use hadris_fs::sync::FileSystem;
use hadris_fs::{DirCursor, FileType, NodeId};
use hadris_ntfs::sync::NtfsFs;
use hadris_storage::{BlockSize, MemDevice};
use libfuzzer_sys::fuzz_target;

type Fs = NtfsFs<MemDevice<Vec<u8>>>;

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

fn read_streams(fs: &mut Fs, node: NodeId) {
    let mut names = Vec::new();
    let _ = fs.streams(node, |name, _| names.push(name.to_string()));
    for name in names.iter().take(8) {
        let mut buf = [0u8; 4096];
        let _ = fs.read_stream_at(node, name, 0, &mut buf);
    }
}

/// Walks the tree. A corrupt directory graph (entries pointing at sibling
/// or ancestor directories) has a path count that grows like
/// branching^depth, so a flat work budget bounds the entries processed.
fn walk(fs: &mut Fs) {
    let _ = fs.statfs();
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
            }
            let _ = fs.stat(node);
            read_streams(fs, node);
            match entry.file_type() {
                FileType::Dir => stack.push((node, depth + 1)),
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
    let Ok(mut fs) = NtfsFs::mount(
        MemDevice::new(bytes, BlockSize::new(512).unwrap()),
        MountOptions::new(),
    ) else {
        return;
    };
    let mut label = [0u8; 1024];
    let _ = fs.label(&mut label);
    let _ = (fs.volume_serial(), fs.cluster_size(), fs.mft_record_size());
    walk(&mut fs);
}

fuzz_target!(|data: &[u8]| {
    drive(data);
});
