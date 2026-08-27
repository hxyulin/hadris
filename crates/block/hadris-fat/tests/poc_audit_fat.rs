//! Audit POCs for hadris-fat. Each test demonstrates a robustness bug
//! reachable from untrusted disk-image input; the tests FAIL (or panic) while
//! the bug is present.
//!
//! Run:
//!   cargo test -p hadris-fat --lib poc_audit_fat::
//!   cargo test -p hadris-fat --features unstable-exfat --lib poc_audit_fat::
//!   cargo test -p hadris-fat --features tool --lib poc_audit_fat::

use std::io::Cursor;

#[cfg(any(feature = "unstable-exfat", feature = "tool"))]
use std::sync::atomic::Ordering;

#[cfg(feature = "unstable-exfat")]
use hadris_fat::exfat::ExFatVolume;

// ---------------------------------------------------------------------------
// Counting allocator: records the largest single allocation request so a test
// can prove an untrusted on-disk length field drove an allocation size.
// ---------------------------------------------------------------------------

#[cfg(any(feature = "unstable-exfat", feature = "tool"))]
mod alloc_watch {
    use std::alloc::{GlobalAlloc, Layout, System};
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicUsize, Ordering};

    pub static MAX_ALLOC: AtomicUsize = AtomicUsize::new(0);
    pub static LOCK: Mutex<()> = Mutex::new(());

    pub struct Watch;

    unsafe impl GlobalAlloc for Watch {
        unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
            MAX_ALLOC.fetch_max(layout.size(), Ordering::SeqCst);
            unsafe { System.alloc(layout) }
        }
        unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
            unsafe { System.dealloc(ptr, layout) }
        }
    }
}

#[cfg(any(feature = "unstable-exfat", feature = "tool"))]
#[global_allocator]
static ALLOC_WATCH: alloc_watch::Watch = alloc_watch::Watch;

// ---------------------------------------------------------------------------
// FAT12/16 image builder (512-byte sectors, 1 FAT, 224 root entries)
// ---------------------------------------------------------------------------

const FAT12_ROOT_ENTRIES: usize = 224; // standard floppy geometry: 7168 bytes
const FAT12_SPF: usize = 9;
const FAT12_ROOT_SECTORS: usize = FAT12_ROOT_ENTRIES * 32 / 512; // 14
const FAT12_DATA_SECTORS: usize = 100;
const FAT12_TOTAL_SECTORS: usize = 1 + FAT12_SPF + FAT12_ROOT_SECTORS + FAT12_DATA_SECTORS;
const FAT12_ROOT_OFFSET: usize = (1 + FAT12_SPF) * 512;

fn fat12_image() -> Vec<u8> {
    let mut img = vec![0u8; FAT12_TOTAL_SECTORS * 512];
    img[0..3].copy_from_slice(&[0xEB, 0x3C, 0x90]);
    img[3..11].copy_from_slice(b"MSDOS5.0");
    img[11..13].copy_from_slice(&512u16.to_le_bytes()); // bytes per sector
    img[13] = 1; // sectors per cluster
    img[14..16].copy_from_slice(&1u16.to_le_bytes()); // reserved sectors
    img[16] = 1; // FAT count
    img[17..19].copy_from_slice(&(FAT12_ROOT_ENTRIES as u16).to_le_bytes());
    img[19..21].copy_from_slice(&(FAT12_TOTAL_SECTORS as u16).to_le_bytes());
    img[21] = 0xF8; // media
    img[22..24].copy_from_slice(&(FAT12_SPF as u16).to_le_bytes());
    img[24..26].copy_from_slice(&18u16.to_le_bytes()); // sectors per track
    img[26..28].copy_from_slice(&2u16.to_le_bytes()); // heads
    img[36] = 0x80; // drive
    img[38] = 0x29; // ext boot signature
    img[43..54].copy_from_slice(b"NO NAME    ");
    img[54..62].copy_from_slice(b"FAT12   ");
    img[510] = 0x55;
    img[511] = 0xAA;
    // FAT[0]: media descriptor + reserved bits
    img[512] = 0xF8;
    img[513] = 0xFF;
    img[514] = 0xFF;
    img
}

fn put_short_entry(img: &mut [u8], slot: usize, name: &[u8; 11], attr: u8, cluster: u16) {
    let base = FAT12_ROOT_OFFSET + slot * 32;
    img[base..base + 11].copy_from_slice(name);
    img[base + 11] = attr;
    img[base + 26..base + 28].copy_from_slice(&cluster.to_le_bytes());
}

