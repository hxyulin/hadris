//! Regression tests for the 2026-08 hadris-fat correctness audit.
//!
//! Each test asserts the CORRECT/SAFE behavior and is named after the audit
//! finding it guards. Single-threaded only (`FatVolume` is `!Sync`).

#![cfg(all(feature = "write", feature = "std"))]

use hadris_fat::raw::DirEntryAttrFlags;
use hadris_fat::time::FatDateTime;
use hadris_fat::write::FileWriter;
use hadris_fat::{Error, FatVolume, FatVolumeReadExt, FatVolumeWriteExt};

mod fat32_image {
    use std::io::Cursor;
    const SECTOR_SIZE: usize = 512;
    const SECTORS_PER_CLUSTER: u8 = 1;
    const CLUSTER_SIZE: usize = SECTOR_SIZE * SECTORS_PER_CLUSTER as usize;
    const RESERVED_SECTORS: u16 = 32;
    const FAT_COUNT: u8 = 2;
    const SECTORS_PER_FAT: u32 = 128;
    const ROOT_CLUSTER: u32 = 2;
    const FSINFO_LEAD_SIG: u32 = 0x41615252;
    const FSINFO_STRUC_SIG: u32 = 0x61417272;
    const FSINFO_TRAIL_SIG: u32 = 0xAA550000;
    pub const TOTAL_DATA_CLUSTERS: u32 = 256;

    pub fn create_fat32_image() -> Cursor<Vec<u8>> {
        let data_start_sector = RESERVED_SECTORS as u32 + FAT_COUNT as u32 * SECTORS_PER_FAT;
        let total_sectors = data_start_sector + TOTAL_DATA_CLUSTERS;
        let total_size = total_sectors as usize * SECTOR_SIZE;
        let mut image = vec![0u8; total_size];
        write_boot_sector(&mut image);
        write_fsinfo_sector(&mut image, TOTAL_DATA_CLUSTERS - 1);
        let fat_start = RESERVED_SECTORS as usize * SECTOR_SIZE;
        write_fat_table(&mut image, fat_start);
        let fat2_start = fat_start + SECTORS_PER_FAT as usize * SECTOR_SIZE;
        write_fat_table(&mut image, fat2_start);
        Cursor::new(image)
    }

    fn write_boot_sector(image: &mut [u8]) {
        image[0] = 0xEB;
        image[1] = 0x58;
        image[2] = 0x90;
        image[3..11].copy_from_slice(b"HADRISFT");
        image[11..13].copy_from_slice(&(SECTOR_SIZE as u16).to_le_bytes());
        image[13] = SECTORS_PER_CLUSTER;
        image[14..16].copy_from_slice(&RESERVED_SECTORS.to_le_bytes());
        image[16] = FAT_COUNT;
        image[17..19].copy_from_slice(&0u16.to_le_bytes());
        image[19..21].copy_from_slice(&0u16.to_le_bytes());
        image[21] = 0xF8;
        image[22..24].copy_from_slice(&0u16.to_le_bytes());
        image[24..26].copy_from_slice(&63u16.to_le_bytes());
        image[26..28].copy_from_slice(&255u16.to_le_bytes());
        image[28..32].copy_from_slice(&0u32.to_le_bytes());
        let total_sectors = RESERVED_SECTORS as u32 + FAT_COUNT as u32 * SECTORS_PER_FAT + 256;
        image[32..36].copy_from_slice(&total_sectors.to_le_bytes());
        image[36..40].copy_from_slice(&SECTORS_PER_FAT.to_le_bytes());
        image[40..42].copy_from_slice(&0u16.to_le_bytes());
        image[42..44].copy_from_slice(&0u16.to_le_bytes());
        image[44..48].copy_from_slice(&ROOT_CLUSTER.to_le_bytes());
        image[48..50].copy_from_slice(&1u16.to_le_bytes());
        image[50..52].copy_from_slice(&6u16.to_le_bytes());
        image[64] = 0x80;
        image[66] = 0x29;
        image[67..71].copy_from_slice(&0x12345678u32.to_le_bytes());
        image[71..82].copy_from_slice(b"TEST       ");
        image[82..90].copy_from_slice(b"FAT32   ");
        image[510] = 0x55;
        image[511] = 0xAA;
    }

    fn write_fsinfo_sector(image: &mut [u8], free_clusters: u32) {
        let offset = SECTOR_SIZE;
        image[offset..offset + 4].copy_from_slice(&FSINFO_LEAD_SIG.to_le_bytes());
        image[offset + 484..offset + 488].copy_from_slice(&FSINFO_STRUC_SIG.to_le_bytes());
        image[offset + 488..offset + 492].copy_from_slice(&free_clusters.to_le_bytes());
        image[offset + 492..offset + 496].copy_from_slice(&3u32.to_le_bytes());
        image[offset + 508..offset + 512].copy_from_slice(&FSINFO_TRAIL_SIG.to_le_bytes());
    }

