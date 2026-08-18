#![no_main]
//! Fuzz the FAT reader: mount an arbitrary image, then walk every directory
//! and read every file. Arbitrary bytes must never panic/abort/OOM.
//!
//! Self-consistency oracles (failures are tagged `ORACLE:`): every file is
//! read through two fresh readers and the bytes must match, and walked entries
//! are re-resolved by name through `FatDir::find` (guarded against ambiguous
//! duplicate names and lossy OEM short-name decoding on corrupt images).

use libfuzzer_sys::fuzz_target;
use std::collections::HashSet;
use std::io::Cursor;

use hadris_fat::{FatVolume, FatVolumeReadExt, FileEntry};

/// `find` re-scans a directory from the start, so bound name re-resolution
/// lookups two ways: a per-directory cap keeps coverage spread across
/// directories, and a flat per-input budget bounds the total — on
/// budget-exhausting inputs (tens of thousands of directories) a per-dir
/// cap alone still adds up to seconds of oracle time per exec, which
/// libFuzzer flags as slow units.
const MAX_LOOKUPS_PER_DIR: usize = 32;
const LOOKUP_BUDGET: u32 = 8192;

fn drive(data: &[u8]) {
    let Ok(fs) = FatVolume::open(Cursor::new(data)) else {
        return;
    };

    // Chunked read with a byte cap: the entry's size is fuzz-controlled and a
    // corrupt FAT can serve the same sectors over and over, so never buffer an
    // unbounded `read_to_vec` (a single file could otherwise grow to GiBs).
    let read_pass = |fe: &FileEntry| -> Option<Vec<u8>> {
        let mut reader = fs.read_file(fe).ok()?;
        let mut buf = [0u8; 64 * 1024];
        let mut out = Vec::new();
        loop {
            match reader.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    out.extend_from_slice(&buf[..n]);
                    if out.len() >= 16 * 1024 * 1024 {
                        break;
                    }
                }
            }
        }
        Some(out)
    };

    // Depth-guarded worklist. The depth cap alone is NOT enough: a corrupt
    // directory graph (entries pointing at sibling/ancestor clusters) has a
    // path count that grows like branching^depth, so a naive walk fans out to
    // billions of `open_entry`/`read_file` calls and hangs — a harness DoS, not
    // a library bug (the library bounds single chains via ClusterLoop). A flat
    // work budget bounds total entries processed on ANY input.
    // ponytail: budget over visited-set — no per-format cluster accessor needed.
    let mut budget: u32 = 200_000;
    let mut lookup_budget = LOOKUP_BUDGET;
    let mut stack = vec![(fs.root_dir(), 0u32)];
    while let Some((dir, depth)) = stack.pop() {
        if depth > 64 {
            continue;
        }
        let mut saw_error = false;
        let mut lookups = 0usize;
        let mut seen_names: HashSet<String> = HashSet::new();
        for item in dir.entries() {
            if budget == 0 {
                return;
            }
            budget -= 1;
            let Ok(de) = item else {
                saw_error = true;
                continue;
            };
            let Some(fe) = de.as_entry() else { continue };
            let name = fe.name();
            if name == "." || name == ".." {
                continue;
            }
            let is_new_name = seen_names.insert(name.clone().into_owned());

            // Name re-resolution oracle. Only sound when the entry's own
            // display name round-trips through `find`'s matcher (lossy OEM
            // decoding can break that for short names) and no earlier item in
            // this directory errored (`find` would hit the same error first).
            if !saw_error && lookups < MAX_LOOKUPS_PER_DIR && lookup_budget > 0 {
                lookups += 1;
                let self_findable = match fe.long_name() {
                    Some(lfn) => lfn.eq_str(&name),
                    None => fe.short_name().matches(&name),
                };
                if self_findable {
                    lookup_budget -= 1;
                    match dir.find(&name) {
                        Ok(Some(found)) => {
                            if is_new_name && found.name() == name {
                                assert_eq!(
                                    found.is_directory(),
                                    fe.is_directory(),
                                    "ORACLE: find({name:?}) returned entry with different kind"
                                );
                                assert_eq!(
                                    found.len(),
                                    fe.len(),
                                    "ORACLE: find({name:?}) returned entry with different size"
                                );
                            }
                        }
                        other => {
                            panic!("ORACLE: find({name:?}) failed to re-resolve walked entry: {other:?}")
                        }
                    }
                }
            }

            if fe.is_directory() {
                if let Ok(child) = dir.open_entry(fe) {
                    stack.push((child, depth + 1));
                }
            } else {
                // Read-twice oracle: two fresh readers must yield identical bytes.
                let first = read_pass(fe);
                let second = read_pass(fe);
                assert_eq!(
                    first.is_some(),
                    second.is_some(),
                    "ORACLE: repeated reads of {name:?} disagree on success"
                );
                if let (Some(a), Some(b)) = (first, second) {
                    assert_eq!(
                        a, b,
                        "ORACLE: repeated reads of {name:?} returned different bytes"
                    );
                }
            }
        }
    }
}

fuzz_target!(|data: &[u8]| {
    drive(data);
});
