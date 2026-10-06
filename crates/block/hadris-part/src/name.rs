use core::fmt;

use hadris_fs::ErrorKind;

use crate::error::{Detail, TableError};

/// Code units in a GPT partition name.
const UNITS: usize = 36;

/// A GPT partition name: up to 36 UTF-16 code units.
///
/// Names read from disk are kept as stored, including unpaired surrogates;
/// [`chars`](Self::chars) and [`Display`](fmt::Display) replace those with
/// U+FFFD, and [`to_str`](Self::to_str) reports them.
///
/// ```rust
/// use hadris_part::PartitionName;
///
/// let name = PartitionName::new("Système EFI").unwrap();
/// assert_eq!(name.to_string(), "Système EFI");
/// assert_eq!(name.units().len(), 11);
/// ```
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct PartitionName {
    units: [u16; UNITS],
}

impl PartitionName {
    /// The empty name.
    pub const EMPTY: Self = Self { units: [0; UNITS] };

    /// Encodes `name` as UTF-16.
    ///
    /// Fails with [`ErrorKind::NameTooLong`] beyond 36 code units (a
    /// character outside the Basic Multilingual Plane takes two) and with
    /// [`ErrorKind::InvalidInput`] when it contains a NUL.
    pub fn new(name: &str) -> Result<Self, TableError> {
        let mut units = [0u16; UNITS];
        for (len, unit) in name.encode_utf16().enumerate() {
            if unit == 0 {
                return Err(TableError::invalid(Detail::Name));
            }
            let slot = units
                .get_mut(len)
                .ok_or(TableError::new(ErrorKind::NameTooLong, Detail::Name))?;
            *slot = unit;
        }
        Ok(Self { units })
    }

    /// A name from raw UTF-16 code units, which need not be valid UTF-16.
    /// The name ends at the first NUL.
    ///
    /// Fails with [`ErrorKind::NameTooLong`] beyond 36 units.
    pub fn from_units(units: &[u16]) -> Result<Self, TableError> {
        let used = units.iter().position(|&u| u == 0).unwrap_or(units.len());
        let mut name = Self::EMPTY;
        name.units
            .get_mut(..used)
            .ok_or(TableError::new(ErrorKind::NameTooLong, Detail::Name))?
            .copy_from_slice(&units[..used]);
        Ok(name)
    }

    /// Decodes the 72 on-disk bytes of a GPT entry name (UTF-16LE).
    pub const fn from_le_bytes(bytes: [u8; 72]) -> Self {
        let mut units = [0u16; UNITS];
        let mut i = 0;
        while i < UNITS {
            units[i] = u16::from_le_bytes([bytes[2 * i], bytes[2 * i + 1]]);
            i += 1;
        }
        Self { units }
    }

    /// The 72 on-disk bytes, zero-padded after the name.
    pub const fn to_le_bytes(&self) -> [u8; 72] {
        let mut bytes = [0u8; 72];
        let mut i = 0;
        let mut ended = false;
        while i < UNITS {
            if self.units[i] == 0 {
                ended = true;
            }
            if !ended {
                let [lo, hi] = self.units[i].to_le_bytes();
                bytes[2 * i] = lo;
                bytes[2 * i + 1] = hi;
            }
            i += 1;
        }
        bytes
    }

    /// The code units before the first NUL.
    pub fn units(&self) -> &[u16] {
        let len = self.units.iter().position(|&u| u == 0).unwrap_or(UNITS);
        &self.units[..len]
    }

    /// Whether the name has no code units.
    pub const fn is_empty(&self) -> bool {
        self.units[0] == 0
    }