    fn write_fat_table(image: &mut [u8], fat_start: usize) {
        image[fat_start..fat_start + 4].copy_from_slice(&0x0FFFFFF8u32.to_le_bytes());
        image[fat_start + 4..fat_start + 8].copy_from_slice(&0x0FFFFFFFu32.to_le_bytes());
        image[fat_start + 8..fat_start + 12].copy_from_slice(&0x0FFFFFF8u32.to_le_bytes());
    }

    pub const fn cluster_size() -> usize {
        CLUSTER_SIZE
    }
    pub const fn fat_start_bytes() -> usize {
        RESERVED_SECTORS as usize * SECTOR_SIZE
    }
    pub const fn data_start_bytes() -> usize {
        (RESERVED_SECTORS as usize + FAT_COUNT as usize * SECTORS_PER_FAT as usize) * SECTOR_SIZE
    }

    /// Count non-free FAT32 entries (index >= 2) in the primary FAT.
    /// A "free" entry is 0x0000000 (after masking off the top 4 bits).
    #[allow(dead_code)] // used by TOCTOU tests added in a later commit
    pub fn allocated_fat_entries(image: &[u8]) -> usize {
        let fat_start = fat_start_bytes();
        let mut allocated = 0;
        for i in 2..(2 + TOTAL_DATA_CLUSTERS as usize) {
            let off = fat_start + i * 4;
            let raw =
                u32::from_le_bytes([image[off], image[off + 1], image[off + 2], image[off + 3]])
                    & 0x0FFF_FFFF;
            if raw != 0 {
                allocated += 1;
            }
        }
        allocated
    }

    /// Read the 28-bit FAT32 entry for `cluster` from the primary FAT.
    #[allow(dead_code)] // used by TOCTOU tests added in a later commit
    pub fn fat_entry(image: &[u8], cluster: usize) -> u32 {
        let off = fat_start_bytes() + cluster * 4;
        u32::from_le_bytes([image[off], image[off + 1], image[off + 2], image[off + 3]])
            & 0x0FFF_FFFF
    }
}

// ---------------------------------------------------------------------------
// FAT16 image builder (copied from tests/test_write.rs::fat16_image)
// ---------------------------------------------------------------------------
mod fat16_image {
    use std::io::Cursor;
    const SECTOR_SIZE: usize = 512;
    const SECTORS_PER_CLUSTER: u8 = 1;
    const RESERVED_SECTORS: u16 = 1;
    const FAT_COUNT: u8 = 2;
    const SECTORS_PER_FAT: u16 = 32;
    pub const ROOT_ENTRY_COUNT: u16 = 512;

    pub fn create_fat16_image() -> Cursor<Vec<u8>> {
        let root_dir_sectors = (ROOT_ENTRY_COUNT as usize * 32).div_ceil(SECTOR_SIZE);
        let data_start_sector = RESERVED_SECTORS as usize
            + FAT_COUNT as usize * SECTORS_PER_FAT as usize
            + root_dir_sectors;
        let total_data_clusters: usize = 8192;
        let total_sectors = data_start_sector + total_data_clusters;
        let total_size = total_sectors * SECTOR_SIZE;
        let mut image = vec![0u8; total_size];
        write_boot_sector(&mut image, total_sectors as u32);
        let fat_start = RESERVED_SECTORS as usize * SECTOR_SIZE;
        write_fat_table(&mut image, fat_start);
        let fat2_start = fat_start + SECTORS_PER_FAT as usize * SECTOR_SIZE;
        write_fat_table(&mut image, fat2_start);
        Cursor::new(image)
    }

    fn write_boot_sector(image: &mut [u8], total_sectors: u32) {
        image[0] = 0xEB;
        image[1] = 0x3C;
        image[2] = 0x90;
        image[3..11].copy_from_slice(b"HADRISFT");
        image[11..13].copy_from_slice(&(SECTOR_SIZE as u16).to_le_bytes());
        image[13] = SECTORS_PER_CLUSTER;
        image[14..16].copy_from_slice(&RESERVED_SECTORS.to_le_bytes());
        image[16] = FAT_COUNT;
        image[17..19].copy_from_slice(&ROOT_ENTRY_COUNT.to_le_bytes());
        if total_sectors <= 65535 {
            image[19..21].copy_from_slice(&(total_sectors as u16).to_le_bytes());
        } else {
            image[19..21].copy_from_slice(&0u16.to_le_bytes());
        }
        image[21] = 0xF8;
        image[22..24].copy_from_slice(&SECTORS_PER_FAT.to_le_bytes());
        image[24..26].copy_from_slice(&63u16.to_le_bytes());
        image[26..28].copy_from_slice(&255u16.to_le_bytes());
        image[28..32].copy_from_slice(&0u32.to_le_bytes());
        if total_sectors > 65535 {
            image[32..36].copy_from_slice(&total_sectors.to_le_bytes());
        } else {
            image[32..36].copy_from_slice(&0u32.to_le_bytes());
        }
        image[36] = 0x80;
        image[38] = 0x29;
        image[39..43].copy_from_slice(&0x12345678u32.to_le_bytes());
        image[43..54].copy_from_slice(b"TEST       ");
        image[54..62].copy_from_slice(b"FAT16   ");
        image[510] = 0x55;
        image[511] = 0xAA;
    }