fn filler_name(i: usize) -> [u8; 11] {
    let s = format!("FILE{i:04}TXT");
    let mut name = [b' '; 11];
    name.copy_from_slice(s.as_bytes());
    name
}

/// POC: `FatDirIter` loads a fixed FAT12/16 root directory into a buffer
/// capped at 4096 bytes (`remaining.min(4096)` in dir.rs) and, once the read
/// offset walks past the buffer end, returns `None` (end of directory)
/// instead of refilling. Root directories larger than 4096 bytes — the
/// standard 224-entry floppy root is 7168 bytes — silently lose every entry
/// past slot 128. Affects both the sync `Iterator` and the async
/// `next_entry` (shared source). This is observable on *valid* images.
#[test]
fn poc_fixed_root_iteration_drops_entries_past_4kib() {
    let mut img = fat12_image();
    for i in 0..FAT12_ROOT_ENTRIES {
        put_short_entry(&mut img, i, &filler_name(i), 0x20, 0);
    }
    put_short_entry(&mut img, 200, b"TARGET  TXT", 0x20, 0);

    let fs = hadris_fat::FatVolume::open(Cursor::new(img)).unwrap();
    let root = fs.root_dir();
    let mut count = 0usize;
    let mut found = false;
    for item in root.entries() {
        let item = item.unwrap();
        count += 1;
        if item.name() == "TARGET.TXT" {
            found = true;
        }
    }
    assert_eq!(
        count, FAT12_ROOT_ENTRIES,
        "BUG: fixed root iteration stopped early at {count} entries (buffer capped at 4096 bytes)"
    );
    assert!(
        found,
        "BUG: entry at root slot 200 is invisible to iteration/lookup"
    );
}

/// Async twin of the POC above: the shared `io_transform!` source produces
/// the same bug in `FatDirIter::next_entry`.
#[cfg(feature = "async")]
#[test]
fn poc_fixed_root_iteration_drops_entries_past_4kib_async() {
    use core::future::Future;
    use core::task::{Context, Poll};
    use std::sync::Arc;
    use std::task::{Wake, Waker};

    struct ThreadWaker(std::thread::Thread);
    impl Wake for ThreadWaker {
        fn wake(self: Arc<Self>) {
            self.0.unpark();
        }
    }
    fn block_on<F: Future>(future: F) -> F::Output {
        let waker = Waker::from(Arc::new(ThreadWaker(std::thread::current())));
        let mut context = Context::from_waker(&waker);
        let mut future = std::pin::pin!(future);
        loop {
            match future.as_mut().poll(&mut context) {
                Poll::Ready(output) => return output,
                Poll::Pending => std::thread::park(),
            }
        }
    }

    let mut img = fat12_image();
    for i in 0..FAT12_ROOT_ENTRIES {
        put_short_entry(&mut img, i, &filler_name(i), 0x20, 0);
    }
    put_short_entry(&mut img, 200, b"TARGET  TXT", 0x20, 0);

    block_on(async {
        let fs = hadris_fat::r#async::FatVolume::open(hadris_io::Cursor::new(&img))
            .await
            .unwrap();
        let root = fs.root_dir();
        let mut iter = root.entries();
        let mut count = 0usize;
        let mut found = false;
        while let Some(item) = iter.next_entry().await {
            let item = item.unwrap();
            count += 1;
            if item.name() == "TARGET.TXT" {
                found = true;
            }
        }
        assert_eq!(
            count, FAT12_ROOT_ENTRIES,
            "BUG: async fixed root iteration stopped early at {count} entries"
        );
        assert!(found, "BUG: async lookup misses entry at root slot 200");
    });
}

// ---------------------------------------------------------------------------
// exFAT image builder
// ---------------------------------------------------------------------------

#[cfg(feature = "unstable-exfat")]
mod exfat_img {
    pub const SECTOR: usize = 512;
    pub const CLUSTER: usize = 4096; // sectors_per_cluster_shift = 3
    pub const FAT_SECTOR: usize = 24;
    pub const HEAP_SECTOR: usize = 32;
    pub const HEAP_OFFSET: usize = HEAP_SECTOR * SECTOR;

    pub fn cluster_offset(cluster: u32) -> usize {
        HEAP_OFFSET + (cluster as usize - 2) * CLUSTER
    }

    pub struct Builder {
        pub cluster_count: u32,
        pub heap_clusters_present: u32,
        pub root_entries: Vec<[u8; 32]>,
        pub fat_entries: Vec<(u32, u32)>,
        pub cluster_fill: Vec<(u32, u8)>,
    }

