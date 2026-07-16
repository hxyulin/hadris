//! Integration tests for the FAT-sector cache.
//!
//! These tests cover the cache MVP added in commit b4630d6 and refined
//! alongside this commit:
//!   * Round-trip parity: cached chain walks return what the disk stores.
//!   * Persistence: cached writes plus `flush()` round-trip through a fresh
//!     mount.
//!   * Dirty eviction: a full-cache write across more sectors than the
//!     capacity must NOT lose dirty bytes (regression for the silent-drop
//!     hole called out in cache.rs:277-280 before this fix).
//!   * Read-path safety: a read that would need to evict a dirty sector
//!     surfaces `FatError::CacheDirtyEviction` rather than dropping data.
//!   * Robustness: planted FAT cycles surface as `ClusterLoop`; bad-cluster
//!     and out-of-range entries surface their respective errors.
//!   * Builder ergonomics: `with_fat_cache(0)` is a silent no-op.
//!
//! Together this answers the recurring "how do I actually use the cache?"
//! question (issue #27) by exercising the recommended `FatFs::with_cached_fat`
//! entry point end-to-end.

#![cfg(all(feature = "cache", feature = "write", feature = "std"))]

use std::io::Cursor;

use hadris_fat::format::{FatTypeSelection, FatVolumeFormatter, FormatOptions};
use hadris_fat::{FatError, FatFs, FatFsReadExt};

const FAT32_SIZE: usize = 40 * 1024 * 1024;

/// FAT byte-layout pieces we need to patch entries by hand. Each FAT type
/// uses the same shape: `fat_start` is the byte offset of FAT[0]; `fat_size`
/// is the size of one FAT copy; `fat_count` is the number of copies.
struct FatLayout {
    fat_start: usize,
    fat_size: usize,
    fat_count: usize,
    sector_size: usize,
}

impl FatLayout {
    fn new(opts: &FormatOptions) -> Self {
        let params = FatVolumeFormatter::calculate_params(opts).expect("calc params");
        let sector_size = params.sector_size;
        Self {
            fat_start: params.reserved_sectors as usize * sector_size,
            fat_size: params.sectors_per_fat as usize * sector_size,
            fat_count: params.fat_count as usize,
            sector_size,
        }
    }

    fn fat32_entry_offset(&self, copy: usize, cluster: u32) -> usize {
        self.fat_start + copy * self.fat_size + cluster as usize * 4
    }

    /// Sector number (within FAT[0]) that contains FAT32 `cluster`.
    fn fat32_sector_of(&self, cluster: u32) -> usize {
        (cluster as usize * 4) / self.sector_size
    }
}

/// Format a FAT image into the supplied buffer and immediately drop the
/// returned filesystem so the buffer is once again accessible.
fn format_into_buffer(buffer: &mut [u8], opts: &FormatOptions) -> FatLayout {
    let layout = FatLayout::new(opts);
    {
        let cursor = Cursor::new(&mut buffer[..]);
        let _fs = FatVolumeFormatter::format(cursor, opts.clone()).expect("format");
        // _fs drops here, releasing the &mut [u8] borrow.
    }
    layout
}

/// Patch a FAT32 entry in *both* FAT copies. Without the second copy,
/// FatFs's fallback FAT-comparison path could mask our planted value.
fn patch_fat32_entry_all_copies(buffer: &mut [u8], layout: &FatLayout, cluster: u32, value: u32) {
    let bytes = value.to_le_bytes();
    for copy in 0..layout.fat_count {
        let off = layout.fat32_entry_offset(copy, cluster);
        buffer[off..off + 4].copy_from_slice(&bytes);
    }
}

fn read_fat32_entry_raw(buffer: &[u8], layout: &FatLayout, cluster: u32) -> u32 {
    let off = layout.fat32_entry_offset(0, cluster);
    u32::from_le_bytes(buffer[off..off + 4].try_into().unwrap())
}

fn fat32_options() -> FormatOptions {
    FormatOptions::new(FAT32_SIZE as u64).fat_type(FatTypeSelection::Fat32)
}

// =============================================================================
// Round-trip parity
// =============================================================================

