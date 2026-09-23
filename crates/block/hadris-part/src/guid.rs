use core::fmt;
use core::str::FromStr;

/// A 128-bit GUID in the mixed-endian byte order GPT stores.
///
/// The first three groups of the text form are little-endian on disk, the
/// last two big-endian. [`Display`](fmt::Display) prints the lowercase text
/// form, and [`FromStr`] parses it in either case, with or without braces.
///
/// ```rust
/// use hadris_part::Guid;
///
/// let esp: Guid = "C12A7328-F81F-11D2-BA4B-00A0C93EC93B".parse().unwrap();
/// assert_eq!(esp, hadris_part::gpt::types::EFI_SYSTEM);
/// assert_eq!(esp.to_string(), "c12a7328-f81f-11d2-ba4b-00a0c93ec93b");
/// ```
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Default)]
pub struct Guid([u8; 16]);

impl Guid {
    /// The all-zero GUID, the type of an unused GPT entry.
    pub const NIL: Self = Self([0; 16]);

    /// A GUID from its 16 on-disk bytes.
    pub const fn from_bytes(bytes: [u8; 16]) -> Self {
        Self(bytes)
    }

    /// The 16 on-disk bytes.
    pub const fn to_bytes(self) -> [u8; 16] {
        self.0
    }

    /// A GUID from the five groups of its text form, as numbers.
    pub const fn from_fields(time_low: u32, time_mid: u16, time_hi: u16, rest: [u8; 8]) -> Self {
        let a = time_low.to_le_bytes();
        let b = time_mid.to_le_bytes();
        let c = time_hi.to_le_bytes();
        Self([
            a[0], a[1], a[2], a[3], b[0], b[1], c[0], c[1], rest[0], rest[1], rest[2], rest[3],
            rest[4], rest[5], rest[6], rest[7],
        ])
    }

    /// Whether every byte is zero.
    pub const fn is_nil(self) -> bool {
        let mut i = 0;
        while i < 16 {
            if self.0[i] != 0 {
                return false;
            }
            i += 1;
        }
        true
    }

    /// Parses the 36-character text form `xxxxxxxx-xxxx-xxxx-xxxx-xxxxxxxxxxxx`
    /// in a `const` context, returning `None` when it is malformed.
    ///
    /// ```rust
    /// use hadris_part::Guid;
    ///
    /// const LINUX: Guid = match Guid::parse_const("0FC63DAF-8483-4772-8E79-3D69D8477DE4") {
    ///     Some(guid) => guid,
    ///     None => panic!("bad GUID"),
    /// };
    /// assert_eq!(LINUX, hadris_part::gpt::types::LINUX_FILESYSTEM);
    /// ```
    pub const fn parse_const(text: &str) -> Option<Self> {
        let s = text.as_bytes();
        if s.len() != 36 || s[8] != b'-' || s[13] != b'-' || s[18] != b'-' || s[23] != b'-' {
            return None;
        }
        let mut digits = [0u8; 32];
        let mut n = 0;
        let mut i = 0;
        while i < 36 {
            if i != 8 && i != 13 && i != 18 && i != 23 {
                digits[n] = match hex(s[i]) {
                    Some(v) => v,
                    None => return None,
                };
                n += 1;
            }
            i += 1;
        }
        let mut text_order = [0u8; 16];
        let mut k = 0;
        while k < 16 {
            text_order[k] = (digits[2 * k] << 4) | digits[2 * k + 1];
            k += 1;
        }
        let t = text_order;
        Some(Self([
            t[3], t[2], t[1], t[0], t[5], t[4], t[7], t[6], t[8], t[9], t[10], t[11], t[12], t[13],
            t[14], t[15],
        ]))
    }

    /// A random version 4 GUID from the standard library's hasher keys,
    /// which the operating system seeds.
    ///
    /// Hadris never makes one on its own: every constructor that needs a
    /// GUID takes it.
    #[cfg(feature = "std")]
    #[cfg_attr(docsrs, doc(cfg(feature = "std")))]
    pub fn random() -> Self {
        use std::hash::{BuildHasher, Hasher};

        static COUNTER: core::sync::atomic::AtomicU64 = core::sync::atomic::AtomicU64::new(0);
        let state = std::collections::hash_map::RandomState::new();
        let mut bytes = [0u8; 16];
        for (i, chunk) in bytes.chunks_mut(8).enumerate() {
            let mut hasher = state.build_hasher();
            hasher.write_u64(COUNTER.fetch_add(1, core::sync::atomic::Ordering::Relaxed));
            hasher.write_usize(i);
            if let Ok(now) = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH) {
                hasher.write_u128(now.as_nanos());
            }
            chunk.copy_from_slice(&hasher.finish().to_le_bytes());
        }
        Self(bytes).with_version_4()
    }

    /// This GUID with the version 4 and RFC 4122 variant bits set.
    pub(crate) const fn with_version_4(mut self) -> Self {
        self.0[7] = (self.0[7] & 0x0F) | 0x40;
        self.0[8] = (self.0[8] & 0x3F) | 0x80;
        self
    }

    /// A version 4 GUID derived from this one and `index`, the same for the
    /// same inputs.
    pub(crate) const fn derive(self, index: u64) -> Self {
        let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
        let mut bytes = self.0;
        let mut i = 0;
        while i < 16 {
            hash ^= self.0[i] as u64;
            hash = hash.wrapping_mul(0x0100_0000_01b3);
            i += 1;
        }
        let index = index.to_le_bytes();
        let mut j = 0;
        while j < 8 {
            hash ^= index[j] as u64;
            hash = hash.wrapping_mul(0x0100_0000_01b3);
            j += 1;
        }
        let mixed = hash.to_le_bytes();
        let mut k = 0;
        while k < 8 {
            bytes[k] ^= mixed[k];
            bytes[k + 8] ^= mixed[7 - k];
            k += 1;
        }
        Self(bytes).with_version_4()
    }
}