    impl Builder {
        pub fn new(cluster_count: u32) -> Self {
            Self {
                cluster_count,
                heap_clusters_present: 8,
                root_entries: Vec::new(),
                fat_entries: Vec::new(),
                cluster_fill: Vec::new(),
            }
        }

        pub fn build(self) -> Vec<u8> {
            let mut img = vec![0u8; HEAP_OFFSET + self.heap_clusters_present as usize * CLUSTER];

            // Main boot sector
            img[0..3].copy_from_slice(&[0xEB, 0x76, 0x90]);
            img[3..11].copy_from_slice(b"EXFAT   ");
            img[64..72].copy_from_slice(&0u64.to_le_bytes()); // partition offset
            img[72..80].copy_from_slice(&4096u64.to_le_bytes()); // volume length
            img[80..84].copy_from_slice(&(FAT_SECTOR as u32).to_le_bytes()); // fat offset
            img[84..88].copy_from_slice(&1u32.to_le_bytes()); // fat length
            img[88..92].copy_from_slice(&(HEAP_SECTOR as u32).to_le_bytes()); // heap offset
            img[92..96].copy_from_slice(&self.cluster_count.to_le_bytes());
            img[96..100].copy_from_slice(&2u32.to_le_bytes()); // root cluster
            img[104..106].copy_from_slice(&0x0100u16.to_le_bytes()); // revision
            img[108] = 9; // bytes per sector shift
            img[109] = 3; // sectors per cluster shift
            img[110] = 1; // number of FATs
            img[510] = 0x55;
            img[511] = 0xAA;

            // FAT entries
            for (cluster, value) in &self.fat_entries {
                let off = FAT_SECTOR * SECTOR + *cluster as usize * 4;
                img[off..off + 4].copy_from_slice(&value.to_le_bytes());
            }

            // Cluster payloads
            for (cluster, byte) in &self.cluster_fill {
                let off = cluster_offset(*cluster);
                img[off..off + CLUSTER].fill(*byte);
            }

            // Root directory entries
            let root = cluster_offset(2);
            for (i, entry) in self.root_entries.iter().enumerate() {
                img[root + i * 32..root + (i + 1) * 32].copy_from_slice(entry);
            }

            // Boot checksum over sectors 0..=10, stored repeated in sector 11
            let mut checksum = 0u32;
            for sector in 0..11 {
                for i in 0..SECTOR {
                    let idx = sector * SECTOR + i;
                    if sector == 0 && (i == 106 || i == 107 || i == 112) {
                        continue;
                    }
                    checksum = checksum.rotate_right(1).wrapping_add(img[idx] as u32);
                }
            }
            for i in 0..SECTOR / 4 {
                img[11 * SECTOR + i * 4..11 * SECTOR + (i + 1) * 4]
                    .copy_from_slice(&checksum.to_le_bytes());
            }

            img
        }
    }

    pub fn bitmap_entry(first_cluster: u32, data_length: u64) -> [u8; 32] {
        let mut e = [0u8; 32];
        e[0] = 0x81;
        e[20..24].copy_from_slice(&first_cluster.to_le_bytes());
        e[24..32].copy_from_slice(&data_length.to_le_bytes());
        e
    }

    pub fn set_checksum(entries: &[[u8; 32]]) -> u16 {
        let mut sum = 0u16;
        for (i, e) in entries.iter().enumerate() {
            for (j, &b) in e.iter().enumerate() {
                if i == 0 && (j == 2 || j == 3) {
                    continue;
                }
                sum = sum.rotate_right(1).wrapping_add(b as u16);
            }
        }
        sum
    }

    /// A directory entry set for a fragmented (FAT-chained) directory named `name`.
    pub fn fragmented_dir_entry_set(name: &str, first_cluster: u32) -> Vec<[u8; 32]> {
        fragmented_entry_set(name, first_cluster, 0x10, 0)
    }

    /// An entry set for a fragmented (FAT-chained) file named `name`.
    pub fn fragmented_file_entry_set(
        name: &str,
        first_cluster: u32,
        valid_data_length: u64,
    ) -> Vec<[u8; 32]> {
        fragmented_entry_set(name, first_cluster, 0x20, valid_data_length)
    }

