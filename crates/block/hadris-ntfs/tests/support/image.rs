//! Crafted NTFS images for tests: a small volume with files, a
//! subdirectory, hidden metadata entries and index blocks, built byte by
//! byte so each test can break one structure.

#![allow(dead_code)]

use hadris_ntfs::raw;

pub const SECTOR: usize = 512;
pub const REC: usize = 1024;
pub const MFT_LCN: usize = 4;
pub const MFT_RECORDS: usize = 32;
pub const IMAGE_LEN: usize = 256 * 1024;
pub const UPCASE_LCN: usize = 100;
pub const BIN_LCN: usize = 120;
pub const INDX_LCN: usize = 130;
pub const STREAM_LCN: usize = 140;
pub const SEQ: u16 = 1;

pub fn reference(record: u64) -> u64 {
    record | u64::from(SEQ) << 48
}

pub fn boot_sector() -> Vec<u8> {
    let mut boot = vec![0u8; SECTOR];
    boot[3..11].copy_from_slice(&raw::OEM_ID);
    boot[11..13].copy_from_slice(&512u16.to_le_bytes());
    boot[13] = 1;
    boot[40..48].copy_from_slice(&((IMAGE_LEN / SECTOR) as u64).to_le_bytes());
    boot[48..56].copy_from_slice(&(MFT_LCN as u64).to_le_bytes());
    boot[64] = (-10i8) as u8;
    boot[68] = (-10i8) as u8;
    boot[72..80].copy_from_slice(&0x1122_3344_5566_7788u64.to_le_bytes());
    boot[510..512].copy_from_slice(&raw::BOOT_SIGNATURE.to_le_bytes());
    boot
}

pub fn utf16(s: &str) -> Vec<u8> {
    s.encode_utf16().flat_map(u16::to_le_bytes).collect()
}

pub fn resident(kind: u32, name: &[u8], value: &[u8]) -> Vec<u8> {
    let value_at = (0x18 + name.len() + 7) & !7;
    let total = (value_at + value.len() + 7) & !7;
    let mut a = vec![0u8; total];
    a[0..4].copy_from_slice(&kind.to_le_bytes());
    a[4..8].copy_from_slice(&(total as u32).to_le_bytes());
    a[9] = (name.len() / 2) as u8;
    a[0x0A..0x0C].copy_from_slice(&0x18u16.to_le_bytes());
    a[0x18..0x18 + name.len()].copy_from_slice(name);
    a[0x10..0x14].copy_from_slice(&(value.len() as u32).to_le_bytes());
    a[0x14..0x16].copy_from_slice(&(value_at as u16).to_le_bytes());
    a[value_at..value_at + value.len()].copy_from_slice(value);
    a
}

pub fn non_resident(
    kind: u32,
    name: &[u8],
    last_vcn: u64,
    size: u64,
    initialized: u64,
    runs: &[u8],
) -> Vec<u8> {
    let runs_at = (0x40 + name.len() + 7) & !7;
    let total = (runs_at + runs.len() + 7) & !7;
    let mut a = vec![0u8; total];
    a[0..4].copy_from_slice(&kind.to_le_bytes());
    a[4..8].copy_from_slice(&(total as u32).to_le_bytes());
    a[8] = 1;
    a[9] = (name.len() / 2) as u8;
    a[0x0A..0x0C].copy_from_slice(&0x40u16.to_le_bytes());
    a[0x40..0x40 + name.len()].copy_from_slice(name);
    a[0x18..0x20].copy_from_slice(&last_vcn.to_le_bytes());
    a[0x20..0x22].copy_from_slice(&(runs_at as u16).to_le_bytes());
    a[0x28..0x30].copy_from_slice(&size.to_le_bytes());
    a[0x30..0x38].copy_from_slice(&size.to_le_bytes());
    a[0x38..0x40].copy_from_slice(&initialized.to_le_bytes());
    a[runs_at..runs_at + runs.len()].copy_from_slice(runs);
    a
}

