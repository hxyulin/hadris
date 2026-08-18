#![no_main]
//! Fuzz the CPIO reader: arbitrary bytes must never panic/abort/OOM.
//!
//! Drives the full streaming read path — header parse, filename allocation,
//! entry-data allocation — over attacker-controlled `namesize`/`filesize`.
//!
//! Self-consistency oracle (failures are tagged `ORACLE:`): the archive is
//! iterated twice from fresh cursors and the (name, size) entry sequence must
//! be identical across both passes.

use libfuzzer_sys::fuzz_target;
use std::io::Cursor;

use hadris_cpio::sync::CpioArchiveReader;

fn pass(data: &[u8]) -> Vec<(Vec<u8>, u32)> {
    // The cursor is finite, so each entry consumes >= 1 header (110 bytes);
    // the loop terminates when the input is exhausted (read returns Err).
    let mut reader = CpioArchiveReader::new(Cursor::new(data));
    let mut entries = Vec::new();
    while let Ok(Some(entry)) = reader.next_entry_alloc() {
        entries.push((entry.name().to_vec(), entry.file_size()));
        let _ = reader.read_entry_data_alloc(&entry);
    }
    entries
}

fn drive(data: &[u8]) {
    let first = pass(data);
    let second = pass(data);
    assert_eq!(
        first, second,
        "ORACLE: repeated archive iteration produced a different entry sequence"
    );
}

fuzz_target!(|data: &[u8]| {
    drive(data);
});