    fn fragmented_entry_set(
        name: &str,
        first_cluster: u32,
        attributes: u16,
        valid_data_length: u64,
    ) -> Vec<[u8; 32]> {
        let mut primary = [0u8; 32];
        primary[0] = 0x85;
        primary[1] = 2; // secondary count
        primary[4..6].copy_from_slice(&attributes.to_le_bytes());

        let mut stream = [0u8; 32];
        stream[0] = 0xC0;
        stream[1] = 0x01; // AllocationPossible set, NoFatChain clear => FAT chain
        stream[3] = name.encode_utf16().count() as u8;
        stream[8..16].copy_from_slice(&valid_data_length.to_le_bytes());
        stream[20..24].copy_from_slice(&first_cluster.to_le_bytes());
        stream[24..32].copy_from_slice(&valid_data_length.to_le_bytes());

        let mut name_entry = [0u8; 32];
        name_entry[0] = 0xC1;
        for (i, unit) in name.encode_utf16().enumerate() {
            name_entry[2 + i * 2..4 + i * 2].copy_from_slice(&unit.to_le_bytes());
        }

        let mut set = vec![primary, stream, name_entry];
        let sum = set_checksum(&set);
        set[0][2..4].copy_from_slice(&sum.to_le_bytes());
        set
    }
}

/// Regression: `AllocationBitmap::load` (exfat/bitmap.rs) must bound the
/// on-disk `data_length` (a full u64) against the cluster heap before
/// allocating, like the up-case table loader does. A ~24 KiB image with a
/// 64-cluster heap (256 KiB capacity) claiming a 256 MiB bitmap must be
/// rejected at validation, not after a 256 MiB allocation.
#[cfg(feature = "unstable-exfat")]
#[test]
fn poc_exfat_bitmap_data_length_drives_unbounded_allocation() {
    let _guard = alloc_watch::LOCK.lock().unwrap();
    let mut b = exfat_img::Builder::new(64);
    b.root_entries = vec![exfat_img::bitmap_entry(3, 256 * 1024 * 1024)];
    let img = b.build();

    alloc_watch::MAX_ALLOC.store(0, Ordering::SeqCst);
    let result = ExFatVolume::open(Cursor::new(img));
    assert!(
        matches!(result, Err(hadris_fat::Error::ExFatInvalidEntry { .. })),
        "oversized bitmap must be rejected with ExFatInvalidEntry, got {:?}",
        result.map(|_| ())
    );
    let requested = alloc_watch::MAX_ALLOC.load(Ordering::SeqCst);
    assert!(
        requested < 64 * 1024 * 1024,
        "bitmap data_length of 256 MiB on a 256 KiB heap drove a {requested}-byte allocation"
    );
}

/// Regression: `ExFatDirIter` and `ExFatFileReader` must bound FAT-chain
/// walks by the volume's cluster count and surface `ClusterLoop` (dir
/// iterator) / an I/O error (file reader) on a cyclic chain (3 -> 4 -> 3)
/// instead of spinning forever.
#[cfg(feature = "unstable-exfat")]
#[test]
fn poc_exfat_fragmented_dir_fat_cycle_hangs_iteration() {
    use hadris_fat::exfat::ExFatFileReader;
    use hadris_fat::io::Read as _;

    let mut b = exfat_img::Builder::new(64);
    let mut entries = exfat_img::fragmented_dir_entry_set("D", 3);
    entries.extend(exfat_img::fragmented_file_entry_set(
        "F",
        3,
        16 * 1024 * 1024,
    ));
    b.root_entries = entries;
    b.fat_entries = vec![(3, 4), (4, 3)];
    b.cluster_fill = vec![(3, 0x83), (4, 0x83)]; // volume-label entries: skipped, never END
    let img = b.build();

    let fs = ExFatVolume::open(Cursor::new(img)).unwrap();

    // Directory iterator: must error with ClusterLoop, not hang.
    let dir = fs.open_dir("/D").unwrap();
    let mut it = dir.entries();
    match it.next() {
        Some(Err(hadris_fat::Error::ClusterLoop { .. })) => {}
        other => panic!("expected ClusterLoop from cyclic dir chain, got {other:?}"),
    }

    // File reader: must error within cluster_count + slack reads, not stream
    // forever. Clusters 3/4 hold 0x83 bytes, so reads succeed until the
    // chain-step bound trips.
    let file = fs.open_path("/F").unwrap();
    let mut reader = ExFatFileReader::new(&fs, &file).unwrap();
    let mut buf = [0u8; 4096];
    let mut reads = 0u32;
    loop {
        match reader.read(&mut buf) {
            Ok(0) => panic!("reader hit EOF on a cyclic chain instead of erroring"),
            Ok(_) => {
                reads += 1;
                assert!(
                    reads <= 70,
                    "reader did not stop after {reads} cluster reads"
                );
            }
            Err(_) => break,
        }
    }
}