pub fn file_name_value(parent: u64, name: &str, dir: bool, namespace: u8) -> Vec<u8> {
    let units = utf16(name);
    let mut v = vec![0u8; 0x42 + units.len()];
    v[0..8].copy_from_slice(&parent.to_le_bytes());
    let flags: u32 = if dir {
        raw::FILE_NAME_INDEX_PRESENT
    } else {
        raw::FILE_ATTRIBUTE_ARCHIVE
    };
    v[0x38..0x3C].copy_from_slice(&flags.to_le_bytes());
    v[0x40] = (units.len() / 2) as u8;
    v[0x41] = namespace;
    v[0x42..].copy_from_slice(&units);
    v
}

/// 2020-01-01T00:00:00Z in NTFS time.
pub const TIME: u64 = 132_223_104_000_000_000;

pub fn standard_information(attributes: u32) -> Vec<u8> {
    let mut v = vec![0u8; 0x48];
    for i in 0..4 {
        v[i * 8..i * 8 + 8].copy_from_slice(&(TIME + i as u64 * 10_000_000).to_le_bytes());
    }
    v[0x20..0x24].copy_from_slice(&attributes.to_le_bytes());
    resident(raw::ATTR_STANDARD_INFORMATION, &[], &v)
}

pub fn index_entry(record: u64, name: &str, dir: bool, namespace: u8) -> Vec<u8> {
    let content = file_name_value(reference(raw::RECORD_ROOT), name, dir, namespace);
    let len = (16 + content.len() + 7) & !7;
    let mut e = vec![0u8; len];
    e[0..8].copy_from_slice(&reference(record).to_le_bytes());
    e[8..10].copy_from_slice(&(len as u16).to_le_bytes());
    e[10..12].copy_from_slice(&(content.len() as u16).to_le_bytes());
    e[16..16 + content.len()].copy_from_slice(&content);
    e
}

pub fn last_entry() -> Vec<u8> {
    let mut e = vec![0u8; 16];
    e[8..10].copy_from_slice(&16u16.to_le_bytes());
    e[12..16].copy_from_slice(&raw::INDEX_ENTRY_LAST.to_le_bytes());
    e
}

pub fn index_root(entries: &[u8], block_size: u32) -> Vec<u8> {
    let mut v = vec![0u8; 0x20 + entries.len()];
    v[0..4].copy_from_slice(&raw::ATTR_FILE_NAME.to_le_bytes());
    v[4..8].copy_from_slice(&1u32.to_le_bytes());
    v[8..12].copy_from_slice(&block_size.to_le_bytes());
    v[12] = 2;
    let total = 16 + entries.len() as u32;
    v[0x10..0x14].copy_from_slice(&16u32.to_le_bytes());
    v[0x14..0x18].copy_from_slice(&total.to_le_bytes());
    v[0x18..0x1C].copy_from_slice(&total.to_le_bytes());
    v[0x20..].copy_from_slice(entries);
    resident(raw::ATTR_INDEX_ROOT, &raw::I30, &v)
}

pub fn seal(buf: &mut [u8], usa: usize) {
    let usn = 0xAAAAu16.to_le_bytes();
    buf[4..6].copy_from_slice(&(usa as u16).to_le_bytes());
    let count = (buf.len() / SECTOR + 1) as u16;
    buf[6..8].copy_from_slice(&count.to_le_bytes());
    buf[usa..usa + 2].copy_from_slice(&usn);
    for i in 0..buf.len() / SECTOR {
        let end = (i + 1) * SECTOR - 2;
        buf[usa + 2 + i * 2] = buf[end];
        buf[usa + 3 + i * 2] = buf[end + 1];
        buf[end..end + 2].copy_from_slice(&usn);
    }
}

pub fn file_record(flags: u16, attrs: &[Vec<u8>]) -> Vec<u8> {
    let mut r = vec![0u8; REC];
    r[0..4].copy_from_slice(b"FILE");
    r[0x10..0x12].copy_from_slice(&SEQ.to_le_bytes());
    r[0x14..0x16].copy_from_slice(&0x38u16.to_le_bytes());
    r[0x16..0x18].copy_from_slice(&flags.to_le_bytes());
    let mut off = 0x38;
    for a in attrs {
        r[off..off + a.len()].copy_from_slice(a);
        off += a.len();
    }
    r[off..off + 4].copy_from_slice(&raw::ATTR_END.to_le_bytes());
    r[0x18..0x1C].copy_from_slice(&((off + 8) as u32).to_le_bytes());
    seal(&mut r, 0x30);
    r
}