#[test]
fn cache_round_trips_fat32_chain() {
    // Plant a 3-cluster chain (5 -> 6 -> 7 -> END), read it via the cache,
    // and assert we got every cluster in order. This validates the read
    // path through `CachedFat::next_cluster` end-to-end.
    let mut bytes = vec![0u8; FAT32_SIZE];
    let opts = fat32_options();
    let layout = format_into_buffer(&mut bytes, &opts);

    patch_fat32_entry_all_copies(&mut bytes, &layout, 5, 6);
    patch_fat32_entry_all_copies(&mut bytes, &layout, 6, 7);
    patch_fat32_entry_all_copies(&mut bytes, &layout, 7, 0x0FFF_FFFF); // END

    let cursor = Cursor::new(&mut bytes[..]);
    let fs = FatFs::builder(cursor).fat_cache(8).open().expect("open");

    let chain = fs
        .with_cached_fat(|cached, disk| cached.read_chain(disk, 5))
        .expect("cache installed")
        .expect("read_chain ok");
    assert_eq!(chain, vec![5, 6, 7]);
}

// =============================================================================
// Persistence
// =============================================================================

#[test]
fn cache_writes_persist_across_remount_after_flush() {
    let mut bytes = vec![0u8; FAT32_SIZE];
    let opts = fat32_options();
    let layout = format_into_buffer(&mut bytes, &opts);

    // Open with cache, write a recognizable value via the cache, flush, drop.
    {
        let cursor = Cursor::new(&mut bytes[..]);
        let fs = FatFs::builder(cursor).fat_cache(8).open().expect("open");
        fs.with_fat_cache_locked(|cache, disk| {
            cache
                .write_fat32_entry(disk, 100, 0x0BEE_F123)
                .expect("write_fat32_entry");
        })
        .expect("with_fat_cache_locked");
        fs.flush().expect("flush");
    }

    // Re-mount without a cache and verify the on-disk byte pattern matches.
    let observed = read_fat32_entry_raw(&bytes, &layout, 100);
    assert_eq!(
        observed & 0x0FFF_FFFF,
        0x0BEE_F123,
        "post-flush FAT[100] must persist what we wrote through the cache"
    );

    // (The byte-level assert above already proves persistence; an
    // additional FatFs round-trip would be redundant.)
    let _ = FatFs::open(Cursor::new(&bytes[..])).expect("re-open after flush");
}

// =============================================================================
// Dirty eviction (regression test)
// =============================================================================

/// REGRESSION TEST for the silent dirty-data loss documented in
/// cache.rs:277-280 prior to this commit. Strategy:
///   * Use a small cache (capacity 2).
///   * Write FAT entries spanning *three* distinct FAT sectors. The third
///     write must trigger LRU eviction of the first sector, which is dirty.
///   * The fix flushes that sector through to disk on eviction. The pre-fix
///     code dropped it on the floor.
///   * Then call `fs.flush()` for the remaining two sectors.
///   * Re-mount and verify all three planted values made it to disk.
#[test]
fn cache_dirty_eviction_does_not_lose_data() {
    let mut bytes = vec![0u8; FAT32_SIZE];
    let opts = fat32_options();
    let layout = format_into_buffer(&mut bytes, &opts);

    // Pick clusters in three different FAT sectors. With sector_size = 512
    // and 4 bytes per FAT32 entry, sector N covers clusters [128N, 128N+128).
    let writes: &[(u32, u32)] = &[
        (10, 0x0AAA_AAAA),  // sector 0
        (200, 0x0BBB_BBBB), // sector 1
        (400, 0x0CCC_CCCC), // sector 3
    ];
    // Sanity: confirm those are in distinct sectors.
    let s0 = layout.fat32_sector_of(writes[0].0);
    let s1 = layout.fat32_sector_of(writes[1].0);
    let s2 = layout.fat32_sector_of(writes[2].0);
    assert_ne!(s0, s1);
    assert_ne!(s1, s2);
    assert_ne!(s0, s2);

    {
        let cursor = Cursor::new(&mut bytes[..]);
        let fs = FatFs::builder(cursor)
            .fat_cache(2) // < number of sectors written
            .open()
            .expect("open");
        fs.with_fat_cache_locked(|cache, disk| {
            for &(cluster, value) in writes {
                cache
                    .write_fat32_entry(disk, cluster as usize, value)
                    .expect("write_fat32_entry");
            }
            // At this point cap=2 is exceeded, so the first sector got
            // evicted on insert of the third. The fix flushes on eviction;
            // the bug silently dropped the dirty bytes.
            assert!(cache.stats().evictions >= 1);
            assert!(cache.stats().dirty_writes >= 1);
        })
        .expect("with_fat_cache_locked");
        // Flush the remaining (still-dirty) cache contents.
        fs.flush().expect("flush");
    }

    // All three planted values must be on disk now. If the bug were present,
    // the first write (sector 0) would be missing.
    for &(cluster, value) in writes {
        let observed = read_fat32_entry_raw(&bytes, &layout, cluster);
        assert_eq!(
            observed & 0x0FFF_FFFF,
            value & 0x0FFF_FFFF,
            "FAT[{cluster}] must persist value 0x{value:08x}"
        );
    }
}