const fn hex(c: u8) -> Option<u8> {
    match c {
        b'0'..=b'9' => Some(c - b'0'),
        b'a'..=b'f' => Some(c - b'a' + 10),
        b'A'..=b'F' => Some(c - b'A' + 10),
        _ => None,
    }
}

impl fmt::Display for Guid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let b = &self.0;
        write!(
            f,
            "{:02x}{:02x}{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
            b[3],
            b[2],
            b[1],
            b[0],
            b[5],
            b[4],
            b[7],
            b[6],
            b[8],
            b[9],
            b[10],
            b[11],
            b[12],
            b[13],
            b[14],
            b[15]
        )
    }
}

impl fmt::Debug for Guid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Guid({self})")
    }
}

/// The text given to [`Guid::from_str`] is not a GUID.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GuidParseError;

impl fmt::Display for GuidParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("invalid GUID text")
    }
}

impl core::error::Error for GuidParseError {}

impl FromStr for Guid {
    type Err = GuidParseError;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        let inner = text
            .strip_prefix('{')
            .and_then(|rest| rest.strip_suffix('}'))
            .unwrap_or(text);
        Self::parse_const(inner).ok_or(GuidParseError)
    }
}

#[cfg(test)]
mod tests {
    extern crate alloc;

    use super::*;
    use alloc::string::ToString;

    #[test]
    fn text_form_round_trips() {
        let text = "c12a7328-f81f-11d2-ba4b-00a0c93ec93b";
        let guid: Guid = text.parse().unwrap();
        assert_eq!(
            guid.to_bytes(),
            [
                0x28, 0x73, 0x2A, 0xC1, 0x1F, 0xF8, 0xD2, 0x11, 0xBA, 0x4B, 0x00, 0xA0, 0xC9, 0x3E,
                0xC9, 0x3B
            ]
        );
        assert_eq!(guid.to_string(), text);
        assert_eq!(
            Guid::from_fields(
                0xC12A_7328,
                0xF81F,
                0x11D2,
                [0xBA, 0x4B, 0, 0xA0, 0xC9, 0x3E, 0xC9, 0x3B]
            ),
            guid
        );
        assert_eq!("{C12A7328-F81F-11D2-BA4B-00A0C93EC93B}".parse(), Ok(guid));
    }

    #[test]
    fn malformed_text_is_rejected() {
        for bad in [
            "",
            "c12a7328f81f11d2ba4b00a0c93ec93b",
            "c12a7328-f81f-11d2-ba4b-00a0c93ec93",
            "c12a7328-f81f-11d2-ba4b-00a0c93ec93g",
            "c12a7328+f81f-11d2-ba4b-00a0c93ec93b",
            "{c12a7328-f81f-11d2-ba4b-00a0c93ec93b",
            "é12a7328-f81f-11d2-ba4b-00a0c93ec93",
        ] {
            assert_eq!(bad.parse::<Guid>(), Err(GuidParseError), "{bad}");
        }
    }

    #[test]
    fn derived_guids_are_stable_version_4_and_distinct() {
        let disk = Guid::from_bytes([7; 16]);
        let a = disk.derive(0);
        assert_eq!(a, disk.derive(0));
        assert_ne!(a, disk.derive(1));
        assert_eq!(a.to_bytes()[7] >> 4, 4);
        assert_eq!(a.to_bytes()[8] >> 6, 0b10);
    }

    #[cfg(feature = "std")]
    #[cfg_attr(miri, ignore = "Miri isolation has no clock or entropy")]
    #[test]
    fn random_guids_differ() {
        let a = Guid::random();
        assert_ne!(a, Guid::random());
        assert_eq!(a.to_bytes()[7] >> 4, 4);
    }
}
