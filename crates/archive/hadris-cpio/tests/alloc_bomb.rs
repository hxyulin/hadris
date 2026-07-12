//! Regression test for the allocation-DoS (issue: attacker-controlled
//! `namesize`/`filesize` sized the buffer before validation).
//!
//! A malicious ~114-byte archive claims a ~4 GiB `namesize`. The reader must
//! grow its buffer as bytes actually arrive, never pre-allocating the claim.
//!
//! This test binary installs a capped global allocator: any single allocation
//! larger than `CAP` fails, which makes Rust's alloc-error handler abort. If the
//! reader ever regresses to `vec![0u8; claimed_len]`, that abort crashes this
//! binary and fails CI — on any host, regardless of memory overcommit.

use std::alloc::{GlobalAlloc, Layout, System};

use hadris_cpio::read::CpioReader;

const CAP: usize = 512 * 1024 * 1024; // 512 MiB — far above any legit allocation here

struct Capped;
unsafe impl GlobalAlloc for Capped {
    unsafe fn alloc(&self, l: Layout) -> *mut u8 {
        if l.size() > CAP {
            return std::ptr::null_mut();
        }
        unsafe { System.alloc(l) }
    }
    unsafe fn dealloc(&self, p: *mut u8, l: Layout) {
        unsafe { System.dealloc(p, l) }
    }
    unsafe fn realloc(&self, p: *mut u8, l: Layout, new: usize) -> *mut u8 {
        if new > CAP {
            return std::ptr::null_mut();
        }
        unsafe { System.realloc(p, l, new) }
    }
}

#[global_allocator]
static ALLOC: Capped = Capped;

/// Build a 110-byte newc header (all fields zero) then override one 8-char hex field.
fn header_with(field_offset: usize, hex8: &str) -> Vec<u8> {
    let mut h = Vec::new();
    h.extend_from_slice(b"070701"); // magic
    for _ in 0..13 {
        h.extend_from_slice(b"00000000"); // ino..check
    }
    assert_eq!(h.len(), 110);
    h[field_offset..field_offset + 8].copy_from_slice(hex8.as_bytes());
    h
}

#[test]
fn oversized_namesize_does_not_preallocate() {
    // namesize is field index 12 → offset 6 + 12*8 = 102... use documented layout: namesize at 94.
    let mut archive = header_with(94, "FFFFFFFF");
    archive.extend_from_slice(b"AAAA"); // 114 bytes total, far short of the 4 GiB claim

    let mut reader = CpioReader::new(archive.as_slice());
    // Must return an error (EOF), not abort from a 4 GiB allocation.
    assert!(reader.next_entry_alloc().is_err());
}

#[test]
fn oversized_filesize_does_not_preallocate() {
    // A valid one-char name "x", then a bogus 4 GiB filesize.
    let mut archive = header_with(54, "FFFFFFFF"); // filesize at offset 54
    archive[94..102].copy_from_slice(b"00000002"); // namesize = 2 ("x\0")
    archive.extend_from_slice(b"x\0"); // name
    archive.extend_from_slice(b"\0\0"); // pad to 4-byte boundary (110+2=112, already aligned; harmless slack)

    let mut reader = CpioReader::new(archive.as_slice());
    let entry = reader
        .next_entry_alloc()
        .expect("header parses")
        .expect("not trailer");
    assert_eq!(entry.file_size(), 0xFFFF_FFFF);
    // Reading the claimed 4 GiB of data must error at EOF, not abort.
    assert!(reader.read_entry_data_alloc(&entry).is_err());
}
