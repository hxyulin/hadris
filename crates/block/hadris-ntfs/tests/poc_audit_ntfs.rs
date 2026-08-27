//! Audit POCs for hadris-ntfs: crafted images probing dir walking, index
//! allocation parsing, data-run arithmetic, and the public attr helpers.
//! Every probe asserts graceful errors / bounded behavior; none should panic.

use std::io::Cursor;

use hadris_ntfs::NtfsError;
use hadris_ntfs::attr::{self, parse_index_entries};
use hadris_ntfs::sync::{NtfsFs, NtfsFsReadExt};

const SECTOR: usize = 512;
const REC: usize = 1024;
const MFT_OFF: usize = 2048; // LCN 4
const MFT_RECORDS: usize = 16;
const IMAGE_LEN: usize = 32768;

const I30: &[u8] = &[0x24, 0x00, 0x49, 0x00, 0x33, 0x00, 0x30, 0x00];

fn boot_sector() -> Vec<u8> {
    let mut boot = vec![0_u8; SECTOR];
    boot[3..11].copy_from_slice(b"NTFS    ");
    boot[11..13].copy_from_slice(&512_u16.to_le_bytes());
    boot[13] = 1; // sectors per cluster
    boot[40..48].copy_from_slice(&512_u64.to_le_bytes()); // total sectors (256 KiB)
    boot[48..56].copy_from_slice(&4_u64.to_le_bytes()); // $MFT LCN
    boot[64] = (-10_i8) as u8; // 1024-byte MFT records
    boot[68] = (-10_i8) as u8; // 1024-byte index records
    boot[510..512].copy_from_slice(&0xAA55_u16.to_le_bytes());
    boot
}

fn resident_attr(attr_type: u32, name: Option<&[u8]>, value: &[u8]) -> Vec<u8> {
    let name_len_units = name.map_or(0, |n| n.len() / 2);
    let name_bytes = name.map_or(0, |n| n.len());
    let value_off = 0x18 + name_bytes;
    let total = (value_off + value.len() + 7) & !7;
    let mut a = vec![0_u8; total];
    a[0..4].copy_from_slice(&attr_type.to_le_bytes());
    a[4..8].copy_from_slice(&(total as u32).to_le_bytes());
    a[9] = name_len_units as u8;
    if let Some(n) = name {
        a[0x0A..0x0C].copy_from_slice(&0x18_u16.to_le_bytes());
        a[0x18..0x18 + n.len()].copy_from_slice(n);
    }
    a[0x10..0x14].copy_from_slice(&(value.len() as u32).to_le_bytes());
    a[0x14..0x16].copy_from_slice(&(value_off as u16).to_le_bytes());
    a[value_off..value_off + value.len()].copy_from_slice(value);
    a
}

fn nonresident_attr(
    attr_type: u32,
    name: Option<&[u8]>,
    last_vcn: u64,
    data_size: u64,
    initialized: u64,
    runs: &[u8],
) -> Vec<u8> {
    let name_bytes = name.map_or(0, |n| n.len());
    let runs_off = 0x40 + name_bytes;
    let total = (runs_off + runs.len() + 7) & !7;
    let mut a = vec![0_u8; total];
    a[0..4].copy_from_slice(&attr_type.to_le_bytes());
    a[4..8].copy_from_slice(&(total as u32).to_le_bytes());
    a[8] = 1;
    if let Some(n) = name {
        a[9] = (n.len() / 2) as u8;
        a[0x0A..0x0C].copy_from_slice(&0x40_u16.to_le_bytes());
        a[0x40..0x40 + n.len()].copy_from_slice(n);
    }
    a[0x18..0x20].copy_from_slice(&last_vcn.to_le_bytes());
    a[0x20..0x22].copy_from_slice(&(runs_off as u16).to_le_bytes());
    a[0x28..0x30].copy_from_slice(&data_size.to_le_bytes());
    a[0x30..0x38].copy_from_slice(&data_size.to_le_bytes());
    a[0x38..0x40].copy_from_slice(&initialized.to_le_bytes());
    a[runs_off..runs_off + runs.len()].copy_from_slice(runs);
    a
}

fn utf16le(s: &str) -> Vec<u8> {
    s.encode_utf16().flat_map(|u| u.to_le_bytes()).collect()
}

