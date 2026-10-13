use crate::name;

const NAME: usize = 8;

fn unit(raw: &[u8], index: usize, ucs2: bool) -> u16 {
    if ucs2 {
        u16::from_be_bytes([raw[2 * index], raw[2 * index + 1]])
    } else {
        u16::from(raw[index])
    }
}

#[kani::proof]
#[kani::unwind(10)]
fn file_identifier_version_matches_specification() {
    let bytes: [u8; NAME] = kani::any();
    let len: usize = kani::any();
    let ucs2: bool = kani::any();
    kani::assume(len <= NAME);
    let raw = &bytes[..len];
    let width = if ucs2 { 2 } else { 1 };
    let units = len / width;
    let mut separator = None;
    let mut index = 0;
    while index < units {
        if unit(raw, index, ucs2) == u16::from(b';') {
            separator = Some(index);
        }
        index += 1;
    }
    let mut expected = None;
    if let Some(at) = separator.filter(|&at| len % width == 0 && at + 1 < units) {
        let mut value = 0u32;
        let mut digits = true;
        let mut index = at + 1;
        while index < units {
            let digit = unit(raw, index, ucs2).wrapping_sub(u16::from(b'0'));
            digits &= digit < 10;
            value = value * 10 + u32::from(digit.min(9));
            index += 1;
        }
        if digits && (1..=32767).contains(&value) {
            expected = Some(value as u16);
        }
    }
    assert_eq!(name::version(raw, ucs2), expected);
    kani::cover!(ucs2 && expected.is_some(), "UCS-2 version");
    kani::cover!(!ucs2 && expected == Some(32767), "largest version");

    if ucs2 {
        return;
    }
    let stripped = name::strip_version(raw);
    let versioned =
        separator.is_some_and(|at| at + 1 < len && raw[at + 1..].iter().all(u8::is_ascii_digit));
    match separator.filter(|_| versioned) {
        None => assert_eq!(stripped, raw),
        Some(at) if at >= 2 && raw[at - 1] == b'.' => assert_eq!(stripped, &raw[..at - 1]),
        Some(at) => assert_eq!(stripped, &raw[..at]),
    }
    if expected.is_some() {
        assert!(versioned);
    }
}

#[cfg(feature = "alloc")]
#[kani::proof]
fn primary_name_characters_are_d_characters() {
    use crate::options::NameCase;
    use crate::plan::names::convert_char;

    let c: char = kani::any();
    let upper = convert_char(c, NameCase::Upper);
    assert!(upper.is_ascii_uppercase() || upper.is_ascii_digit() || upper == b'_');
    let kept = convert_char(c, NameCase::Preserve);
    assert!(kept.is_ascii_alphanumeric() || kept == b'_');
    if c.is_ascii_alphanumeric() || c == '_' {
        assert_eq!(char::from(upper), c.to_ascii_uppercase());
        assert_eq!(char::from(kept), c);
    } else {
        assert_eq!((upper, kept), (b'_', b'_'));
    }
}
