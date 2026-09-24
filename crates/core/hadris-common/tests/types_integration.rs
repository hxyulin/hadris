//! Integration tests for hadris-common types.

use hadris_common::types::endian::*;
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
fn number_display() {
    let v = U16::<LittleEndian>::new(1234);
    assert_eq!(format!("{v}"), "1234");
    assert_eq!(format!("{v:x}"), "0x04d2");

    let v = U32::<BigEndian>::new(0xDEAD);
    assert_eq!(format!("{v}"), "57005");
    assert_eq!(format!("{v:X}"), "0x0000DEAD");
}

#[test]
fn bytemuck_roundtrip_u32() {
    let original = U32::<LittleEndian>::new(0x12345678);
    let bytes = bytemuck::bytes_of(&original);
    let restored: &U32<LittleEndian> = bytemuck::from_bytes(bytes);
    assert_eq!(restored.get(), 0x12345678);
}

// ---------------------------------------------------------------------------
// Alignment
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