    fn write_fat_table(image: &mut [u8], fat_start: usize) {
        image[fat_start..fat_start + 2].copy_from_slice(&0xFFF8u16.to_le_bytes());
        image[fat_start + 2..fat_start + 4].copy_from_slice(&0xFFFFu16.to_le_bytes());
    }

    pub const fn fat_start_bytes() -> usize {
        RESERVED_SECTORS as usize * SECTOR_SIZE
    }

    /// Count non-free FAT16 entries (index >= 2) in the primary FAT. A free
    /// entry is 0x0000. The two reserved entries (0, 1) are skipped.
    pub fn allocated_fat_entries(image: &[u8]) -> usize {
        let fat_start = fat_start_bytes();
        let entries = SECTORS_PER_FAT as usize * SECTOR_SIZE / 2;
        let mut allocated = 0;
        for i in 2..entries {
            let off = fat_start + i * 2;
            let raw = u16::from_le_bytes([image[off], image[off + 1]]);
            if raw != 0 {
                allocated += 1;
            }
        }
        allocated
    }
}
// ---------------------------------------------------------------------------
// B1: create_dir leaks its cluster on the DirectoryFull error path.
//
// Target: crates/block/hadris-fat/src/write.rs
//   create_dir allocates its data cluster and decrements the free count
//   (write.rs:1249-1253) BEFORE the fallible find_free_entry_run_in_dir call
//   (write.rs:1275). On a full fixed-root directory that call returns
//   DirectoryFull, leaving the just-allocated cluster marked EOC in the FAT
//   but referenced by nothing.
//
// Compare create_file, which allocates NO data cluster for the entry (empty
// file, cluster 0) and does its LFN work before touching the FAT — no
// asymmetric leak is possible there.
// ---------------------------------------------------------------------------
#[test]
fn b1_create_dir_leaks_cluster_on_directory_full() {
    let image = fat16_image::create_fat16_image();
    let fs = FatVolume::open(image).expect("open FAT16");

    // Fill the fixed root directory to capacity with single-entry 8.3 files.
    // Each empty file occupies exactly one 32-byte slot and allocates NO data
    // cluster, so the FAT stays empty (all data clusters free) before the
    // create_dir attempt.
    let root = fs.root_dir();
    for i in 0..fat16_image::ROOT_ENTRY_COUNT {
        let name = format!("{i:03}.TXT");
        fs.create_file(&root, &name)
            .expect("create_file should succeed while root has space");
    }

    // Now attempt to create a directory in the full root. This must fail with
    // DirectoryFull.
    let root = fs.root_dir();
    let result = fs.create_dir(&root, "SUB");
    match result {
        Err(Error::DirectoryFull) => {}
        Err(other) => panic!("expected DirectoryFull, got Err({other:?})"),
        Ok(_) => panic!("expected DirectoryFull, but create_dir succeeded"),
    }

    // CORRECT behavior: the failed create_dir must not have leaked a cluster.
    // Empty files consume no data clusters, so a correct implementation leaves
    // the FAT with zero allocated data clusters.
    let bytes = fs.into_inner().into_inner();
    let allocated_after = fat16_image::allocated_fat_entries(&bytes);
    assert_eq!(
        allocated_after, 0,
        "create_dir failing with DirectoryFull must not leak a cluster in the FAT \
         (found {allocated_after} allocated EOC entries referenced by nothing)"
    );
}

