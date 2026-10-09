use crate::exfat;

fn byte32(sum: u32, byte: u8) -> u32 {
    ((sum >> 1) | ((sum & 1) << 31)).wrapping_add(u32::from(byte))
}

#[kani::proof]
#[kani::unwind(114)]
fn exfat_boot_checksum_excludes_only_mutable_fields() {
    let bytes: [u8; 113] = kani::any();
    let initial: u32 = kani::any();
    let sector: u64 = kani::any();
    let mut expected = initial;
    for (offset, byte) in bytes.iter().enumerate() {
        if sector != 0 || !matches!(offset, 106 | 107 | 112) {
            expected = byte32(expected, *byte);
        }
    }
    assert_eq!(exfat::boot_checksum(initial, sector, &bytes), expected);
}

#[kani::proof]
#[kani::unwind(9)]
fn exfat_table_checksum_matches_bounded_prefixes() {
    let bytes: [u8; 8] = kani::any();
    let length: usize = kani::any();
    kani::assume(length <= 8);
    let initial: u32 = kani::any();
    let mut expected = initial;
    for byte in &bytes[..length] {
        expected = byte32(expected, *byte);
    }
    assert_eq!(exfat::table_checksum(initial, &bytes[..length]), expected);
}

#[kani::proof]
#[kani::unwind(9)]
fn exfat_name_hash_matches_bounded_names() {
    let units: [u16; 8] = kani::any();
    let length: usize = kani::any();
    kani::assume(length <= 8);
    let mut expected = 0;
    for unit in &units[..length] {
        expected = super::checksum_byte(expected, (*unit & 255) as u8);
        expected = super::checksum_byte(expected, (*unit >> 8) as u8);
    }
    assert_eq!(exfat::name_hash(&units[..length]), expected);
}
