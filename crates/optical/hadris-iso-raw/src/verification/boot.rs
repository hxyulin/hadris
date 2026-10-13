use crate::{BootInfoTable, BootValidationEntry};

#[kani::proof]
#[kani::unwind(17)]
fn el_torito_validation_checksum_matches_specification() {
    let raw: [u8; 32] = kani::any();
    let entry: BootValidationEntry = bytemuck::cast(raw);
    let mut all = 0u16;
    let mut others = 0u16;
    for word in 0..16 {
        let value = u16::from(raw[2 * word]) | (u16::from(raw[2 * word + 1]) << 8);
        all = all.wrapping_add(value);
        if word != 14 {
            others = others.wrapping_add(value);
        }
    }
    assert_eq!(entry.expected_checksum(), 0u16.wrapping_sub(others));
    assert_eq!(
        entry.is_valid(),
        raw[0] == 1 && raw[30] == 0x55 && raw[31] == 0xaa && all == 0
    );
    kani::cover!(entry.is_valid(), "valid entry");
    let platform: u8 = kani::any();
    let made = BootValidationEntry::new(platform);
    assert!(made.is_valid());
    assert_eq!(made.platform_id, platform);
}

const IMAGE: usize = 16;

#[kani::proof]
#[kani::unwind(17)]
fn boot_info_checksum_sums_zero_padded_words() {
    let bytes: [u8; IMAGE] = kani::any();
    let len: usize = kani::any();
    let split: usize = kani::any();
    let initial: u32 = kani::any();
    kani::assume(len <= IMAGE && split <= len && split % 4 == 0);
    let mut expected = initial;
    for (index, byte) in bytes[..len].iter().enumerate() {
        expected = expected.wrapping_add(u32::from(*byte) << (8 * (index % 4)));
    }
    let whole = BootInfoTable::word_sum(initial, &bytes[..len]);
    assert_eq!(whole, expected);
    let first = BootInfoTable::word_sum(initial, &bytes[..split]);
    assert_eq!(BootInfoTable::word_sum(first, &bytes[split..len]), whole);
    kani::cover!(len % 4 != 0 && split > 0, "partial last word after a split");
}