// ---------------------------------------------------------------------------
// B2: FileWriter::new_append overwrites the last cluster when the file size
// is an exact multiple of cluster_size.
//
// Target: crates/block/hadris-fat/src/write.rs:171
//   offset_in_last = file_size % cluster_size == 0 while current_cluster is
//   the last (full) cluster. write()'s guard `offset_in_cluster >=
//   cluster_size` is false at 0, so the first appended byte lands at offset 0
//   of the already-full last cluster, clobbering existing data.
// ---------------------------------------------------------------------------
#[test]
fn b2_new_append_overwrites_last_full_cluster() {
    let image = fat32_image::create_fat32_image();
    let fs = FatVolume::open(image).expect("open FAT32");
    let cluster_sz = fat32_image::cluster_size();

    // Create a file and write exactly one cluster of a known pattern.
    let root = fs.root_dir();
    let entry = fs.create_file(&root, "APPEND.BIN").expect("create_file");
    {
        let mut w = fs.write_file(&entry).expect("write_file");
        let pattern = vec![0x11u8; cluster_sz];
        w.write(&pattern).expect("write");
        w.finish().expect("finish");
    }

    // Re-find to pick up the committed size / first cluster.
    let root = fs.root_dir();
    let entry = root
        .find("APPEND.BIN")
        .expect("find")
        .expect("entry exists");
    assert_eq!(
        entry.len(),
        cluster_sz as u64,
        "precondition: one full cluster"
    );

    // Append 6 bytes via new_append.
    {
        let mut w = FileWriter::new_append(&fs, &entry).expect("new_append");
        w.write(b"APPEND").expect("append write");
        w.finish().expect("finish append");
    }

    // Read the file back.
    let root = fs.root_dir();
    let entry = root
        .find("APPEND.BIN")
        .expect("find")
        .expect("entry exists");
    let content = fs
        .read_file(&entry)
        .expect("read_file")
        .read_to_vec()
        .expect("read_to_vec");

    println!(
        "B2: read_back_len={} first8={:02x?} last8={:02x?}",
        content.len(),
        &content[..8.min(content.len())],
        &content[content.len().saturating_sub(8)..]
    );

    // CORRECT behavior: the original cluster is preserved and "APPEND" is
    // appended AFTER it.
    assert_eq!(
        content.len(),
        cluster_sz + 6,
        "appended file should be cluster_size + 6 bytes"
    );
    assert!(
        content[..cluster_sz].iter().all(|&b| b == 0x11),
        "the original cluster's bytes must be preserved (0x11), but they were \
         overwritten: first byte = {:#x}",
        content[0]
    );
    assert_eq!(
        &content[cluster_sz..],
        b"APPEND",
        "the appended bytes must follow the original cluster"
    );
}

// ---------------------------------------------------------------------------
// Stale-handle (TOCTOU) guards. A `FileEntry` snapshots parent cluster, slot
// offset, and short name. If the file is deleted and its slot reused before a
// mutating API acts on the snapshot, the API would operate on the unrelated
// entry now in the slot. Each mutating API (delete/rename/set_attributes/
// set_times/truncate) and `FileWriter::finish` now revalidates the on-disk
// short name (and not-deleted marker) before acting and returns
// `Error::StaleEntry` on mismatch, leaving the reusing entry untouched.
// ---------------------------------------------------------------------------

// A1: delete() through a stale handle must not free the live chain now owned by
// the renamed file.
#[test]
fn a1_stale_delete_rejected_and_preserves_renamed_file() {
    let fs = FatVolume::open(fat32_image::create_fat32_image()).expect("open");
    let cluster_sz = fat32_image::cluster_size();
    let content: Vec<u8> = (0..cluster_sz * 3).map(|i| (i % 251) as u8).collect();

    let root = fs.root_dir();
    let a = fs.create_file(&root, "A.TXT").expect("create A.TXT");
    {
        let mut w = fs.write_file(&a).expect("writer");
        w.write(&content).expect("write");
        w.finish().expect("finish");
    }

    // Snapshot a stale handle to A.TXT, then rename A.TXT -> B.TXT so B owns the
    // same live cluster chain.
    let root = fs.root_dir();
    let stale = root.find("A.TXT").expect("find").expect("A.TXT exists");
    let chain_start = stale.cluster();
    let dst = fs.root_dir();
    let b = fs.rename(&stale, &dst, "B.TXT").expect("rename");
    assert_eq!(b.cluster(), chain_start, "rename keeps the chain");

    match fs.delete(&stale) {
        Err(Error::StaleEntry) => {}
        other => panic!("stale delete must return StaleEntry, got {other:?}"),
    }

    let root = fs.root_dir();
    let b_now = root
        .find("B.TXT")
        .expect("find")
        .expect("B.TXT must still exist");
    let read_back = fs
        .read_file(&b_now)
        .expect("reader")
        .read_to_vec()
        .expect("read");
    assert_eq!(read_back, content, "B.TXT contents must be intact");
}

// A2: set_attributes() through a stale handle must not clear the DIRECTORY bit
// of the directory now occupying the reused slot.
#[test]
fn a2_stale_set_attributes_rejected_and_preserves_dir() {
    let fs = FatVolume::open(fat32_image::create_fat32_image()).expect("open");
    let root = fs.root_dir();
    fs.create_file(&root, "F.TXT").expect("create F.TXT");
    let root = fs.root_dir();
    let stale = root.find("F.TXT").expect("find").expect("F.TXT exists");
    assert!(stale.is_file());

    fs.delete(&stale).expect("delete F.TXT");
    fs.create_dir(&root, "SUB").expect("create_dir SUB");

    let attrs = DirEntryAttrFlags::ARCHIVE | DirEntryAttrFlags::HIDDEN;
    match fs.set_attributes(&stale, attrs) {
        Err(Error::StaleEntry) => {}
        other => panic!("stale set_attributes must return StaleEntry, got {other:?}"),
    }

    let root = fs.root_dir();
    let sub = root.find("SUB").expect("find").expect("SUB must exist");
    assert!(
        sub.is_directory(),
        "SUB must remain a directory (attrs {:?})",
        sub.attributes()
    );
}

