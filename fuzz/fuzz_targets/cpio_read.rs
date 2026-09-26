#![no_main]
//! Fuzz the CPIO reader: arbitrary bytes must never panic/abort/OOM.
//!
//! Drives the streaming read path over every format the reader detects
//! (newc, newc-crc, odc, old binary): header parse, name checks, reading
//! part of each entry's data and skipping the rest, and concatenated
//! archives after a trailer.
//!
//! Self-consistency oracles (failures are tagged `ORACLE:`): the archive is
//! iterated twice from fresh cursors and the entry sequence must be
//! identical, and an entry whose data reads without error yields exactly
//! its declared length.

use hadris_io::sync::Read;
use hadris_io::Cursor;
use libfuzzer_sys::fuzz_target;

use hadris_cpio::sync::CpioReader;

fn pass(data: &[u8], read_all: bool) -> Vec<(Vec<u8>, u64, u32)> {
    let mut reader = CpioReader::new(Cursor::new(data));
    let mut entries = Vec::new();
    let mut buf = [0u8; 97];
    loop {
        while let Ok(Some(mut entry)) = reader.next_entry() {
            entries.push((entry.path().to_vec(), entry.len(), entry.mode()));
            if !read_all {
                let _ = entry.read(&mut buf);
                continue;
            }
            let mut total = 0u64;
            let complete = loop {
                match entry.read(&mut buf) {
                    Ok(0) => break true,
                    Ok(read) => total += read as u64,
                    Err(_) => break false,
                }
            };
            if complete {
                assert_eq!(
                    total,
                    entry.len(),
                    "ORACLE: a fully read entry yielded a different length"
                );
            }
        }
        if !reader.next_segment().unwrap_or(false) {
            break;
        }
    }
    entries
}

fuzz_target!(|data: &[u8]| {
    let first = pass(data, true);
    let second = pass(data, false);
    assert_eq!(
        first, second,
        "ORACLE: reading or skipping the data produced a different entry sequence"
    );
});