fn file_name_value(name: &str, is_dir: bool, data_size: u64) -> Vec<u8> {
    let name_bytes = utf16le(name);
    let mut v = vec![0_u8; 0x42 + name_bytes.len()];
    v[0..8].copy_from_slice(&5_u64.to_le_bytes()); // parent: root dir
    v[0x28..0x30].copy_from_slice(&data_size.to_le_bytes());
    v[0x30..0x38].copy_from_slice(&data_size.to_le_bytes());
    let flags: u32 = if is_dir { 0x1000_0020 } else { 0x20 };
    v[0x38..0x3C].copy_from_slice(&flags.to_le_bytes());
    v[0x40] = (name_bytes.len() / 2) as u8;
    v[0x41] = attr::FILE_NAME_POSIX;
    v[0x42..].copy_from_slice(&name_bytes);
    v
}

fn index_entry(mft_ref: u64, name: &str, is_dir: bool, data_size: u64) -> Vec<u8> {
    let content = file_name_value(name, is_dir, data_size);
    let entry_len = 16 + content.len();
    let mut e = vec![0_u8; entry_len];
    e[0..8].copy_from_slice(&mft_ref.to_le_bytes());
    e[8..10].copy_from_slice(&(entry_len as u16).to_le_bytes());
    e[10..12].copy_from_slice(&(content.len() as u16).to_le_bytes());
    e[16..].copy_from_slice(&content);
    e
}

fn last_index_entry() -> Vec<u8> {
    let mut e = vec![0_u8; 16];
    e[8..10].copy_from_slice(&16_u16.to_le_bytes());
    e[12..16].copy_from_slice(&attr::INDEX_ENTRY_LAST.to_le_bytes());
    e
}

fn index_root_value(entries: &[u8], irs: u32) -> Vec<u8> {
    let mut v = vec![0_u8; 0x20 + entries.len()];
    v[0..4].copy_from_slice(&attr::ATTR_FILE_NAME.to_le_bytes());
    v[4..8].copy_from_slice(&1_u32.to_le_bytes()); // collation: unicode
    v[8..12].copy_from_slice(&irs.to_le_bytes());
    v[12] = 2; // clusters per index record
    v[0x10..0x14].copy_from_slice(&16_u32.to_le_bytes()); // entries offset
    let total = 16 + entries.len() as u32;
    v[0x14..0x18].copy_from_slice(&total.to_le_bytes());
    v[0x18..0x1C].copy_from_slice(&total.to_le_bytes());
    v[0x20..].copy_from_slice(entries);
    v
}

fn seal_record(buf: &mut [u8], uso: usize) {
    assert_eq!(buf.len() % SECTOR, 0);
    let usn = 0xAAAA_u16.to_le_bytes();
    buf[4..6].copy_from_slice(&(uso as u16).to_le_bytes());
    let uss = (buf.len() / SECTOR + 1) as u16;
    buf[6..8].copy_from_slice(&uss.to_le_bytes());
    buf[uso..uso + 2].copy_from_slice(&usn);
    for i in 0..buf.len() / SECTOR {
        let end = (i + 1) * SECTOR - 2;
        buf[uso + 2 + i * 2] = buf[end];
        buf[uso + 3 + i * 2] = buf[end + 1];
        buf[end..end + 2].copy_from_slice(&usn);
    }
}

fn file_record(flags: u16, attrs: &[Vec<u8>]) -> Vec<u8> {
    let mut r = vec![0_u8; REC];
    r[0..4].copy_from_slice(b"FILE");
    r[0x10..0x12].copy_from_slice(&1_u16.to_le_bytes()); // sequence
    r[0x14..0x16].copy_from_slice(&0x38_u16.to_le_bytes()); // first attr
    r[0x16..0x18].copy_from_slice(&flags.to_le_bytes());
    let mut off = 0x38;
    for a in attrs {
        r[off..off + a.len()].copy_from_slice(a);
        off += a.len();
    }
    r[off..off + 4].copy_from_slice(&attr::ATTR_END.to_le_bytes());
    let used = (off + 8) & !7;
    r[0x18..0x1C].copy_from_slice(&(used as u32).to_le_bytes());
    seal_record(&mut r, 0x30);
    r
}

fn put_record(image: &mut [u8], index: usize, record: &[u8]) {
    let off = MFT_OFF + index * REC;
    image[off..off + REC].copy_from_slice(record);
}