// A3: FileWriter::finish() at stale coordinates must not clobber the entry now
// occupying the reused slot. The reusing entry here is a directory (created via
// create_dir, which opens no writer) so the slot's exclusive-writer guard does
// not pre-empt the scenario — this isolates the finish-time revalidation.
#[test]
fn a3_stale_finish_rejected_and_preserves_reused_slot() {
    let fs = FatVolume::open(fat32_image::create_fat32_image()).expect("open");
    let cluster_sz = fat32_image::cluster_size();
    let root = fs.root_dir();
    let old = fs.create_file(&root, "OLD.TXT").expect("create OLD.TXT");

    // Open a writer against OLD.TXT and write (but do not finish yet).
    let mut writer = fs.write_file(&old).expect("writer");
    let old_data: Vec<u8> = (0..cluster_sz * 2).map(|i| (i % 253) as u8).collect();
    writer.write(&old_data).expect("write old");

    // Delete OLD.TXT, then create a directory that reuses the freed slot.
    fs.delete(&old).expect("delete OLD.TXT");
    fs.create_dir(&root, "REUSED").expect("create_dir REUSED");

    let root = fs.root_dir();
    let reused_before = root.find("REUSED").expect("find").expect("REUSED exists");
    let reused_cluster = reused_before.cluster();

    match writer.finish() {
        Err(Error::StaleEntry) => {}
        other => panic!("stale finish must return StaleEntry, got {other:?}"),
    }

    // The directory that reused the slot must be untouched.
    let root = fs.root_dir();
    let reused_after = root
        .find("REUSED")
        .expect("find")
        .expect("REUSED must still exist");
    assert!(
        reused_after.is_directory(),
        "REUSED must remain a directory after the stale finish"
    );
    assert_eq!(
        reused_after.cluster(),
        reused_cluster,
        "REUSED first cluster must be intact"
    );
}

// A4: rename() through a stale handle must not move the entry now occupying the
// reused slot, nor create the destination name.
#[test]
fn a4_stale_rename_rejected_and_preserves_reused_slot() {
    let fs = FatVolume::open(fat32_image::create_fat32_image()).expect("open");
    let root = fs.root_dir();
    fs.create_file(&root, "A.TXT").expect("create A.TXT");
    let root = fs.root_dir();
    let stale = root.find("A.TXT").expect("find").expect("A.TXT exists");

    fs.delete(&stale).expect("delete A.TXT");
    fs.create_dir(&root, "SUB").expect("create_dir SUB");

    let dst = fs.root_dir();
    match fs.rename(&stale, &dst, "Z.TXT") {
        Err(Error::StaleEntry) => {}
        other => panic!("stale rename must return StaleEntry, got {other:?}"),
    }

    let root = fs.root_dir();
    assert!(
        root.find("Z.TXT").expect("find").is_none(),
        "stale rename must not create Z.TXT"
    );
    let sub = root.find("SUB").expect("find").expect("SUB must exist");
    assert!(sub.is_directory(), "SUB must remain a directory");
}

// A6: a FileReader held across a delete + reuse must not disclose the data of
// the file that reused the cluster. The reader revalidates its slot before
// serving bytes and returns StaleEntry instead of leaking PUBLIC's content.
#[test]
fn a6_stale_reader_rejected_not_disclosing_reused_cluster() {
    let fs = FatVolume::open(fat32_image::create_fat32_image()).expect("open");
    let cluster_sz = fat32_image::cluster_size();
    let secret_pattern = vec![0xABu8; cluster_sz];
    let public_pattern = vec![0xCDu8; cluster_sz];

    let root = fs.root_dir();
    let secret = fs.create_file(&root, "SECRET.TXT").expect("create SECRET");
    {
        let mut w = fs.write_file(&secret).expect("writer");
        w.write(&secret_pattern).expect("write secret");
        w.finish().expect("finish");
    }

    let root = fs.root_dir();
    let secret = root
        .find("SECRET.TXT")
        .expect("find")
        .expect("SECRET exists");
    let secret_cluster = secret.cluster();

    // Open a reader but do not read yet.
    let mut reader = fs.read_file(&secret).expect("reader");

    // Delete SECRET, then create PUBLIC which reuses the freed cluster.
    fs.delete(&secret).expect("delete SECRET");
    let public = fs.create_file(&root, "PUBLIC.TXT").expect("create PUBLIC");
    {
        let mut w = fs.write_file(&public).expect("writer");
        w.write(&public_pattern).expect("write public");
        w.finish().expect("finish");
    }
    let root = fs.root_dir();
    let public = root
        .find("PUBLIC.TXT")
        .expect("find")
        .expect("PUBLIC exists");
    assert_eq!(
        public.cluster(),
        secret_cluster,
        "precondition: PUBLIC reuses SECRET's cluster"
    );

    // The stale reader must refuse to serve PUBLIC's bytes.
    match reader.read_to_vec() {
        Err(Error::StaleEntry) => {}
        Ok(bytes) => panic!(
            "stale reader disclosed {} bytes (first = {:#x}) instead of erroring",
            bytes.len(),
            bytes.first().copied().unwrap_or(0)
        ),
        Err(other) => panic!("expected StaleEntry from stale reader, got {other:?}"),
    }
}

