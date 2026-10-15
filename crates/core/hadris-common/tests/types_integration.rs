//! Integration tests for hadris-common types.

use hadris_common::types::endian::LittleEndian;
use hadris_common::types::number::{U16, U32, U64};

#[test]
fn u16_le_roundtrip() {
    let le = U16::<LittleEndian>::new(0xABCD);
    assert_eq!(le.get(), 0xABCD);
    assert_eq!(bytemuck::bytes_of(&le), [0xCD, 0xAB]);
}

#[test]
fn u32_le_roundtrip() {
    let le = U32::<LittleEndian>::new(0xDEADBEEF);
    assert_eq!(le.get(), 0xDEADBEEF);
    assert_eq!(bytemuck::bytes_of(&le), [0xEF, 0xBE, 0xAD, 0xDE]);
}

#[test]
fn u64_le_roundtrip() {
    let le = U64::<LittleEndian>::new(0x0123_4567_89AB_CDEF);
    assert_eq!(le.get(), 0x0123_4567_89AB_CDEF);
}

#[test]
fn set_and_get() {
    let mut le = U32::<LittleEndian>::new(0);
    le.set(42);
    assert_eq!(le.get(), 42);
    le.set(u32::MAX);
    assert_eq!(le.get(), u32::MAX);
}

#[test]
fn number_display() {
    let v = U16::<LittleEndian>::new(1234);
    assert_eq!(format!("{v}"), "1234");
    assert_eq!(format!("{v:x}"), "0x04d2");
    assert_eq!(format!("{v:?}"), "U16(1234)");

    let v = U32::<LittleEndian>::new(0xDEAD);
    assert_eq!(format!("{v:X}"), "0x0000DEAD");
}

#[test]
fn bytemuck_roundtrip_u32() {
    let original = U32::<LittleEndian>::new(0x12345678);
    let restored: &U32<LittleEndian> = bytemuck::from_bytes(bytemuck::bytes_of(&original));
    assert_eq!(restored.get(), 0x12345678);
}
