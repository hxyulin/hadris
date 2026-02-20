//! Integration tests for hadris-common types.

use hadris_common::types::endian::*;
use hadris_common::types::extent::*;
use hadris_common::types::file::FixedFilename;
use hadris_common::types::number::*;

// ---------------------------------------------------------------------------
// Endian number types
// ---------------------------------------------------------------------------

#[test]
fn u16_le_roundtrip() {
    let original: u16 = 0xABCD;
    let le = U16::<LittleEndian>::new(original);
    assert_eq!(le.get(), original);

    // Verify byte representation
    let bytes: [u8; 2] = bytemuck::bytes_of(&le).try_into().unwrap();
    assert_eq!(bytes, [0xCD, 0xAB]);
}

#[test]
fn u16_be_roundtrip() {
    let original: u16 = 0xABCD;
    let be = U16::<BigEndian>::new(original);
    assert_eq!(be.get(), original);

    let bytes: [u8; 2] = bytemuck::bytes_of(&be).try_into().unwrap();
    assert_eq!(bytes, [0xAB, 0xCD]);
}

#[test]
fn u32_le_roundtrip() {
    let original: u32 = 0xDEADBEEF;
    let le = U32::<LittleEndian>::new(original);
    assert_eq!(le.get(), original);

    let bytes: [u8; 4] = bytemuck::bytes_of(&le).try_into().unwrap();
    assert_eq!(bytes, [0xEF, 0xBE, 0xAD, 0xDE]);
}

#[test]
fn u32_be_roundtrip() {
    let original: u32 = 0xDEADBEEF;
    let be = U32::<BigEndian>::new(original);
    assert_eq!(be.get(), original);

    let bytes: [u8; 4] = bytemuck::bytes_of(&be).try_into().unwrap();
    assert_eq!(bytes, [0xDE, 0xAD, 0xBE, 0xEF]);
}

#[test]
fn u64_roundtrip() {
    let original: u64 = 0x0123_4567_89AB_CDEF;
    let le = U64::<LittleEndian>::new(original);
    let be = U64::<BigEndian>::new(original);
    assert_eq!(le.get(), original);
    assert_eq!(be.get(), original);
}

#[test]
fn u24_roundtrip() {
    let original: u32 = 0x123456;
    let le = U24::<LittleEndian>::new(original);
    let be = U24::<BigEndian>::new(original);
    assert_eq!(le.get(), original);
    assert_eq!(be.get(), original);
}

#[test]
fn endian_set_and_get() {
    let mut le = U32::<LittleEndian>::new(0);
    le.set(42);
    assert_eq!(le.get(), 42);

    le.set(u32::MAX);
    assert_eq!(le.get(), u32::MAX);

    le.set(0);
    assert_eq!(le.get(), 0);
}

#[test]
fn endian_type_display() {
    assert_eq!(format!("{}", EndianType::LittleEndian), "little-endian");
    assert_eq!(format!("{}", EndianType::BigEndian), "big-endian");
    assert_eq!(format!("{}", EndianType::NativeEndian), "native");
}

#[test]
fn number_display() {
    let v = U16::<LittleEndian>::new(1234);
    assert_eq!(format!("{}", v), "1234");
    assert_eq!(format!("{:x}", v), "0x04d2");

    let v = U32::<BigEndian>::new(0xDEAD);
    assert_eq!(format!("{}", v), "57005");
    assert_eq!(format!("{:X}", v), "0x0000DEAD");
}

#[test]
fn bytemuck_roundtrip_u32() {
    let original = U32::<LittleEndian>::new(0x12345678);
    let bytes = bytemuck::bytes_of(&original);
    let restored: &U32<LittleEndian> = bytemuck::from_bytes(bytes);
    assert_eq!(restored.get(), 0x12345678);
}

// ---------------------------------------------------------------------------
// Extent
// ---------------------------------------------------------------------------

#[test]
fn extent_empty() {
    let e = Extent::new(0, 0);
    assert!(e.is_empty());
    assert_eq!(e.sector_count(2048), 0);
    assert_eq!(e.end_sector(2048), 0);
}