// A7: creating an entry through a stale FatDir (its directory was deleted and
// the slot reused) must be rejected, not write directory entries into the
// unrelated file that now occupies the directory's old slot/clusters.
#[test]
fn a7_stale_dir_create_rejected_and_preserves_reuser() {
    let fs = FatVolume::open(fat32_image::create_fat32_image()).expect("open");
    let root = fs.root_dir();

    // Create SUB and keep its directory handle.
    let sub_dir = fs.create_dir(&root, "SUB").expect("create_dir SUB");

    // Delete SUB, freeing its entry slot and its data cluster.
    let root = fs.root_dir();
    let sub_entry = root.find("SUB").expect("find").expect("SUB exists");
    fs.delete(&sub_entry).expect("delete SUB");

    // Create a file in root that reuses SUB's freed slot/cluster.
    let victim = fs.create_file(&root, "VICTIM.TXT").expect("create VICTIM");
    {
        let mut w = fs.write_file(&victim).expect("writer");
        w.write(b"victim-data").expect("write");
        w.finish().expect("finish");
    }

    // Both create paths through the stale directory handle must be rejected.
    match fs.create_file(&sub_dir, "X.TXT") {
        Err(Error::StaleEntry) => {}
        other => panic!("stale-dir create_file must return StaleEntry, got {other:?}"),
    }
    match fs.create_dir(&sub_dir, "Y") {
        Err(Error::StaleEntry) => {}
        Err(other) => panic!("stale-dir create_dir must return StaleEntry, got {other:?}"),
        Ok(_) => panic!("stale-dir create_dir unexpectedly succeeded"),
    }

    // VICTIM.TXT must be intact and root must not have gained X.TXT / Y.
    let root = fs.root_dir();
    assert!(
        root.find("X.TXT").expect("find").is_none(),
        "stale-dir create must not create X.TXT"
    );
    assert!(
        root.find("Y").expect("find").is_none(),
        "stale-dir create must not create Y"
    );
    let v = root
        .find("VICTIM.TXT")
        .expect("find")
        .expect("VICTIM exists");
    let data = fs
        .read_file(&v)
        .expect("reader")
        .read_to_vec()
        .expect("read");
    assert_eq!(data, b"victim-data", "VICTIM.TXT content must be intact");
}

// A5: a second FileWriter for a directory entry that already has one open must
// be rejected, so two writers cannot independently allocate and cross-link the
// file's chain (leaking the loser's clusters when the last finish wins).
#[test]
fn a5_second_writer_on_same_entry_rejected() {
    let fs = FatVolume::open(fat32_image::create_fat32_image()).expect("open");
    let cluster_sz = fat32_image::cluster_size();
    let root = fs.root_dir();
    let entry = fs.create_file(&root, "SHARED.BIN").expect("create");
    let free_before = fs.free_cluster_count().expect("fat32 tracks free count");

    let mut a = fs.write_file(&entry).expect("writer a");
    match fs.write_file(&entry) {
        Err(Error::WriterConflict) => {}
        Err(other) => panic!("second writer must return WriterConflict, got {other:?}"),
        Ok(_) => panic!("second writer on the same entry was unexpectedly allowed"),
    }

    // The first writer works normally.
    let data = vec![0xAAu8; cluster_sz * 2];
    a.write(&data).expect("write a");
    a.finish().expect("finish a");

    // Content is intact and exactly the two written clusters were consumed —
    // no orphaned/leaked chain from a phantom second writer.
    let entry = fs
        .root_dir()
        .find("SHARED.BIN")
        .expect("find")
        .expect("exists");
    let read_back = fs
        .read_file(&entry)
        .expect("reader")
        .read_to_vec()
        .expect("read");
    assert_eq!(read_back, data, "single-writer content must be intact");
    let free_after = fs.free_cluster_count().expect("fat32 tracks free count");
    assert_eq!(
        free_before - free_after,
        2,
        "exactly two clusters consumed; none leaked"
    );

    // Finishing the first writer released the slot, so a new writer opens.
    // This one overwrites from offset 0, so it runs after the assertions above.
    let b = fs.write_file(&entry).expect("writer after finish");
    b.finish().expect("finish b");
}