    /// The characters of the name, with U+FFFD for each unpaired surrogate.
    pub fn chars(&self) -> impl Iterator<Item = char> + '_ {
        char::decode_utf16(self.units().iter().copied())
            .map(|c| c.unwrap_or(char::REPLACEMENT_CHARACTER))
    }

    /// Writes the name as UTF-8 into `buf` and returns it.
    ///
    /// Fails with [`ErrorKind::Corrupt`] when the name is not valid UTF-16
    /// and with [`ErrorKind::LimitExceeded`] when `buf` is too small; 108
    /// bytes always suffice.
    pub fn to_str<'a>(&self, buf: &'a mut [u8]) -> Result<&'a str, TableError> {
        let mut len = 0;
        for c in char::decode_utf16(self.units().iter().copied()) {
            let c = c.map_err(|_| TableError::new(ErrorKind::Corrupt, Detail::Name))?;
            let end = len + c.len_utf8();
            let slot = buf
                .get_mut(len..end)
                .ok_or(TableError::new(ErrorKind::LimitExceeded, Detail::Name))?;
            c.encode_utf8(slot);
            len = end;
        }
        core::str::from_utf8(&buf[..len])
            .map_err(|_| TableError::new(ErrorKind::Corrupt, Detail::Name))
    }
}

impl Default for PartitionName {
    fn default() -> Self {
        Self::EMPTY
    }
}

impl fmt::Display for PartitionName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        use fmt::Write;
        self.chars().try_for_each(|c| f.write_char(c))
    }
}

impl fmt::Debug for PartitionName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("PartitionName(\"")?;
        for c in self.chars() {
            fmt::Display::fmt(&c.escape_debug(), f)?;
        }
        f.write_str("\")")
    }
}

#[cfg(test)]
mod tests {
    extern crate alloc;

    use super::*;
    use alloc::string::ToString;

    #[test]
    fn encodes_and_decodes_beyond_ascii() {
        let name = PartitionName::new("EFI \u{1F600} ß").unwrap();
        assert_eq!(name.units().len(), 8);
        let bytes = name.to_le_bytes();
        assert_eq!(&bytes[..4], &[b'E', 0, b'F', 0]);
        assert!(bytes[16..].iter().all(|&b| b == 0));
        let back = PartitionName::from_le_bytes(bytes);
        assert_eq!(back, name);
        assert_eq!(back.to_string(), "EFI \u{1F600} ß");
        let mut buf = [0u8; 108];
        assert_eq!(back.to_str(&mut buf).unwrap(), "EFI \u{1F600} ß");
    }

    #[test]
    fn rejects_long_and_nul_names() {
        let fits = "a".repeat(36);
        assert!(PartitionName::new(&fits).is_ok());
        let long = "a".repeat(37);
        assert_eq!(
            PartitionName::new(&long).unwrap_err().kind(),
            ErrorKind::NameTooLong
        );
        let astral = "\u{1F600}".repeat(18);
        assert!(PartitionName::new(&astral).is_ok());
        let astral = alloc::format!("{astral}a");
        assert_eq!(
            PartitionName::new(&astral).unwrap_err().kind(),
            ErrorKind::NameTooLong
        );
        assert_eq!(
            PartitionName::new("a\0b").unwrap_err().kind(),
            ErrorKind::InvalidInput
        );
    }

    #[test]
    fn unpaired_surrogates_decode_lossily_and_never_as_str() {
        let mut bytes = [0u8; 72];
        bytes[..8].copy_from_slice(&[b'A', 0, 0x00, 0xD8, b'B', 0, 0x00, 0xDC]);
        let name = PartitionName::from_le_bytes(bytes);
        assert_eq!(name.units(), &[0x41, 0xD800, 0x42, 0xDC00]);
        assert_eq!(name.to_string(), "A\u{FFFD}B\u{FFFD}");
        let mut buf = [0u8; 108];
        assert_eq!(
            name.to_str(&mut buf).unwrap_err().kind(),
            ErrorKind::Corrupt
        );
        let full = PartitionName::from_le_bytes([0xFF; 72]);
        assert_eq!(full.units().len(), 36);
        assert_eq!(full.chars().count(), 36);
        let mut small = [0u8; 4];
        let ascii = PartitionName::new("hello").unwrap();
        assert_eq!(
            ascii.to_str(&mut small).unwrap_err().kind(),
            ErrorKind::LimitExceeded
        );
    }

    #[test]
    fn from_units_stops_at_nul() {
        let name = PartitionName::from_units(&[0x41, 0, 0x42]).unwrap();
        assert_eq!(name.units(), &[0x41]);
        assert!(PartitionName::from_units(&[0x41; 37]).is_err());
        assert!(PartitionName::from_units(&[]).unwrap().is_empty());
    }
}