#[test]
fn extent_partial_sector() {
    // 1 byte should still occupy 1 sector
    let e = Extent::new(10, 1);
    assert_eq!(e.sector_count(2048), 1);
    assert_eq!(e.end_sector(2048), 11);
}

#[test]
fn extent_exact_sector_boundary() {
    let e = Extent::new(0, 2048);
    assert_eq!(e.sector_count(2048), 1);
    assert_eq!(e.end_sector(2048), 1);
}

#[test]
fn extent_display() {
    let e = Extent::new(100, 4096);
    assert_eq!(format!("{}", e), "sector 100 (4096 bytes)");
}

#[test]
fn extent_overlap_self() {
    let e = Extent::new(10, 4096);
    assert!(e.overlaps(&e, 2048));
}

#[test]
fn extent_no_overlap_adjacent() {
    let a = Extent::new(0, 2048); // sector 0
    let b = Extent::new(1, 2048); // sector 1
    assert!(!a.overlaps(&b, 2048));
}

// ---------------------------------------------------------------------------
// FileType
// ---------------------------------------------------------------------------

#[test]
fn file_type_display() {
    assert_eq!(format!("{}", FileType::RegularFile), "file");
    assert_eq!(format!("{}", FileType::Directory), "directory");
    assert_eq!(format!("{}", FileType::Symlink), "symlink");
}

#[test]
fn file_type_default() {
    assert_eq!(FileType::default(), FileType::RegularFile);
}

// ---------------------------------------------------------------------------
// FixedFilename
// ---------------------------------------------------------------------------

#[test]
fn fixed_filename_push_operations() {
    let mut name = FixedFilename::<32>::empty();
    assert!(name.is_empty());

    name.push_slice(b"hello");
    assert_eq!(name.len(), 5);
    assert_eq!(name.as_str(), "hello");

    name.push_byte(b'.');
    name.push_slice(b"txt");
    assert_eq!(name.as_str(), "hello.txt");
}

#[test]
fn fixed_filename_try_push_overflow() {
    let mut name = FixedFilename::<5>::empty();
    name.push_slice(b"hello");
    assert_eq!(name.remaining_capacity(), 0);

    assert!(name.try_push_byte(b'!').is_none());
    assert!(name.try_push_slice(b"x").is_none());
}

#[test]
fn fixed_filename_truncate() {
    let mut name = FixedFilename::<32>::from(b"hello.txt".as_slice());
    name.truncate(5);
    assert_eq!(name.as_str(), "hello");
}

#[test]
fn fixed_filename_display() {
    let name = FixedFilename::<32>::from(b"test.iso".as_slice());
    assert_eq!(format!("{}", name), "test.iso");
}

// ---------------------------------------------------------------------------
// align_up
// ---------------------------------------------------------------------------

#[test]
fn align_up_already_aligned() {
    assert_eq!(align_up(2048u32, 2048), 2048);
}

#[test]
fn align_up_needs_alignment() {
    assert_eq!(align_up(1u32, 2048), 2048);
    assert_eq!(align_up(2049u32, 2048), 4096);
}

// ---------------------------------------------------------------------------
// Path utilities
// ---------------------------------------------------------------------------

#[test]
fn split_path_file_only() {
    let (dir, file) = hadris_common::path::split_path("file.txt").unwrap();
    assert_eq!(dir, "");
    assert_eq!(file, "file.txt");
}

#[test]
fn split_path_nested() {
    let (dir, file) = hadris_common::path::split_path("a/b/c/file.txt").unwrap();
    assert_eq!(dir, "a/b/c");
    assert_eq!(file, "file.txt");
}

#[test]
fn split_path_leading_slash() {
    let (dir, file) = hadris_common::path::split_path("/root/file.txt").unwrap();
    assert_eq!(dir, "root");
    assert_eq!(file, "file.txt");
}

#[test]
fn split_path_empty() {
    assert!(hadris_common::path::split_path("").is_none());
    assert!(hadris_common::path::split_path("/").is_none());
}
