use core::mem;

use hadris_fat_raw::{RawBpb, RawBpbExt32, RawFsInfo};

fn roundtrip<T: bytemuck::AnyBitPattern + bytemuck::NoUninit>(expected_size: usize) {
    assert_eq!(mem::size_of::<T>(), expected_size);
    assert_eq!(mem::align_of::<T>(), 1);
    for byte in 0..=u8::MAX {
        let bytes = vec![byte; expected_size];
        let value: T = bytemuck::pod_read_unaligned(&bytes);
        assert_eq!(bytemuck::bytes_of(&value), bytes, "byte {byte:#04x}");
    }
    let bytes: Vec<u8> = (0..expected_size).map(|index| index as u8).collect();
    let value: T = bytemuck::pod_read_unaligned(&bytes);
    assert_eq!(bytemuck::bytes_of(&value), bytes);
}

#[test]
fn raw_bpb_accepts_and_preserves_arbitrary_bytes() {
    roundtrip::<RawBpb>(36);
}

#[test]
fn raw_bpb_ext32_accepts_and_preserves_arbitrary_bytes() {
    roundtrip::<RawBpbExt32>(476);
}

#[test]
fn raw_fs_info_accepts_and_preserves_arbitrary_bytes() {
    roundtrip::<RawFsInfo>(512);
}