/// A8: a handle whose slot was freed and re-taken by a *same-named* file must
/// not act on the new file. The short name alone cannot tell the two apart, so
/// the creation timestamp is compared as well.
#[test]
fn a8_stale_handle_rejected_when_slot_reused_by_same_name() {
    let fs = FatVolume::open(fat32_image::create_fat32_image()).expect("open");
    let root = fs.root_dir();

    let victim = fs
        .create_file(&root, "REUSED.BIN")
        .expect("create original");
    {
        let mut w = fs.write_file(&victim).expect("writer");
        w.write(&[0x11u8; 64]).expect("write");
        w.finish().expect("finish");
    }
    let stale = fs
        .root_dir()
        .find("REUSED.BIN")
        .expect("find")
        .expect("exists");

    fs.delete(&stale).expect("delete original");

    // Recreate under the exact same name. Stamp a creation timestamp that
    // differs from the original's so the test does not depend on clock
    // granularity between the two creates.
    let fresh = fs.create_file(&root, "REUSED.BIN").expect("recreate");
    let payload = [0x22u8; 96];
    {
        let mut w = fs.write_file(&fresh).expect("writer");
        w.write(&payload).expect("write");
        w.finish().expect("finish");
    }
    let fresh = fs
        .root_dir()
        .find("REUSED.BIN")
        .expect("find")
        .expect("exists");
    let distinct = FatDateTime::new(2001, 2, 3, 4, 5, 6);
    fs.set_times(&fresh, None, None, Some(distinct))
        .expect("stamp creation time");

    match fs.delete(&stale) {
        Err(Error::StaleEntry) => {}
        other => panic!("stale same-name handle must be rejected, got {other:?}"),
    }

    let survivor = fs
        .root_dir()
        .find("REUSED.BIN")
        .expect("find")
        .expect("recreated file must survive");
    let content = fs
        .read_file(&survivor)
        .expect("reader")
        .read_to_vec()
        .expect("read");
    assert_eq!(content, payload, "recreated file must be untouched");
}

/// B3: `finish()` must surface a real I/O failure. Under `dirty-file-panic` the
/// Drop guard previously fired on the way out of a failing `finish()`, masking
/// the error with a panic about a writer that had in fact been finished.
#[test]
fn b3_finish_reports_io_error_instead_of_dirty_panic() {
    let fs = FatVolume::open(fail_after::device(fat32_image::create_fat32_image())).expect("open");
    let root = fs.root_dir();
    let entry = fs.create_file(&root, "IOFAIL.BIN").expect("create");

    let mut w = fs.write_file(&entry).expect("writer");
    w.write(&[0x33u8; 64]).expect("write");

    fail_after::arm();
    let result = w.finish();
    fail_after::disarm();

    match result {
        Err(Error::Io(_)) => {}
        other => panic!("finish() must report the I/O error, got {other:?}"),
    }
}

#[test]
fn b4_failed_cluster_link_releases_the_unlinked_allocation() {
    let fs = FatVolume::open(fail_link::device(fat32_image::create_fat32_image())).expect("open");
    let root = fs.root_dir();
    let entry = fs.create_file(&root, "LINKFAIL.BIN").expect("create");
    let free_before = fs.free_cluster_count().expect("fat32 free count");

    let mut writer = fs.write_file(&entry).expect("writer");
    fail_link::arm();
    let result = writer.write(&vec![0x5a; fat32_image::cluster_size() * 2]);

    match result {
        Err(Error::Io(_)) => {}
        other => panic!("second cluster link must report the I/O error, got {other:?}"),
    }

    writer.finish().expect("finish rolled-back file");
    assert_eq!(
        fs.free_cluster_count(),
        Some(free_before),
        "failed link leaked an unlinked cluster"
    );
}

#[test]
fn legacy_case_colliding_long_names_prefer_exact_match() {
    let fs = FatVolume::open(fat32_image::create_fat32_image()).expect("open");
    let root = fs.root_dir();
    fs.create_file(&root, "Legacy-One.txt").expect("first");
    fs.create_file(&root, "Legacy-Two.txt").expect("second");

    let first = root
        .find("Legacy-One.txt")
        .expect("find first")
        .expect("first exists");
    let second = root
        .find("Legacy-Two.txt")
        .expect("find second")
        .expect("second exists");
    let first_short = first.short_name().raw_bytes();
    let second_short = second.short_name().raw_bytes();
    let second_lfn_offset = fat32_image::data_start_bytes() + second.offset_within_cluster - 64;

    let mut image = fs.into_inner().into_inner();
    legacy_lfn::replace_two_entry_name(
        &mut image[second_lfn_offset..second_lfn_offset + 64],
        "LEGACY-ONE.TXT",
    );

    let fs = FatVolume::open(std::io::Cursor::new(image)).expect("reopen");
    let root = fs.root_dir();
    assert_eq!(
        root.find("Legacy-One.txt")
            .expect("find mixed case")
            .expect("mixed case exists")
            .short_name()
            .raw_bytes(),
        first_short
    );
    assert_eq!(
        root.find("LEGACY-ONE.TXT")
            .expect("find uppercase")
            .expect("uppercase exists")
            .short_name()
            .raw_bytes(),
        second_short
    );
}