#[test]
fn cache_dirty_eviction_writes_to_all_fat_copies() {
    // Same setup as the regression test, but we additionally check FAT[1]
    // (the backup copy) — a write-through eviction must mirror to every
    // copy, otherwise fsck would flag a FAT mismatch.
    let mut bytes = vec![0u8; FAT32_SIZE];
    let opts = fat32_options();
    let layout = format_into_buffer(&mut bytes, &opts);
    assert!(
        layout.fat_count >= 2,
        "this test assumes the formatter writes >= 2 FAT copies"
    );

    let cluster = 50u32;
    let value = 0x0DEA_DBEE_u32 & 0x0FFF_FFFF;

    {
        let cursor = Cursor::new(&mut bytes[..]);
        let fs = FatFs::builder(cursor)
            .fat_cache(1) // cap 1 forces eviction on every new sector
            .open()
            .expect("open");
        fs.with_fat_cache_locked(|cache, disk| {
            cache
                .write_fat32_entry(disk, cluster as usize, value)
                .expect("write");
            // Force eviction by writing a different sector.
            cache
                .write_fat32_entry(disk, 200, 0)
                .expect("write triggers eviction");
        })
        .expect("with_fat_cache_locked");
        fs.flush().expect("flush");
    }

    // Both FAT copies must contain our value.
    for copy in 0..layout.fat_count {
        let off = layout.fat32_entry_offset(copy, cluster);
        let observed = u32::from_le_bytes(bytes[off..off + 4].try_into().unwrap());
        assert_eq!(
            observed & 0x0FFF_FFFF,
            value,
            "FAT copy {copy} at cluster {cluster} must reflect cached write"
        );
    }
}

// =============================================================================
// Read-path safety
// =============================================================================

/// When the cache is full of dirty entries and a fresh *read* would have to
/// evict one, we must surface `CacheDirtyEviction` rather than silently
/// dropping the dirty bytes. The user can then call `flush()` and retry.
#[test]
fn cache_read_returns_cache_dirty_eviction_when_all_dirty() {
    let mut bytes = vec![0u8; FAT32_SIZE];
    let opts = fat32_options();
    let _layout = format_into_buffer(&mut bytes, &opts);

    let cursor = Cursor::new(&mut bytes[..]);
    let fs = FatFs::builder(cursor).fat_cache(2).open().expect("open");

    fs.with_fat_cache_locked(|cache, disk| {
        // Fill cache to capacity with dirty writes (two distinct sectors).
        cache.write_fat32_entry(disk, 10, 0x0AAA_AAAA).unwrap();
        cache.write_fat32_entry(disk, 200, 0x0BBB_BBBB).unwrap();
        // Now the cache holds two dirty entries. A read of a *third* sector
        // would have to evict one, but read paths can't drop dirty data.
        let err = cache.read_fat32_entry(disk, 400).unwrap_err();
        match err {
            FatError::CacheDirtyEviction { .. } => {}
            other => panic!("expected CacheDirtyEviction, got {other:?}"),
        }
    })
    .expect("with_fat_cache_locked");

    // After flushing, the same read should succeed.
    fs.flush().expect("flush");
    fs.with_fat_cache_locked(|cache, disk| {
        let _val = cache.read_fat32_entry(disk, 400).expect("read after flush");
    })
    .expect("with_fat_cache_locked");
}