pub const FILE: u16 = raw::MFT_RECORD_IN_USE;
pub const DIR: u16 = raw::MFT_RECORD_IN_USE | raw::MFT_RECORD_IS_DIRECTORY;

pub fn put(image: &mut [u8], record: u64, bytes: &[u8]) {
    let at = MFT_LCN * SECTOR + record as usize * REC;
    image[at..at + REC].copy_from_slice(bytes);
}

pub fn named(parent: u64, name: &str, dir: bool) -> Vec<u8> {
    resident(
        raw::ATTR_FILE_NAME,
        &[],
        &file_name_value(reference(parent), name, dir, raw::FILE_NAME_WIN32),
    )
}

/// A mountable image: `HELLO.TXT` (record 16, resident, with a named stream
/// `extra`), `SUBDIR` (17, empty), `BIN.DAT` (18, 3 bytes at LCN 120),
/// hidden entries for `$MFT` and `.`, and a DOS alias of `HELLO.TXT`.
pub fn base_image() -> Vec<u8> {
    let mut image = boot_sector();
    image.resize(IMAGE_LEN, 0);

    let mft_bytes = (MFT_RECORDS * REC) as u64;
    let clusters = mft_bytes / SECTOR as u64;
    let mft = non_resident(
        raw::ATTR_DATA,
        &[],
        clusters - 1,
        mft_bytes,
        mft_bytes,
        &[0x11, clusters as u8, MFT_LCN as u8, 0],
    );
    put(
        &mut image,
        raw::RECORD_MFT,
        &file_record(FILE, &[named(5, "$MFT", false), mft]),
    );

    let volume_name = resident(raw::ATTR_VOLUME_NAME, &[], &utf16("HADRIS"));
    put(
        &mut image,
        raw::RECORD_VOLUME,
        &file_record(FILE, &[volume_name]),
    );

    let mut bitmap = vec![0u8; IMAGE_LEN / SECTOR / 8];
    bitmap[..20].fill(0xFF);
    let bitmap = resident(raw::ATTR_DATA, &[], &bitmap);
    put(
        &mut image,
        raw::RECORD_BITMAP,
        &file_record(FILE, &[bitmap]),
    );

    let upcase = non_resident(
        raw::ATTR_DATA,
        &[],
        255,
        131_072,
        131_072,
        &[0x11, 0x01, UPCASE_LCN as u8, 0x02, 0xFF, 0x00, 0x00],
    );
    put(
        &mut image,
        raw::RECORD_UPCASE,
        &file_record(FILE, &[upcase]),
    );
    for unit in 0..256u16 {
        let upper = if (b'a' as u16..=b'z' as u16).contains(&unit)
            || (0xE0..=0xFE).contains(&unit) && unit != 0xF7
        {
            unit - 0x20
        } else {
            unit
        };
        let at = UPCASE_LCN * SECTOR + unit as usize * 2;
        image[at..at + 2].copy_from_slice(&upper.to_le_bytes());
    }

    let mut entries = index_entry(0, "$MFT", false, raw::FILE_NAME_WIN32_AND_DOS);
    entries.extend(index_entry(5, ".", true, raw::FILE_NAME_WIN32_AND_DOS));
    entries.extend(index_entry(16, "HELLO.TXT", false, raw::FILE_NAME_WIN32));
    entries.extend(index_entry(16, "HELLO~1.TXT", false, raw::FILE_NAME_DOS));
    entries.extend(index_entry(17, "SUBDIR", true, raw::FILE_NAME_WIN32));
    entries.extend(index_entry(18, "BIN.DAT", false, raw::FILE_NAME_POSIX));
    entries.extend(last_entry());
    put(
        &mut image,
        raw::RECORD_ROOT,
        &file_record(DIR, &[named(5, ".", true), index_root(&entries, 1024)]),
    );

    let hello = [
        standard_information(raw::FILE_ATTRIBUTE_READONLY | raw::FILE_ATTRIBUTE_ARCHIVE),
        named(5, "HELLO.TXT", false),
        resident(raw::ATTR_DATA, &[], b"hello ntfs"),
        resident(raw::ATTR_DATA, &utf16("extra"), b"side data"),
    ];
    put(&mut image, 16, &file_record(FILE, &hello));

    let sub = [
        standard_information(0),
        named(5, "SUBDIR", true),
        index_root(&last_entry(), 1024),
    ];
    put(&mut image, 17, &file_record(DIR, &sub));

    let bin = [
        named(5, "BIN.DAT", false),
        non_resident(
            raw::ATTR_DATA,
            &[],
            0,
            3,
            3,
            &[0x11, 0x01, BIN_LCN as u8, 0],
        ),
    ];
    put(&mut image, 18, &file_record(FILE, &bin));
    image[BIN_LCN * SECTOR..BIN_LCN * SECTOR + 3].copy_from_slice(b"bin");
    image
}