/// Regression: `AllocationBitmap::free_cluster_count` must not underflow
/// when the bitmap is shorter than the claimed cluster count.
#[cfg(feature = "unstable-exfat")]
#[test]
fn poc_exfat_free_cluster_count_subtract_overflow() {
    let mut b = exfat_img::Builder::new(1024);
    b.root_entries = vec![exfat_img::bitmap_entry(3, 1)]; // 1-byte bitmap, 1024 clusters
    let img = b.build();

    let fs = ExFatVolume::open(Cursor::new(img)).unwrap();
    let n = fs.free_cluster_count(); // used to panic: attempt to subtract with overflow
    assert!(n <= 1024);
}

/// Regression: bitmap cluster-bound arithmetic must saturate at
/// cluster_count = u32::MAX instead of overflowing (the boot.rs/fat.rs side
/// of this class was fixed in 3fda34c; bitmap.rs was missed).
#[cfg(feature = "unstable-exfat")]
#[test]
fn poc_exfat_bitmap_validate_cluster_add_overflow() {
    let mut b = exfat_img::Builder::new(u32::MAX);
    b.heap_clusters_present = 4;
    let img = b.build();

    let fs = ExFatVolume::open(Cursor::new(img)).unwrap();
    // No bitmap was loaded, so the (valid) cluster lookup must error, not panic.
    assert!(fs.is_cluster_allocated(2).is_err());
}

// ---------------------------------------------------------------------------
// FAT32 image builder (for the tool-feature scan_fat POC)
// ---------------------------------------------------------------------------

#[cfg(feature = "tool")]
fn fat32_image_claiming_sectors(total_sectors: u32) -> Vec<u8> {
    let mut img = vec![0u8; 64 * 512];
    img[0..3].copy_from_slice(&[0xEB, 0x58, 0x90]);
    img[3..11].copy_from_slice(b"MSDOS5.0");
    img[11..13].copy_from_slice(&512u16.to_le_bytes());
    img[13] = 1; // sectors per cluster
    img[14..16].copy_from_slice(&32u16.to_le_bytes()); // reserved
    img[16] = 1; // fats
    img[21] = 0xF8;
    img[32..36].copy_from_slice(&total_sectors.to_le_bytes()); // totsec32
    img[36..40].copy_from_slice(&100u32.to_le_bytes()); // fatsz32 (implausibly small; not validated)
    img[42..44].copy_from_slice(&0u16.to_le_bytes()); // version
    img[44..48].copy_from_slice(&2u32.to_le_bytes()); // root cluster
    img[48..50].copy_from_slice(&1u16.to_le_bytes()); // fsinfo sector
    img[66] = 0x29;
    img[72..83].copy_from_slice(b"NO NAME    ");
    img[83..91].copy_from_slice(b"FAT32   ");
    img[510] = 0x55;
    img[511] = 0xAA;
    // FSInfo at sector 1
    img[512..516].copy_from_slice(&0x4161_5252u32.to_le_bytes());
    img[512 + 484..512 + 488].copy_from_slice(&0x6141_7272u32.to_le_bytes());
    img[512 + 508..512 + 512].copy_from_slice(&0xAA55_0000u32.to_le_bytes());
    img
}

/// Regression: `FatAnalysisExt::scan_fat` (tool/analysis.rs) must cap its
/// up-front Vec reservation — `max_cluster` derives from the untrusted BPB
/// total_sectors, so a 32 KiB image claiming ~50 million clusters must not
/// drive a ~400 MB allocation request.
#[cfg(feature = "tool")]
#[test]
fn poc_scan_fat_preallocates_from_claimed_geometry() {
    use hadris_fat::{FatAnalysisExt, FatVolume};

    let _guard = alloc_watch::LOCK.lock().unwrap();
    let img = fat32_image_claiming_sectors(50_000_064);
    let fs = FatVolume::open(Cursor::new(img)).unwrap();

    alloc_watch::MAX_ALLOC.store(0, Ordering::SeqCst);
    // Errors with Io once reads pass the end of the image; must not panic or OOM.
    assert!(fs.scan_fat().is_err());
    let requested = alloc_watch::MAX_ALLOC.load(Ordering::SeqCst);
    assert!(
        requested < 64 * 1024 * 1024,
        "scan_fat requested {requested} bytes of capacity from claimed BPB geometry on a 32 KiB image"
    );
}
