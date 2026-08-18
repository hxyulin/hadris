//! Regression tests for audit-found robustness bugs in untrusted-input paths
//! the `iso_read` fuzz target does not reach (`IsoDir::read_entries`,
//! `IsoModifier`).

use std::io::Cursor;

use hadris_iso::modify::IsoModifier;
use hadris_iso::read::IsoImage;

const SECTOR: usize = 2048;

fn be32_at(buf: &mut [u8], off: usize, value: u32) {
    buf[off..off + 4].copy_from_slice(&value.to_le_bytes());
    buf[off + 4..off + 8].copy_from_slice(&value.to_be_bytes());
}

fn be16_at(buf: &mut [u8], off: usize, value: u16) {
    buf[off..off + 2].copy_from_slice(&value.to_le_bytes());
    buf[off + 2..off + 4].copy_from_slice(&value.to_be_bytes());
}

/// Write a minimal valid directory record at `off`.
fn dir_record(buf: &mut [u8], off: usize, extent: u32, data_len: u32, flags: u8, name: &[u8]) {
    let len = (33 + name.len() + 1) & !1;
    buf[off] = len as u8;
    be32_at(buf, off + 2, extent);
    be32_at(buf, off + 10, data_len);
    buf[off + 25] = flags;
    be16_at(buf, off + 28, 1);
    buf[off + 32] = name.len() as u8;
    buf[off + 33..off + 33 + name.len()].copy_from_slice(name);
}

/// Minimal image: PVD at sector 16, terminator at sector 17, root dir at `root_lba`.
fn image_with_pvd(sectors: usize, root_lba: u32, root_len: u32) -> Vec<u8> {
    let mut img = vec![0u8; sectors * SECTOR];
    let pvd = &mut img[16 * SECTOR..17 * SECTOR];
    pvd[0] = 0x01;
    pvd[1..6].copy_from_slice(b"CD001");
    pvd[6] = 0x01;
    be32_at(pvd, 80, sectors as u32); // volume space size
    be16_at(pvd, 120, 1); // volume set size
    be16_at(pvd, 124, 1); // volume sequence number
    be16_at(pvd, 128, 2048); // logical block size
    be32_at(pvd, 132, 0); // path table size
    dir_record(pvd, 156, root_lba, root_len, 0x02, &[0x00]);
    pvd[881] = 1; // file structure version

    let term = &mut img[17 * SECTOR..18 * SECTOR];
    term[0] = 0xFF;
    term[1..6].copy_from_slice(b"CD001");
    term[6] = 0x01;
    img
}

mod alloc_counter {
    use std::alloc::{GlobalAlloc, Layout, System};
    use std::sync::atomic::{AtomicUsize, Ordering};

    static MAX_ALLOC: AtomicUsize = AtomicUsize::new(0);

    pub fn peak() -> usize {
        MAX_ALLOC.load(Ordering::Relaxed)
    }

    pub struct Counter;

    unsafe impl GlobalAlloc for Counter {
        unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
            MAX_ALLOC.fetch_max(layout.size(), Ordering::Relaxed);
            unsafe { System.alloc(layout) }
        }
        unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
            MAX_ALLOC.fetch_max(layout.size(), Ordering::Relaxed);
            unsafe { System.alloc_zeroed(layout) }
        }
        unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
            unsafe { System.dealloc(ptr, layout) }
        }
        unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
            MAX_ALLOC.fetch_max(new_size, Ordering::Relaxed);
            unsafe { System.realloc(ptr, layout, new_size) }
        }
    }
}

#[global_allocator]
static ALLOCATOR: alloc_counter::Counter = alloc_counter::Counter;

/// A directory record claiming ~4 GiB in a 43 KiB image must be rejected
/// before the claimed size is allocated.
#[test]
fn read_entries_rejects_oversized_directory_without_allocating() {
    const CLAIMED: u32 = 0xFFFF_F000;
    let mut img = image_with_pvd(21, 20, CLAIMED);
    dir_record(&mut img, 20 * SECTOR, 20, CLAIMED, 0x02, &[0x00]);

    let image = IsoImage::open(Cursor::new(img)).expect("open");
    let dir = image.open_dir(image.root_dir().dir_ref());
    let error = dir.read_entries().unwrap_err();

    assert_eq!(error.kind(), hadris_iso::ErrorKind::InvalidData);
    let peak = alloc_counter::peak();
    assert!(
        peak < CLAIMED as usize,
        "directory bytes were allocated before validation (peak {peak})"
    );
}

/// An image with no primary volume descriptor must yield an error, not a panic.
#[test]
fn modifier_open_without_pvd_returns_error() {
    let mut img = vec![0u8; 18 * SECTOR];
    let svd = &mut img[16 * SECTOR..17 * SECTOR];
    svd[0] = 0x02; // supplementary, valid CD001 header, no primary descriptor
    svd[1..6].copy_from_slice(b"CD001");
    svd[6] = 0x01;
    let term = &mut img[17 * SECTOR..18 * SECTOR];
    term[0] = 0xFF;
    term[1..6].copy_from_slice(b"CD001");
    term[6] = 0x01;

    let error = match IsoModifier::open(Cursor::new(img)) {
        Err(error) => error,
        Ok(_) => panic!("expected an error for an image without a primary descriptor"),
    };
    assert!(
        matches!(error, hadris_iso::modify::IsoModifyError::Io(_)),
        "expected I/O error, got {error:?}"
    );
}

/// A directory whose child entry points back at its own extent must be
/// rejected as cyclic instead of recursing until the stack overflows.
#[test]
fn modifier_open_rejects_cyclic_directory() {
    let mut img = image_with_pvd(21, 20, 2048);
    let root = 20 * SECTOR;
    dir_record(&mut img, root, 20, 2048, 0x02, &[0x00]); // .
    dir_record(&mut img, root + 34, 20, 2048, 0x02, &[0x01]); // ..
    dir_record(&mut img, root + 68, 20, 2048, 0x02, b"A"); // A -> own extent

    let error = match IsoModifier::open(Cursor::new(img)) {
        Err(error) => error,
        Ok(_) => panic!("expected an error for a cyclic directory graph"),
    };
    assert!(
        matches!(error, hadris_iso::modify::IsoModifyError::Io(_)),
        "expected I/O error, got {error:?}"
    );
}

/// A directory identifier that is only a version suffix decodes to an empty
/// name; the entry is dropped instead of panicking in `finish()`.
#[test]
fn modifier_skips_empty_directory_name() {
    let mut img = image_with_pvd(24, 20, 2048);
    let root = 20 * SECTOR;
    dir_record(&mut img, root, 20, 2048, 0x02, &[0x00]); // .
    dir_record(&mut img, root + 34, 20, 2048, 0x02, &[0x01]); // ..
    dir_record(&mut img, root + 68, 21, 2048, 0x02, b";1"); // name decodes to ""
    // sector 21 stays zeroed: empty directory

    let mut modifier = IsoModifier::open(Cursor::new(img)).expect("open");
    assert!(modifier.layout().subdirs.is_empty());
    modifier.append_file("x.txt", b"hi".to_vec());
    modifier.finish().expect("finish");
}