/// An INDX record holding `entries`.
pub fn indx(entries: &[u8]) -> Vec<u8> {
    let mut b = vec![0u8; REC];
    b[0..4].copy_from_slice(b"INDX");
    b[0x18..0x1C].copy_from_slice(&0x28u32.to_le_bytes());
    let total = 0x28 + entries.len() as u32;
    b[0x1C..0x20].copy_from_slice(&total.to_le_bytes());
    b[0x20..0x24].copy_from_slice(&total.to_le_bytes());
    b[0x40..0x40 + entries.len()].copy_from_slice(entries);
    seal(&mut b, 0x28);
    b
}

/// `SUBDIR` spills into two index blocks at LCN 130, the second of which the
/// bitmap marks unused unless `bitmap` says otherwise.
pub fn allocation_image(block_size: u32, bitmap: &[u8], blocks: Option<[Vec<u8>; 2]>) -> Vec<u8> {
    let mut image = base_image();
    let size = 2 * REC as u64;
    let sub = [
        named(5, "SUBDIR", true),
        index_root(&last_entry(), block_size),
        non_resident(
            raw::ATTR_INDEX_ALLOCATION,
            &raw::I30,
            size / 512 - 1,
            size,
            size,
            &[0x21, 0x04, INDX_LCN as u8, 0, 0],
        ),
        resident(raw::ATTR_BITMAP, &raw::I30, bitmap),
    ];
    put(&mut image, 17, &file_record(DIR, &sub));
    let blocks = blocks.unwrap_or_else(|| {
        let mut first = index_entry(19, "A.TXT", false, raw::FILE_NAME_WIN32);
        first.extend(index_entry(20, "B.TXT", false, raw::FILE_NAME_WIN32));
        first.extend(last_entry());
        let mut second = index_entry(21, "UNUSED.TXT", false, raw::FILE_NAME_WIN32);
        second.extend(last_entry());
        [indx(&first), indx(&second)]
    });
    for (i, block) in blocks.iter().enumerate() {
        let at = INDX_LCN * SECTOR + i * REC;
        image[at..at + REC].copy_from_slice(block);
    }
    for (record, name, data) in [
        (19, "A.TXT", &b"a"[..]),
        (20, "B.TXT", b"bb"),
        (21, "UNUSED.TXT", b"x"),
    ] {
        let attrs = [named(17, name, false), resident(raw::ATTR_DATA, &[], data)];
        put(&mut image, record, &file_record(FILE, &attrs));
    }
    image
}

/// Sets the instance number of an attribute built by `resident` or
/// `non_resident`.
pub fn with_id(mut attr: Vec<u8>, id: u16) -> Vec<u8> {
    attr[0x0E..0x10].copy_from_slice(&id.to_le_bytes());
    attr
}

/// Sets the first VCN of an attribute built by `non_resident`.
pub fn starting_at(mut attr: Vec<u8>, vcn: u64) -> Vec<u8> {
    attr[0x10..0x18].copy_from_slice(&vcn.to_le_bytes());
    attr
}

