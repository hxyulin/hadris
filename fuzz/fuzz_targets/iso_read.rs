#![no_main]
//! Fuzz the ISO 9660 reader: open an arbitrary image, read its descriptors
//! and boot catalog, then walk every tree it carries, reading every file,
//! link, Rock Ridge entry set and directory record. Arbitrary bytes must
//! never panic, abort or OOM.
//!
//! Self-consistency oracles (failures are tagged `ORACLE:`): every file is
//! read twice and the bytes must match, listed entries must re-resolve by
//! name through `lookup`, and a listed file must be a valid handle.

use std::collections::HashSet;

use hadris_fs::sync::FileSystem;
use hadris_fs::{DirCursor, ErrorKind, FileType, NodeId};
use hadris_iso::sync::{IsoImage, IsoView};
use hadris_storage::{BlockSize, MemDevice};
use libfuzzer_sys::fuzz_target;

type View<'a> = IsoView<&'a mut MemDevice<Vec<u8>>>;

/// `lookup` re-scans a directory from the start, so cap name re-resolution
/// lookups per directory to keep the walk from going quadratic under the
/// flat work budget.
const MAX_LOOKUPS_PER_DIR: usize = 32;
const READ_CAP: usize = 4 * 1024 * 1024;

/// Reads a file in chunks with a byte cap: the size is fuzz-controlled.
/// Returns the bytes and whether the read ended in an error.
fn read_pass(view: &mut View<'_>, node: NodeId) -> (Vec<u8>, bool) {
    let mut buf = [0u8; 64 * 1024];
    let mut out = Vec::new();
    loop {
        match view.read(node, out.len() as u64, &mut buf) {
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

/// Walks one tree. A corrupt directory graph (records pointing at sibling
/// or ancestor directories) has a path count that grows like
/// branching^depth, so a flat work budget bounds the entries processed.
fn walk(view: &mut View<'_>, budget: &mut u32) {
    let _ = view.statfs();
    let _ = view.label(&mut [0u8; 384]);
    let mut stack = vec![(view.root(), 0u32)];
    'walk: while let Some((dir, depth)) = stack.pop() {
        if depth > 64 {
            continue;
        }
        let _ = view.parent(dir);
        let _ = view.rock_ridge(dir);
        let _ = view.raw_record(dir);
        let mut lookups = 0usize;
        let mut seen_names: HashSet<Vec<u8>> = HashSet::new();
        let mut cursor = DirCursor::START;
        loop {
            if *budget == 0 {
                break 'walk;
            }
            *budget -= 1;
            let entry = match view.readdir(dir, cursor) {
                Ok(Some(entry)) => entry,
                Ok(None) | Err(_) => break,
            };
            cursor = entry.next_cursor();
            let node = entry.node();
            let child_name = entry.name();
            let bytes = child_name.as_bytes().to_vec();
            let is_new_name = seen_names.insert(bytes.clone());

            if lookups < MAX_LOOKUPS_PER_DIR {
                lookups += 1;
                let Ok(found) = view.lookup(dir, child_name) else {
                    panic!("ORACLE: lookup({bytes:?}) failed to re-resolve a listed entry");
                };
                // A directory's metadata is read from its own extent, which
                // a corrupt image can place anywhere; any other node's comes
                // from the record just listed.
                if is_new_name && found == node && entry.file_type() != FileType::Dir {
                    if let Err(err) = view.stat(found) {
                        assert_ne!(
                            err.kind(),
                            ErrorKind::InvalidHandle,
                            "ORACLE: lookup({bytes:?}) returned a node that is not a record"
                        );
                    }
                }
                view.forget(found, 1);
            }

            let _ = view.rock_ridge(node);
            let _ = view.raw_record(node);
            let _ = view.extents(node, |_| {});
            match entry.file_type() {
                FileType::Dir => stack.push((node, depth + 1)),
                FileType::Symlink => {
                    let mut target = [0u8; 4096];
                    let _ = view.readlink(node, &mut target);
                }
                _ => {
                    let first = read_pass(view, node);
                    let second = read_pass(view, node);
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
    let dev = MemDevice::new(bytes, BlockSize::new(512).unwrap());
    let Ok(mut image) = IsoImage::open(dev) else {
        return;
    };
    for index in 0..70 {
        if !matches!(image.descriptor(index), Ok(Some(_))) {
            break;
        }
    }
    let _ = image.primary_descriptor();
    let _ = image.boot_catalog();

    let mut budget: u32 = 200_000;
    let namespaces: Vec<_> = image.namespaces().iter().collect();
    for namespace in namespaces {
        if let Ok(mut view) = image.view(namespace) {
            walk(&mut view, &mut budget);
        }
    }
}

fuzz_target!(|data: &[u8]| {
    drive(data);
});