/// Base mountable image: $MFT covering records 0..15, sparse zero $UpCase,
/// root dir (5) with file HELLO.TXT (6) and subdir SUBDIR (7), a non-resident
/// file BIN.DAT (8) reading 3 bytes from LCN 40.
fn base_image() -> Vec<u8> {
    let mut image = boot_sector();
    image.resize(IMAGE_LEN, 0);

    let mft_data = nonresident_attr(
        attr::ATTR_DATA,
        None,
        31,
        (MFT_RECORDS * REC) as u64,
        (MFT_RECORDS * REC) as u64,
        &[0x11, 0x20, 0x04, 0x00],
    );
    put_record(&mut image, 0, &file_record(1, &[mft_data]));

    let upcase = nonresident_attr(
        attr::ATTR_DATA,
        None,
        255,
        131072,
        0,
        &[0x02, 0x00, 0x01, 0x00], // 256 sparse clusters
    );
    put_record(&mut image, 10, &file_record(1, &[upcase]));

    let mut root_entries = index_entry(6, "HELLO.TXT", false, 10);
    root_entries.extend(index_entry(7, "SUBDIR", true, 0));
    root_entries.extend(index_entry(8, "BIN.DAT", false, 3));
    root_entries.extend(last_index_entry());
    let index_root = resident_attr(
        attr::ATTR_INDEX_ROOT,
        Some(I30),
        &index_root_value(&root_entries, 1024),
    );
    put_record(&mut image, 5, &file_record(3, &[index_root]));

    let hello = resident_attr(attr::ATTR_DATA, None, b"hello ntfs");
    put_record(&mut image, 6, &file_record(1, &[hello]));

    let empty_root = resident_attr(
        attr::ATTR_INDEX_ROOT,
        Some(I30),
        &index_root_value(&last_index_entry(), 1024),
    );
    put_record(&mut image, 7, &file_record(3, &[empty_root]));

    let bin = nonresident_attr(attr::ATTR_DATA, None, 0, 3, 3, &[0x11, 0x01, 0x28, 0x00]);
    put_record(&mut image, 8, &file_record(1, &[bin]));
    image[40 * SECTOR..40 * SECTOR + 3].copy_from_slice(b"bin");

    image
}

/// Variant where the root directory spills into $INDEX_ALLOCATION: one INDX
/// block at LCN 44 holding INDXFILE.TXT (record 9), bitmap marking block 0.
fn index_alloc_image(
    irs: u32,
    bitmap: &[u8],
    alloc_data_size: u64,
    indx: Option<Vec<u8>>,
) -> Vec<u8> {
    let mut image = base_image();

    let index_root = resident_attr(
        attr::ATTR_INDEX_ROOT,
        Some(I30),
        &index_root_value(&last_index_entry(), irs),
    );
    let index_alloc = nonresident_attr(
        attr::ATTR_INDEX_ALLOCATION,
        Some(I30),
        alloc_data_size / 512,
        alloc_data_size,
        alloc_data_size,
        &[0x11, 0x02, 0x2C, 0x00], // 2 clusters at LCN 44
    );
    let bitmap_attr = resident_attr(attr::ATTR_BITMAP, Some(I30), bitmap);
    put_record(
        &mut image,
        5,
        &file_record(3, &[index_root, index_alloc, bitmap_attr]),
    );

    let indx = indx.unwrap_or_else(|| {
        let mut entries = index_entry(9, "INDXFILE.TXT", false, 9);
        entries.extend(last_index_entry());
        let mut b = vec![0_u8; REC];
        b[0..4].copy_from_slice(b"INDX");
        b[0x18..0x1C].copy_from_slice(&0x18_u32.to_le_bytes()); // entries at 0x30
        let total = 0x18 + entries.len() as u32;
        b[0x1C..0x20].copy_from_slice(&total.to_le_bytes());
        b[0x20..0x24].copy_from_slice(&total.to_le_bytes());
        b[0x30..0x30 + entries.len()].copy_from_slice(&entries);
        seal_record(&mut b, 0x28);
        b
    });
    image[44 * SECTOR..44 * SECTOR + REC].copy_from_slice(&indx);

    let data = resident_attr(attr::ATTR_DATA, None, b"indx data");
    put_record(&mut image, 9, &file_record(1, &[data]));

    image
}

