#![no_main]
//! Fuzz the ISO 9660 reader: open an arbitrary image, then walk every
//! directory and read every file. Arbitrary bytes must never panic/abort/OOM.
//!
//! Self-consistency oracles (failures are tagged `ORACLE:`): every file is
//! read twice and the bytes must match, and walked entries are re-resolved by
//! name through `IsoDir::find` (guarded against ambiguous case-insensitive
//! display-name matches on corrupt images).

use libfuzzer_sys::fuzz_target;
use std::collections::HashSet;
use std::io::Cursor;

use hadris_iso::read::{DirEntry, IsoImage};

/// `find` re-reads the whole directory, so cap name re-resolution lookups
/// per directory to keep the walk from going quadratic under the flat work
/// budget.
const MAX_LOOKUPS_PER_DIR: usize = 32;

/// A query string that `IsoDir::find` (via `matches_name`) is guaranteed to
/// match against this entry, or `None` if no such query can be derived.
fn lookup_query(entry: &DirEntry) -> Option<String> {
    let display = entry.display_name();
    if entry.matches_name(&display) {
        return Some(display.into_owned());
    }
    // Mirror `matches_name`'s display branch: drop NULs and a `;<digits>`
    // version suffix.
    let filtered: String = display.chars().filter(|c| *c != '\0').collect();
    let stripped = match filtered.rsplit_once(';') {
        Some((base, version))
            if !version.is_empty() && version.bytes().all(|b| b.is_ascii_digit()) =>
        {
            base.to_string()
        }
        _ => filtered,
    };
    if entry.matches_name(&stripped) {
        return Some(stripped);
    }
    None
}

fn drive(data: &[u8]) {
    let Ok(image) = IsoImage::open(Cursor::new(data)) else {
        return;
    };

    // Depth-guarded `DirectoryRef` worklist. The depth cap alone is NOT enough:
    // a corrupt directory graph (extents pointing at sibling/ancestor dirs) has
    // a path count that grows like branching^depth, so a naive walk fans out and
    // hangs — a harness DoS, not a library bug. A flat work budget bounds total
    // entries processed on ANY input.
    // ponytail: budget over visited-set — no per-format extent accessor needed.
    let mut budget: u32 = 200_000;
    let mut stack = vec![(image.root_dir().dir_ref(), 0u32)];
    while let Some((dref, depth)) = stack.pop() {
        if depth > 64 {
            continue;
        }
        let dir = image.open_dir(dref);
        let mut saw_error = false;
        let mut lookups = 0usize;
        let mut seen_names: HashSet<Vec<u8>> = HashSet::new();
        for item in dir.entries() {
            if budget == 0 {
                return;
            }
            budget -= 1;
            let Ok(entry) = item else {
                saw_error = true;
                continue;
            };
            if entry.is_special() {
                continue; // "." and ".."
            }
            let is_new_name = seen_names.insert(entry.name().to_vec());

            // Name re-resolution oracle. `find` collects entries through a
            // different path than the streaming iterator, so an `Err` is not
            // asserted on — but an `Ok(None)` for a self-matching query is a
            // genuine wrong result.
            if !saw_error && lookups < MAX_LOOKUPS_PER_DIR {
                lookups += 1;
                if let Some(query) = lookup_query(&entry) {
                    match dir.find(&query) {
                        Ok(Some(found)) => {
                            if is_new_name && found.name() == entry.name() {
                                assert_eq!(
                                    found.is_directory(),
                                    entry.is_directory(),
                                    "ORACLE: find({query:?}) returned entry with different kind"
                                );
                                assert_eq!(
                                    found.size(),
                                    entry.size(),
                                    "ORACLE: find({query:?}) returned entry with different size"
                                );
                            }
                        }
                        Ok(None) => {
                            panic!("ORACLE: find({query:?}) failed to re-resolve walked entry")
                        }
                        Err(_) => {}
                    }
                }
            }

            if entry.is_directory() {
                if let Ok(child) = entry.as_dir_ref(&image) {
                    stack.push((child, depth + 1));
                }
            } else {
                // Read-twice oracle: two reads must yield identical bytes.
                let first = image.read_file(&entry);
                let second = image.read_file(&entry);
                assert_eq!(
                    first.is_ok(),
                    second.is_ok(),
                    "ORACLE: repeated reads of {:?} disagree on success",
                    entry.display_name()
                );
                if let (Ok(a), Ok(b)) = (first, second) {
                    assert_eq!(
                        a,
                        b,
                        "ORACLE: repeated reads of {:?} returned different bytes",
                        entry.display_name()
                    );
                }
            }
        }
    }
}

fuzz_target!(|data: &[u8]| {
    drive(data);
});
