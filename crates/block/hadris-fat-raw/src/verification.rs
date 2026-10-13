use crate::{ChainError, FatKind, exfat};

mod allocation;
mod checksums;
mod geometry;
mod names;

#[kani::proof]
fn fat12_updates_preserve_neighbor() {
    let mut bytes: [u8; 3] = kani::any();
    let original = bytes;
    let value: u32 = kani::any();
    let odd: bool = kani::any();
    let cluster = u64::from(odd);
    let start = usize::from(odd);
    FatKind::Fat12.encode(cluster, value, &mut bytes[start..start + 2]);
    let even_entry = u32::from(bytes[0]) | (u32::from(bytes[1] & 0x0f) << 8);
    let odd_entry = u32::from(bytes[1] >> 4) | (u32::from(bytes[2]) << 4);
    if odd {
        assert_eq!(odd_entry, value & 0x0fff);
        assert_eq!(bytes[0], original[0]);
        assert_eq!(bytes[1] & 0x0f, original[1] & 0x0f);
    } else {
        assert_eq!(even_entry, value & 0x0fff);
        assert_eq!(bytes[1] & 0xf0, original[1] & 0xf0);
        assert_eq!(bytes[2], original[2]);
    }
    assert_eq!(
        FatKind::Fat12.decode(cluster, &bytes[start..start + 2]),
        value & 0x0fff
    );
}

#[kani::proof]
fn fat16_entries_are_little_endian() {
    let mut bytes: [u8; 2] = kani::any();
    let value: u32 = kani::any();
    let cluster: u64 = kani::any();
    FatKind::Fat16.encode(cluster, value, &mut bytes);
    assert_eq!(bytes[0], (value & 0xff) as u8);
    assert_eq!(bytes[1], ((value >> 8) & 0xff) as u8);
    assert_eq!(FatKind::Fat16.decode(cluster, &bytes), value & 0xffff);
}

#[kani::proof]
fn fat32_updates_preserve_reserved_bits() {
    let mut bytes: [u8; 4] = kani::any();
    let reserved = bytes[3] & 0xf0;
    let value: u32 = kani::any();
    let cluster: u64 = kani::any();
    FatKind::Fat32.encode(cluster, value, &mut bytes);
    let stored = u32::from(bytes[0])
        | (u32::from(bytes[1]) << 8)
        | (u32::from(bytes[2]) << 16)
        | (u32::from(bytes[3]) << 24);
    assert_eq!(stored & 0x0fff_ffff, value & 0x0fff_ffff);
    assert_eq!(bytes[3] & 0xf0, reserved);
    assert_eq!(FatKind::Fat32.decode(cluster, &bytes), stored);
}

#[kani::proof]
fn fat_chain_markers_match_specification() {
    let selector: u8 = kani::any();
    kani::assume(selector < 3);
    let (kind, mask, bad, end) = match selector {
        0 => (FatKind::Fat12, 0x0fff, 0x0ff7, 0x0ff8),
        1 => (FatKind::Fat16, 0xffff, 0xfff7, 0xfff8),
        _ => (FatKind::Fat32, 0x0fff_ffff, 0x0fff_fff7, 0x0fff_fff8),
    };
    let stored: u32 = kani::any();
    let max_cluster: u32 = kani::any();
    let value = stored & mask;
    let expected = if value >= end {
        Ok(None)
    } else if value == bad {
        Err(ChainError::Bad)
    } else if value < 2 || value > max_cluster {
        Err(ChainError::OutOfBounds(value))
    } else {
        Ok(Some(value))
    };
    assert_eq!(kind.next(stored, max_cluster), expected);
}

fn checksum_byte(sum: u16, byte: u8) -> u16 {
    (((u32::from(sum) >> 1) | ((u32::from(sum) & 1) << 15)) + u32::from(byte)) as u16
}

#[kani::proof]
#[kani::unwind(33)]
fn exfat_entry_checksum_matches_specification() {
    let entry: [u8; 32] = kani::any();
    let initial: u16 = kani::any();
    let index: usize = kani::any();
    let mut expected = initial;
    for (offset, byte) in entry.iter().enumerate() {
        if index != 0 || !(2..4).contains(&offset) {
            expected = checksum_byte(expected, *byte);
        }
    }
    assert_eq!(exfat::set_checksum_step(initial, index, &entry), expected);
}

#[kani::proof]
#[kani::unwind(3)]
fn exfat_name_hash_matches_specification() {
    let initial: u16 = kani::any();
    let unit: u16 = kani::any();
    let low = checksum_byte(initial, (unit & 0xff) as u8);
    let expected = checksum_byte(low, (unit >> 8) as u8);
    assert_eq!(exfat::hash_unit(initial, unit), expected);
}
