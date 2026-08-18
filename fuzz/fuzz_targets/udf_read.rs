#![no_main]
//! Fuzz the UDF reader: open an arbitrary image and recursively read every
//! directory and regular file. Arbitrary bytes must never panic/abort/OOM.
//!
//! This exercises the File Entry / allocation-descriptor / FID parsing that the
//! slice-bounds and extent-allocation fixes hardened.
//!
//! Self-consistency oracle (failures are tagged `ORACLE:`): every file is
//! read twice and the bytes must match.

use libfuzzer_sys::fuzz_target;
use std::io::Cursor;

use hadris_udf::UdfVolume;

fn drive(data: &[u8]) {
    let Ok(fs) = UdfVolume::open(Cursor::new(data)) else {
        return;
    };
    let Ok(root) = fs.root_dir() else {
        return;
    };

    // Depth-guarded worklist. The depth cap alone is NOT enough: a corrupt
    // directory graph (ICBs pointing at sibling/ancestor dirs — `read_directory`
    // has no cycle detection) has a path count that grows like branching^depth,
    // so a naive walk fans out and hangs — a harness DoS, not a library bug.
    // A flat work budget bounds total entries processed on ANY input.
    let mut budget: u32 = 200_000;
    let mut stack = vec![(root, 0u32)];
    while let Some((dir, depth)) = stack.pop() {
        if depth > 64 {
            continue;
        }
        for entry in dir.entries() {
            if budget == 0 {
                return;
            }
            budget -= 1;
            if entry.is_dir() {
                if let Ok(child) = fs.read_directory(&entry.icb) {
                    stack.push((child, depth + 1));
                }
            } else {
                // Read-twice oracle: two reads must yield identical bytes.
                let first = fs.read_file(entry);
                let second = fs.read_file(entry);
                assert_eq!(
                    first.is_ok(),
                    second.is_ok(),
                    "ORACLE: repeated reads of {:?} disagree on success",
                    entry.name()
                );
                if let (Ok(a), Ok(b)) = (first, second) {
                    assert_eq!(
                        a,
                        b,
                        "ORACLE: repeated reads of {:?} returned different bytes",
                        entry.name()
                    );
                }
            }
        }
    }
}

fuzz_target!(|data: &[u8]| {
    drive(data);
});