// =============================================================================
// Robustness — corruption surfacing
// =============================================================================

#[test]
fn cached_fat_read_chain_returns_cluster_loop_on_cycle() {
    let mut bytes = vec![0u8; FAT32_SIZE];
    let opts = fat32_options();
    let layout = format_into_buffer(&mut bytes, &opts);

    // FAT[3] -> 4, FAT[4] -> 3 is a 2-cycle.
    patch_fat32_entry_all_copies(&mut bytes, &layout, 3, 4);
    patch_fat32_entry_all_copies(&mut bytes, &layout, 4, 3);

    let cursor = Cursor::new(&mut bytes[..]);
    let fs = FatFs::builder(cursor).fat_cache(4).open().expect("open");

    let result = fs
        .with_cached_fat(|cached, disk| cached.read_chain(disk, 3))
        .expect("cache installed");
    match result {
        Err(FatError::ClusterLoop { .. }) => {}
        Err(other) => panic!("expected ClusterLoop, got {other:?}"),
        Ok(chain) => panic!("expected ClusterLoop, got chain {chain:?}"),
    }
}

#[test]
fn cached_fat_next_cluster_on_bad_cluster_marker() {
    let mut bytes = vec![0u8; FAT32_SIZE];
    let opts = fat32_options();
    let layout = format_into_buffer(&mut bytes, &opts);

    // 0x0FFF_FFF7 is the FAT32 BadCluster marker.
    patch_fat32_entry_all_copies(&mut bytes, &layout, 5, 0x0FFF_FFF7);

    let cursor = Cursor::new(&mut bytes[..]);
    let fs = FatFs::builder(cursor).fat_cache(4).open().expect("open");

    let result = fs
        .with_cached_fat(|cached, disk| cached.next_cluster(disk, 5))
        .expect("cache installed");
    match result {
        Err(FatError::BadCluster { cluster }) => assert_eq!(cluster, 5),
        other => panic!("expected BadCluster, got {other:?}"),
    }
}

#[test]
fn cached_fat_next_cluster_out_of_bounds() {
    let mut bytes = vec![0u8; FAT32_SIZE];
    let opts = fat32_options();
    let layout = format_into_buffer(&mut bytes, &opts);

    // Plant an entry that points way past the end of the data area.
    patch_fat32_entry_all_copies(&mut bytes, &layout, 5, 0x0FFF_0000);

    let cursor = Cursor::new(&mut bytes[..]);
    let fs = FatFs::builder(cursor).fat_cache(4).open().expect("open");

    let result = fs
        .with_cached_fat(|cached, disk| cached.next_cluster(disk, 5))
        .expect("cache installed");
    match result {
        Err(FatError::ClusterOutOfBounds { .. }) => {}
        other => panic!("expected ClusterOutOfBounds, got {other:?}"),
    }
}

// =============================================================================
// Transparent wiring (Phase C5)
// =============================================================================

/// Phase C5 contract: built-in `FatFs` operations consult the installed cache.
///
/// Before C5, `FatFs::read_status_flags` and chain walks done by `read_file`
/// seeked the disk for every FAT-entry access; the cache's hit counter stayed
/// at 0 unless callers used `with_cached_fat` explicitly. This test pins the
/// new behaviour: two consecutive operations that read the same FAT sector
/// register a hit on the second call, proving that internal FAT-table reads
/// now route through the cache.
#[test]
fn read_status_flags_consults_cache() {
    let mut bytes = vec![0u8; FAT32_SIZE];
    let opts = fat32_options();
    let _layout = format_into_buffer(&mut bytes, &opts);

    let cursor = Cursor::new(&mut bytes[..]);
    let fs = FatFs::builder(cursor).fat_cache(16).open().expect("open");

    fs.with_fat_cache_locked(|cache, _| cache.reset_stats())
        .expect("cache installed");

    // Two reads of FAT[1]: the first is a miss (loads the sector), the second
    // is a hit (sector already cached). Pre-C5 both went directly to disk and
    // bypassed the cache entirely.
    let _ = fs.read_status_flags().expect("read_status_flags 1");
    let _ = fs.read_status_flags().expect("read_status_flags 2");

    let stats = fs
        .with_fat_cache_locked(|cache, _| cache.stats())
        .expect("cache installed");
    assert!(
        stats.hits >= 1,
        "expected at least one cache hit after two read_status_flags() calls, got stats {stats:?}"
    );
    assert!(
        stats.misses >= 1,
        "expected at least one cache miss seeding the sector, got stats {stats:?}"
    );
}

