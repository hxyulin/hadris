#![cfg(feature = "exfat")]
//! Regression test for the exFAT up-case table allocation-DoS.
//!
//! The up-case table directory entry's `data_length` is an untrusted **u64**
//! on-disk field. It must be bounded against the volume before it sizes the
//! read buffer — otherwise a corrupt entry in a tiny image claims ~exabytes and
//! aborts the process on a no-overcommit / embedded target.
//!
//! This binary installs a capped global allocator: any single allocation larger
//! than `CAP` fails, triggering Rust's abort handler. If `UpcaseTable::load`
//! ever regresses to `vec![0u8; size]`, the bogus 4 GiB allocation crashes this
//! binary and fails the test — on any host, regardless of memory overcommit.

use std::alloc::{GlobalAlloc, Layout, System};
use std::io::Cursor;

use hadris_fat::exfat::{ExFatInfo, UpcaseTable};

const CAP: usize = 128 * 1024 * 1024; // 128 MiB — far above any legit allocation here

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

/// A minimal volume with an 8-cluster, 512-byte-per-cluster heap → 4 KiB total.
fn tiny_info() -> ExFatInfo {
    ExFatInfo {
        bytes_per_sector: 512,
        sectors_per_cluster: 1,
        bytes_per_cluster: 512,
        fat_offset: 0,
        fat_length: 0,
        cluster_heap_offset: 0,
        cluster_count: 8,
        root_cluster: 2,
        volume_serial: 0,
        fat_count: 1,
    }
}

#[test]
fn oversized_upcase_data_length_does_not_preallocate() {
    let info = tiny_info();
    let mut data = Cursor::new(Vec::<u8>::new());
    let mut table = UpcaseTable::new();
    // data_length claims 4 GiB on a 4 KiB volume. Must return an error, not
    // attempt a 4 GiB allocation (which the capped allocator turns into an abort).
    let result = table.load(&mut data, &info, 2, 0x1_0000_0000, true);
    assert!(result.is_err());
}