#[test]
fn poc_crafted_image_mounts_and_walks() {
    let image = base_image();
    let fs = NtfsFs::open(Cursor::new(&image)).expect("crafted image must mount");

    let root = fs.root_dir();
    let entries = root.entries().unwrap();
    let names: Vec<&str> = entries.iter().map(|e| e.name()).collect();
    assert_eq!(names, ["HELLO.TXT", "SUBDIR", "BIN.DAT"]);

    let entry = root.find("HELLO.TXT").unwrap().expect("file must exist");
    let mut reader = fs.read_file(&entry).unwrap();
    assert_eq!(reader.read_to_vec().unwrap(), b"hello ntfs");

    let entry = fs.open_path("/BIN.DAT").unwrap();
    let mut reader = fs.read_file(&entry).unwrap();
    assert_eq!(reader.read_to_vec().unwrap(), b"bin");

    let sub = root.open_dir("SUBDIR").unwrap();
    assert!(sub.entries().unwrap().is_empty());

    let entry = fs.open_path("/HELLO.TXT").unwrap();
    assert!(!entry.is_directory());
}

#[test]
fn poc_index_allocation_blocks_walk() {
    let image = index_alloc_image(1024, &[0x01], 1024, None);
    let fs = NtfsFs::open(Cursor::new(&image)).expect("crafted image must mount");
    let entries = fs.root_dir().entries().unwrap();
    let names: Vec<&str> = entries.iter().map(|e| e.name()).collect();
    assert_eq!(names, ["INDXFILE.TXT"]);

    let entry = fs.root_dir().find("INDXFILE.TXT").unwrap().unwrap();
    let mut reader = fs.read_file(&entry).unwrap();
    assert_eq!(reader.read_to_vec().unwrap(), b"indx data");
}

#[test]
fn poc_huge_mft_index_is_rejected_without_overflow() {
    let image = base_image();
    let fs = NtfsFs::open(Cursor::new(&image)).unwrap();
    assert!(matches!(
        fs.read_mft_record(u64::MAX),
        Err(NtfsError::MftRecordOutOfBounds { .. })
    ));
    assert!(matches!(
        fs.read_mft_record(1 << 40),
        Err(NtfsError::MftRecordOutOfBounds { .. }) | Err(NtfsError::UnexpectedEndOfData)
    ));
}

#[test]
fn poc_huge_data_run_length_is_rejected_without_overflow() {
    // A single run claiming u64::MAX clusters: length * cluster_size must not
    // wrap when reading file data.
    let mut image = base_image();
    let mut runs = vec![0x18_u8]; // 8-byte length, 1-byte offset
    runs.extend(u64::MAX.to_le_bytes());
    runs.push(0x01);
    runs.push(0x00);
    let bin = nonresident_attr(attr::ATTR_DATA, None, u64::MAX, 4096, 4096, &runs);
    put_record(&mut image, 8, &file_record(1, &[bin]));

    let fs = NtfsFs::open(Cursor::new(&image)).unwrap();
    let entry = fs.open_path("BIN.DAT").unwrap();
    let mut reader = fs.read_file(&entry).unwrap();
    let mut buf = [0u8; 512];
    assert!(matches!(
        reader.read(&mut buf),
        Err(NtfsError::InvalidDataRun)
    ));
}

#[test]
fn poc_corrupt_index_allocation_variants_error_out() {
    // irs overridden to 4 bytes: block too small for fixups.
    let image = index_alloc_image(4, &[0x01], 4, None);
    let fs = NtfsFs::open(Cursor::new(&image)).unwrap();
    assert!(fs.root_dir().entries().is_err());

    // irs overridden to u32::MAX: must be rejected against the image length,
    // not used as an allocation size.
    let image = index_alloc_image(u32::MAX, &[0x01], 1024, None);
    let fs = NtfsFs::open(Cursor::new(&image)).unwrap();
    assert!(matches!(
        fs.root_dir().entries(),
        Err(NtfsError::InvalidAttribute) | Err(NtfsError::InvalidIndexEntry)
    ));

    // Bitmap claims block 1 but the allocation stream covers only block 0's
    // data... runs cover 1024 bytes; reading block 1 must fail cleanly.
    let image = index_alloc_image(1024, &[0x03], 2048, None);
    let fs = NtfsFs::open(Cursor::new(&image)).unwrap();
    assert!(fs.root_dir().entries().is_err());

    // INDX block with a corrupted sector trailer: fixup mismatch.
    let bad = {
        let mut entries = index_entry(9, "INDXFILE.TXT", false, 9);
        entries.extend(last_index_entry());
        let mut b = vec![0_u8; REC];
        b[0..4].copy_from_slice(b"INDX");
        b[0x18..0x1C].copy_from_slice(&0x18_u32.to_le_bytes());
        let total = 0x18 + entries.len() as u32;
        b[0x1C..0x20].copy_from_slice(&total.to_le_bytes());
        b[0x20..0x24].copy_from_slice(&total.to_le_bytes());
        b[0x30..0x30 + entries.len()].copy_from_slice(&entries);
        seal_record(&mut b, 0x28);
        b[510] ^= 0xFF;
        b
    };
    let image = index_alloc_image(1024, &[0x01], 1024, Some(bad));
    let fs = NtfsFs::open(Cursor::new(&image)).unwrap();
    assert!(matches!(
        fs.root_dir().entries(),
        Err(NtfsError::FixupMismatch { .. })
    ));
}

