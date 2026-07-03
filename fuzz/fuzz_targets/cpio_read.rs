#![no_main]
//! Fuzz the CPIO reader: arbitrary bytes must never panic/abort/OOM.
//!
//! Drives the full streaming read path — header parse, filename allocation,
//! entry-data allocation — over attacker-controlled `namesize`/`filesize`.

use libfuzzer_sys::fuzz_target;
use std::io::Cursor;

use hadris_cpio::CpioReader;

fn drive(data: &[u8]) {
    // The cursor is finite, so each entry consumes >= 1 header (110 bytes);
    // the loop terminates when the input is exhausted (read returns Err).
    let mut reader = CpioReader::new(Cursor::new(data));
    while let Ok(Some(entry)) = reader.next_entry_alloc() {
        let _ = reader.read_entry_data_alloc(&entry);
    }
}

fuzz_target!(|data: &[u8]| {
    drive(data);
});