/// End-to-end transparency: `read_file` walks the on-disk FAT chain using
/// `next_cluster_routed`, so a file spanning multiple clusters triggers
/// cache reads — first a miss to seed the FAT sector, then hits on every
/// subsequent step.
///
/// Setup is done in two passes so the on-disk FAT is fully written before the
/// cache is observed: pass 1 formats and writes a multi-cluster file; pass 2
/// re-opens with a cache, resets stats, and reads the file back.
#[test]
fn read_file_chain_walk_consults_cache() {
    use hadris_fat::FatFsWriteExt;

    let mut bytes = vec![0u8; FAT32_SIZE];
    let opts = fat32_options();
    let _layout = format_into_buffer(&mut bytes, &opts);

    // Pass 1: create and populate the file with no cache, so all FAT
    // mutations land directly on disk. Pick a payload large enough to span
    // many clusters at any reasonable cluster size for a 40 MiB FAT32 volume
    // (typically 512 B; 64 KiB safely guarantees ≥ 4 clusters even at 16 KiB).
    let payload_len: usize = 64 * 1024;
    {
        let cursor = Cursor::new(&mut bytes[..]);
        let fs = FatFs::open(cursor).expect("open");
        let payload = vec![0xABu8; payload_len];

        let root = fs.root_dir();
        let entry = fs.create_file(&root, "BIG.BIN").expect("create_file");
        let mut writer = fs.write_file(&entry).expect("write_file");
        writer.write(&payload).expect("write");
        writer.finish().expect("finish writer");
    }

    // Pass 2: re-open with a cache, reset its stats, and read.
    let cursor = Cursor::new(&mut bytes[..]);
    let fs = FatFs::builder(cursor)
        .fat_cache(16)
        .open()
        .expect("open with cache");

    fs.with_fat_cache_locked(|cache, _| cache.reset_stats())
        .expect("cache installed");

    let entry = fs
        .root_dir()
        .find("BIG.BIN")
        .expect("find ok")
        .expect("find Some");
    let mut reader = fs.read_file(&entry).expect("read_file");
    let buf = reader.read_to_vec().expect("read_to_vec");
    assert_eq!(buf.len(), payload_len);

    let stats = fs
        .with_fat_cache_locked(|cache, _| cache.stats())
        .expect("cache installed");
    assert!(
        stats.hits > 0,
        "expected cache hits when read_file walked a multi-cluster chain, got {stats:?}"
    );
    assert!(
        stats.misses >= 1,
        "expected at least one miss seeding the FAT sector, got {stats:?}"
    );
}

/// Phase C5 correctness: writes performed while the cache is installed must
/// be visible to subsequent cached reads. Before write routing this would
/// silently return stale FAT bytes whenever the cache had pre-loaded the
/// sector that was then modified directly on disk.
#[test]
fn writes_then_reads_through_cache_are_consistent() {
    use hadris_fat::FatFsWriteExt;

    let mut bytes = vec![0u8; FAT32_SIZE];
    let opts = fat32_options();
    let _layout = format_into_buffer(&mut bytes, &opts);

    let cursor = Cursor::new(&mut bytes[..]);
    let fs = FatFs::builder(cursor)
        .fat_cache(16)
        .open()
        .expect("open with cache");

    let payload = vec![0xCDu8; 64 * 1024];

    // Touch the FAT via a read so the relevant sectors are pre-loaded into
    // cache before any writes — this is the scenario that produces stale
    // entries if writes bypass the cache.
    let _ = fs.read_status_flags().expect("read_status_flags");
    let root = fs.root_dir();
    let _ = root.find("does_not_exist").expect("find ok");

    // Now create + write a multi-cluster file with the cache active.
    let entry = fs.create_file(&root, "DATA.BIN").expect("create_file");
    {
        let mut writer = fs.write_file(&entry).expect("write_file");
        writer.write(&payload).expect("write");
        writer.finish().expect("finish");
    }
    fs.flush().expect("flush cache");

    // Read it back through the cache — must return the bytes we just wrote.
    let found = root.find("DATA.BIN").expect("find ok").expect("find Some");
    let mut reader = fs.read_file(&found).expect("read_file");
    let observed = reader.read_to_vec().expect("read_to_vec");
    assert_eq!(observed, payload, "cached read must reflect cached writes");
}

