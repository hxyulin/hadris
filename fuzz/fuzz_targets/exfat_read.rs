#![no_main]
//! Fuzz the exFAT reader: mount an arbitrary image, then walk every directory
//! and read every file. Arbitrary bytes must never panic/abort/OOM.
//!
//! Self-consistency oracles (failures are tagged `ORACLE:`): every file is
//! read through two fresh readers and the bytes must match, and walked entries
//! are re-resolved by name through `ExFatDir::find` (guarded against ambiguous
//! names — the on-disk up-case table is fuzz-controlled and can alias
//! distinct names on corrupt images).

use libfuzzer_sys::fuzz_target;
use std::collections::HashSet;
use std::io::Cursor;

use hadris_fat::exfat::{ExFatFileEntry, ExFatFileReader, ExFatVolume};
use hadris_fat::io::Read;

/// `find` re-scans a directory from the start, so bound name re-resolution
/// lookups two ways: a per-directory cap keeps coverage spread across
/// directories, and a flat per-input budget bounds the total — on
/// budget-exhausting inputs (tens of thousands of directories) a per-dir
/// cap alone still adds up to seconds of oracle time per exec, which
/// libFuzzer flags as slow units.
const MAX_LOOKUPS_PER_DIR: usize = 32;
const LOOKUP_BUDGET: u32 = 8192;

fn drive(data: &[u8]) {
    let Ok(fs) = ExFatVolume::open(Cursor::new(data)) else {
        return;
    };

    // Chunked read with a byte cap: `valid_data_length` is a fuzz-controlled
    // u64, so never size an allocation from it.
    let read_pass = |entry: &ExFatFileEntry| -> Option<Vec<u8>> {
        let mut reader = ExFatFileReader::new(&fs, entry).ok()?;
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

    // Depth-guarded worklist with a flat work budget: same rationale as
    // fat_read — a corrupt directory graph fans out exponentially, so bound
    // total entries processed on ANY input.
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
            let Ok(entry) = item else {
                saw_error = true;
                continue;
            };
            let is_new_name = seen_names.insert(entry.name.clone());

            // Name re-resolution oracle: a name read from this directory must
            // re-resolve. Kind/size are only compared when the resolved name is
            // byte-identical and unseen before — a corrupt up-case table can
            // otherwise legitimately alias the name to a different entry.
            if !saw_error && lookups < MAX_LOOKUPS_PER_DIR && lookup_budget > 0 {
                lookups += 1;
                lookup_budget -= 1;
                match dir.find(&entry.name) {
                    Ok(Some(found)) => {
                        if is_new_name && found.name == entry.name {
                            assert_eq!(
                                found.is_directory(),
                                entry.is_directory(),
                                "ORACLE: find({:?}) returned entry with different kind",
                                entry.name
                            );
                            assert_eq!(
                                found.size(),
                                entry.size(),
                                "ORACLE: find({:?}) returned entry with different size",
                                entry.name
                            );
                        }
                    }
                    other => panic!(
                        "ORACLE: find({:?}) failed to re-resolve walked entry: {other:?}",
                        entry.name
                    ),
                }
            }

            if entry.is_directory() {
                if let Ok(child) = dir.open_dir(&entry.name) {
                    stack.push((child, depth + 1));
                }
            } else {
                // Read-twice oracle: two fresh readers must yield identical bytes.
                let first = read_pass(&entry);
                let second = read_pass(&entry);
                assert_eq!(
                    first.is_some(),
                    second.is_some(),
                    "ORACLE: repeated reads of {:?} disagree on success",
                    entry.name
                );
                if let (Some(a), Some(b)) = (first, second) {
                    assert_eq!(
                        a, b,
                        "ORACLE: repeated reads of {:?} returned different bytes",
                        entry.name
                    );
                }
            }
        }
    }
}

fuzz_target!(|data: &[u8]| {
    drive(data);
});