mod legacy_lfn {
    const UNITS_PER_ENTRY: usize = 13;
    const NAME_OFFSETS: [usize; UNITS_PER_ENTRY] = [1, 3, 5, 7, 9, 14, 16, 18, 20, 22, 24, 28, 30];

    pub fn replace_two_entry_name(entries: &mut [u8], name: &str) {
        let mut units = [0xffff; UNITS_PER_ENTRY * 2];
        let mut len = 0;
        for unit in name.encode_utf16() {
            units[len] = unit;
            len += 1;
        }
        units[len] = 0;
        write_entry(&mut entries[..32], &units[UNITS_PER_ENTRY..]);
        write_entry(&mut entries[32..], &units[..UNITS_PER_ENTRY]);
    }

    fn write_entry(entry: &mut [u8], units: &[u16]) {
        for (offset, unit) in NAME_OFFSETS.iter().zip(units) {
            entry[*offset..*offset + 2].copy_from_slice(&unit.to_le_bytes());
        }
    }
}

mod fail_link {
    use std::cell::Cell;
    use std::io::{Cursor, Read, Result, Seek, SeekFrom, Write};

    const FIRST_FILE_CLUSTER: u64 = 3;
    const SECOND_FILE_CLUSTER: u32 = 4;

    thread_local! {
        static ARMED: Cell<bool> = const { Cell::new(false) };
    }

    pub fn arm() {
        ARMED.with(|armed| armed.set(true));
    }

    pub struct FailSecondClusterLink(Cursor<Vec<u8>>);

    pub fn device(inner: Cursor<Vec<u8>>) -> FailSecondClusterLink {
        FailSecondClusterLink(inner)
    }

    impl Write for FailSecondClusterLink {
        fn write(&mut self, buf: &[u8]) -> Result<usize> {
            let value = buf
                .get(..4)
                .and_then(|bytes| bytes.try_into().ok())
                .map(u32::from_le_bytes);
            let link_offset = super::fat32_image::fat_start_bytes() as u64 + FIRST_FILE_CLUSTER * 4;
            if self.0.position() == link_offset
                && value == Some(SECOND_FILE_CLUSTER)
                && ARMED.with(|armed| armed.replace(false))
            {
                return Err(std::io::Error::other("injected cluster-link failure"));
            }
            self.0.write(buf)
        }

        fn flush(&mut self) -> Result<()> {
            self.0.flush()
        }
    }

    impl Read for FailSecondClusterLink {
        fn read(&mut self, buf: &mut [u8]) -> Result<usize> {
            self.0.read(buf)
        }
    }

    impl Seek for FailSecondClusterLink {
        fn seek(&mut self, pos: SeekFrom) -> Result<u64> {
            self.0.seek(pos)
        }
    }
}

mod fail_after {
    use std::cell::Cell;
    use std::io::{Cursor, Read, Result, Seek, SeekFrom, Write};

    thread_local! {
        static ARMED: Cell<bool> = const { Cell::new(false) };
    }

    pub fn arm() {
        ARMED.with(|a| a.set(true));
    }

    pub fn disarm() {
        ARMED.with(|a| a.set(false));
    }

    /// Wraps an image so that every write fails once armed. Reads and seeks
    /// keep working, so the failure lands inside `finish()` rather than during
    /// mount or lookup.
    pub struct FailingWrites(Cursor<Vec<u8>>);

    pub fn device(inner: Cursor<Vec<u8>>) -> FailingWrites {
        FailingWrites(inner)
    }

    impl Write for FailingWrites {
        fn write(&mut self, buf: &[u8]) -> Result<usize> {
            if ARMED.with(|a| a.get()) {
                return Err(std::io::Error::other("injected write failure"));
            }
            self.0.write(buf)
        }

        fn flush(&mut self) -> Result<()> {
            if ARMED.with(|a| a.get()) {
                return Err(std::io::Error::other("injected flush failure"));
            }
            self.0.flush()
        }
    }

    impl Read for FailingWrites {
        fn read(&mut self, buf: &mut [u8]) -> Result<usize> {
            self.0.read(buf)
        }
    }

    impl Seek for FailingWrites {
        fn seek(&mut self, pos: SeekFrom) -> Result<u64> {
            self.0.seek(pos)
        }
    }
}