// =============================================================================
// Builder ergonomics
// =============================================================================

#[test]
fn with_fat_cache_zero_treats_as_no_cache() {
    let mut bytes = vec![0u8; FAT32_SIZE];
    let opts = fat32_options();
    let _layout = format_into_buffer(&mut bytes, &opts);

    let cursor = Cursor::new(&mut bytes[..]);
    let fs = FatFs::builder(cursor).fat_cache(0).open().expect("open");
    assert!(
        fs.fat_cache().is_none(),
        "with_fat_cache(0) must install no cache"
    );
    assert!(
        fs.with_cached_fat(|_, _| ()).is_none(),
        "with_cached_fat must return None when no cache is installed"
    );
}

#[test]
fn cache_stats_increment_on_hit_miss_eviction() {
    let mut bytes = vec![0u8; FAT32_SIZE];
    let opts = fat32_options();
    let layout = format_into_buffer(&mut bytes, &opts);

    // Plant valid mid-chain entries so next_cluster returns Ok(Some(_)).
    patch_fat32_entry_all_copies(&mut bytes, &layout, 5, 6);
    patch_fat32_entry_all_copies(&mut bytes, &layout, 6, 7);
    patch_fat32_entry_all_copies(&mut bytes, &layout, 7, 8);
    patch_fat32_entry_all_copies(&mut bytes, &layout, 8, 0x0FFF_FFFF);

    let cursor = Cursor::new(&mut bytes[..]);
    let fs = FatFs::builder(cursor).fat_cache(2).open().expect("open");

    // First chain walk: all misses, no evictions yet (clusters 5..8 fall
    // in the same FAT sector for these small numbers).
    let _ = fs
        .with_cached_fat(|cached, disk| cached.read_chain(disk, 5))
        .expect("cache installed")
        .expect("chain ok");
    let stats_after_walk = fs
        .with_fat_cache_locked(|cache, _| cache.stats())
        .expect("locked");
    assert!(
        stats_after_walk.misses >= 1,
        "first chain walk should record at least one miss"
    );

    // Second walk over the same chain: every lookup hits the cache.
    let _ = fs
        .with_cached_fat(|cached, disk| cached.read_chain(disk, 5))
        .expect("cache installed")
        .expect("chain ok");
    let stats_after_replay = fs
        .with_fat_cache_locked(|cache, _| cache.stats())
        .expect("locked");
    assert!(
        stats_after_replay.hits > stats_after_walk.hits,
        "replaying the chain must register additional cache hits"
    );

    // Force eviction by reading sectors beyond the cache's capacity. The
    // first walk consumed one sector (clusters 5..=8 share a sector); now
    // pull in two more — capacity is 2, so the third pull evicts.
    let prev_evictions = stats_after_replay.evictions;
    fs.with_fat_cache_locked(|cache, disk| {
        for cluster in [200u32, 400, 600] {
            let _ = cache.read_fat32_entry(disk, cluster as usize);
        }
    })
    .expect("with_fat_cache_locked");
    let stats_after_evict = fs
        .with_fat_cache_locked(|cache, _| cache.stats())
        .expect("locked");
    assert!(
        stats_after_evict.evictions > prev_evictions,
        "exceeding capacity must register at least one eviction (had {prev_evictions}, now {})",
        stats_after_evict.evictions
    );
}