#[test]
fn poc_uninitialized_stream_beyond_volume_is_rejected() {
    // data_size = u64::MAX, initialized = 0: reads yield zeros by spec, but
    // read_to_vec must reject the bogus size against the claimed volume.
    let mut image = base_image();
    let bin = nonresident_attr(
        attr::ATTR_DATA,
        None,
        u64::MAX,
        u64::MAX,
        0,
        &[0x02, 0x00, 0x01, 0x00],
    );
    put_record(&mut image, 8, &file_record(1, &[bin]));

    let fs = NtfsFs::open(Cursor::new(&image)).unwrap();
    let entry = fs.open_path("BIN.DAT").unwrap();
    let mut reader = fs.read_file(&entry).unwrap();
    assert!(matches!(
        reader.read_to_vec(),
        Err(NtfsError::InvalidAttribute)
    ));
    let mut buf = [0u8; 16];
    assert_eq!(reader.read(&mut buf).unwrap(), 16);
    assert_eq!(buf, [0u8; 16]);
}

#[test]
fn parse_index_entries_rejects_a_huge_node_offset() {
    // Regression: node_header_offset + 16 used to overflow usize in debug
    // (and wrap into an out-of-bounds index in release), panicking on a
    // hostile caller-supplied offset instead of erroring.
    let data = vec![0u8; 64];
    assert!(matches!(
        parse_index_entries(&data, usize::MAX),
        Err(NtfsError::InvalidIndexEntry)
    ));
    assert!(matches!(
        parse_index_entries(&data, usize::MAX - 8),
        Err(NtfsError::InvalidIndexEntry)
    ));
}

#[cfg(feature = "async")]
fn block_on<F: core::future::Future>(fut: F) -> F::Output {
    use core::task::{Context, Poll, RawWaker, RawWakerVTable, Waker};
    fn clone(_: *const ()) -> RawWaker {
        raw_waker()
    }
    fn no_op(_: *const ()) {}
    fn raw_waker() -> RawWaker {
        RawWaker::new(
            core::ptr::null(),
            &RawWakerVTable::new(clone, no_op, no_op, no_op),
        )
    }
    let waker = unsafe { Waker::from_raw(raw_waker()) };
    let mut cx = Context::from_waker(&waker);
    let mut fut = core::pin::pin!(fut);
    loop {
        match fut.as_mut().poll(&mut cx) {
            Poll::Ready(v) => return v,
            Poll::Pending => std::thread::yield_now(),
        }
    }
}

#[cfg(feature = "async")]
#[test]
fn poc_async_twin_mounts_walks_and_rejects_corruption() {
    use hadris_ntfs::r#async::fs::NtfsFs as AsyncNtfsFs;
    use hadris_ntfs::r#async::read::NtfsFsReadExt as _;

    let image = base_image();
    block_on(async {
        let fs = AsyncNtfsFs::open(hadris_io::Cursor::new(&image[..]))
            .await
            .expect("async open must succeed");
        let entries = fs.root_dir().entries().await.unwrap();
        let names: Vec<&str> = entries.iter().map(|e| e.name()).collect();
        assert_eq!(names, ["HELLO.TXT", "SUBDIR", "BIN.DAT"]);

        let entry = fs.root_dir().find("HELLO.TXT").await.unwrap().unwrap();
        let mut reader = fs.read_file(&entry).await.unwrap();
        assert_eq!(reader.read_to_vec().await.unwrap(), b"hello ntfs");

        assert!(matches!(
            fs.read_mft_record(u64::MAX).await,
            Err(NtfsError::MftRecordOutOfBounds { .. })
        ));
    });

    let image = index_alloc_image(u32::MAX, &[0x01], 1024, None);
    block_on(async {
        let fs = AsyncNtfsFs::open(hadris_io::Cursor::new(&image[..]))
            .await
            .unwrap();
        assert!(matches!(
            fs.root_dir().entries().await,
            Err(NtfsError::InvalidAttribute) | Err(NtfsError::InvalidIndexEntry)
        ));
    });
}
