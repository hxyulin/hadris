#![no_main]
//! Fuzz the UDF reader: open an arbitrary image and recursively read every
//! directory and regular file. Arbitrary bytes must never panic/abort/OOM.
//!
//! This exercises the File Entry / allocation-descriptor / FID parsing that the
//! slice-bounds and extent-allocation fixes hardened.

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

    // Depth-guarded worklist: `read_directory` follows an ICB with no cycle
    // detection, so a self-referential directory would otherwise loop forever.
    let mut stack = vec![(root, 0u32)];
    while let Some((dir, depth)) = stack.pop() {
        if depth > 64 {
            continue;
        }
        for entry in dir.entries() {
            if entry.is_dir() {
                if let Ok(child) = fs.read_directory(&entry.icb) {
                    stack.push((child, depth + 1));
                }
            } else {
                let _ = fs.read_file(entry);
            }
        }
    }
}

fuzz_target!(|data: &[u8]| {
    drive(data);
});
