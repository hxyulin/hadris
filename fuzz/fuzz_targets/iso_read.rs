#![no_main]
//! Fuzz the ISO 9660 reader: open an arbitrary image, then walk every
//! directory and read every file. Arbitrary bytes must never panic/abort/OOM.

use libfuzzer_sys::fuzz_target;
use std::io::Cursor;

use hadris_iso::read::IsoImage;

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
        for item in dir.entries() {
            if budget == 0 {
                return;
            }
            budget -= 1;
            let Ok(entry) = item else { continue };
            if entry.is_special() {
                continue; // "." and ".."
            }
            if entry.is_directory() {
                if let Ok(child) = entry.as_dir_ref(&image) {
                    stack.push((child, depth + 1));
                }
            } else if let Ok(bytes) = image.read_file(&entry) {
                let _ = bytes;
            }
        }
    }
}

fuzz_target!(|data: &[u8]| {
    drive(data);
});