/// One `$ATTRIBUTE_LIST` entry.
pub fn list_entry(kind: u32, name: &[u8], start_vcn: u64, record: u64, id: u16) -> Vec<u8> {
    let len = (0x1A + name.len() + 7) & !7;
    let mut e = vec![0u8; len];
    e[0..4].copy_from_slice(&kind.to_le_bytes());
    e[4..6].copy_from_slice(&(len as u16).to_le_bytes());
    e[6] = (name.len() / 2) as u8;
    e[7] = 0x1A;
    e[8..16].copy_from_slice(&start_vcn.to_le_bytes());
    e[0x10..0x18].copy_from_slice(&reference(record).to_le_bytes());
    e[0x18..0x1A].copy_from_slice(&id.to_le_bytes());
    e[0x1A..0x1A + name.len()].copy_from_slice(name);
    e
}

/// `BIN.DAT` (record 18) holds its unnamed data in two extents in the
/// extension records 22 and 23, named by a resident `$ATTRIBUTE_LIST`,
/// and a named stream `alt` in record 22. The second extent starts at VCN
/// `second_vcn`; 2 makes the data contiguous.
pub fn listed_image(second_vcn: u64) -> (Vec<u8>, Vec<u8>) {
    let mut image = base_image();
    let len = 2000u64;
    let content: Vec<u8> = (0..len).map(|i| (i % 253) as u8).collect();
    let first = non_resident(raw::ATTR_DATA, &[], 1, len, len, &[0x21, 0x02, 150, 0, 0]);
    let second = starting_at(
        non_resident(
            raw::ATTR_DATA,
            &[],
            second_vcn + 1,
            0,
            0,
            &[0x21, 0x02, 160, 0, 0],
        ),
        second_vcn,
    );
    let alt = with_id(resident(raw::ATTR_DATA, &utf16("alt"), b"alternate"), 1);
    put(&mut image, 22, &file_record(FILE, &[first, alt]));
    put(&mut image, 23, &file_record(FILE, &[second]));
    let mut list = list_entry(raw::ATTR_FILE_NAME, &[], 0, 18, 3);
    list.extend(list_entry(raw::ATTR_DATA, &[], 0, 22, 0));
    list.extend(list_entry(raw::ATTR_DATA, &[], second_vcn, 23, 0));
    list.extend(list_entry(raw::ATTR_DATA, &utf16("alt"), 0, 22, 1));
    let base = [
        resident(raw::ATTR_ATTRIBUTE_LIST, &[], &list),
        with_id(named(5, "BIN.DAT", false), 3),
    ];
    put(&mut image, 18, &file_record(FILE, &base));
    image[150 * SECTOR..152 * SECTOR].copy_from_slice(&content[..1024]);
    image[160 * SECTOR..160 * SECTOR + 976].copy_from_slice(&content[1024..]);
    (image, content)
}

/// `$MFT` in two extents: records 0 to 15 at LCN 4 and records 16 to 31 at
/// LCN 200, the second extent in extension record 15.
pub fn fragmented_mft_image() -> Vec<u8> {
    let mut image = base_image();
    let tail = MFT_LCN * SECTOR + 16 * REC;
    let moved = image[tail..tail + 16 * REC].to_vec();
    image[tail..tail + 16 * REC].fill(0);
    image[200 * SECTOR..200 * SECTOR + 16 * REC].copy_from_slice(&moved);
    let size = (MFT_RECORDS * REC) as u64;
    let first = with_id(
        non_resident(
            raw::ATTR_DATA,
            &[],
            31,
            size,
            size,
            &[0x11, 0x20, MFT_LCN as u8, 0],
        ),
        2,
    );
    let second = starting_at(
        non_resident(raw::ATTR_DATA, &[], 63, 0, 0, &[0x21, 0x20, 200, 0, 0]),
        32,
    );
    put(&mut image, 15, &file_record(FILE, &[second]));
    let mut list = list_entry(raw::ATTR_FILE_NAME, &[], 0, 0, 1);
    list.extend(list_entry(raw::ATTR_DATA, &[], 0, 0, 2));
    list.extend(list_entry(raw::ATTR_DATA, &[], 32, 15, 0));
    let base = [
        resident(raw::ATTR_ATTRIBUTE_LIST, &[], &list),
        with_id(named(5, "$MFT", false), 1),
        first,
    ];
    put(&mut image, raw::RECORD_MFT, &file_record(FILE, &base));
    image
}
