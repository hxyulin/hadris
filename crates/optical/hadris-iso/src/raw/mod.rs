//! On-disk layouts of ECMA-119 (ISO 9660), Joliet, El Torito, SUSP and
//! Rock Ridge.
//!
//! The items here mirror the specifications byte for byte. The module may
//! gain items; the existing ones follow the specifications and stay
//! exhaustive. The crate root never re-exports them. Tools such as
//! verifiers and dumpers read and write them directly; the rest of the
//! crate does not need them to walk a tree.

use core::fmt;

use hadris_fs::DateTime;

mod boot;
mod descriptor;
mod directory;
mod path_table;
mod susp;

pub use boot::{
    BOOTABLE, BootCatalogHeader, BootInfoTable, BootSectionEntry, BootSectionEntryExtension,
    BootValidationEntry, Grub2BootInfoTable, HEADER_FINAL, HEADER_MORE, NOT_BOOTABLE,
};
pub use descriptor::{
    BootRecordVolumeDescriptor, DescriptorType, EL_TORITO_ID, PrimaryVolumeDescriptor, STANDARD_ID,
    SupplementaryVolumeDescriptor, VolumeDescriptor, VolumeDescriptorHeader,
    VolumeDescriptorSetTerminator,
};
pub use directory::{DirectoryRecord, DirectoryRecordHeader, FileFlags, RootDirectoryRecord};
pub use path_table::PathTableHeader;
pub use susp::{
    ContinuationArea, NmFlags, PnEntry, PxEntry, RRIP_IDENTIFIERS, SlComponentFlags, SuspEntries,
    SuspEntry, TfFlags,
};

/// The size of a logical sector, and of every volume descriptor.
pub const SECTOR_SIZE: usize = 2048;

/// The logical sector of the first volume descriptor.
pub const DESCRIPTOR_START: u32 = 16;

/// The Joliet escape sequences for UCS-2 levels 1, 2 and 3, as stored at the
/// start of the 32-byte escape sequence field.
pub const JOLIET_ESCAPES: [[u8; 3]; 3] = [*b"%/@", *b"%/C", *b"%/E"];

macro_rules! endian_int {
    ($(#[$meta:meta])* $name:ident, $int:ty, $len:literal, $to:ident, $from:ident) => {
        $(#[$meta])*
        #[repr(transparent)]
        #[derive(Clone, Copy, PartialEq, Eq, Hash, Default, bytemuck::Pod, bytemuck::Zeroable)]
        pub struct $name([u8; $len]);

        impl $name {
            /// Encodes `value`.
            pub const fn new(value: $int) -> Self {
                Self(value.$to())
            }

            /// Decodes the value.
            pub const fn get(self) -> $int {
                <$int>::$from(self.0)
            }

            /// Replaces the value.
            pub fn set(&mut self, value: $int) {
                self.0 = value.$to();
            }
        }

        impl fmt::Debug for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                fmt::Debug::fmt(&self.get(), f)
            }
        }
    };
}

endian_int!(
    /// A little-endian 16-bit field (ECMA-119 7.2.1).
    U16Le, u16, 2, to_le_bytes, from_le_bytes
);
endian_int!(
    /// A big-endian 16-bit field (ECMA-119 7.2.2).
    U16Be, u16, 2, to_be_bytes, from_be_bytes
);
endian_int!(
    /// A little-endian 32-bit field (ECMA-119 7.3.1).
    U32Le, u32, 4, to_le_bytes, from_le_bytes
);
endian_int!(
    /// A big-endian 32-bit field (ECMA-119 7.3.2).
    U32Be, u32, 4, to_be_bytes, from_be_bytes
);

macro_rules! both_int {
    ($(#[$meta:meta])* $name:ident, $int:ty, $half:literal) => {
        $(#[$meta])*
        #[repr(C)]
        #[derive(Clone, Copy, PartialEq, Eq, Hash, Default, bytemuck::Pod, bytemuck::Zeroable)]
        pub struct $name {
            le: [u8; $half],
            be: [u8; $half],
        }

        impl $name {
            /// Encodes `value` in both byte orders.
            pub const fn new(value: $int) -> Self {
                Self {
                    le: value.to_le_bytes(),
                    be: value.to_be_bytes(),
                }
            }

            /// Decodes the little-endian copy.
            pub const fn get(self) -> $int {
                <$int>::from_le_bytes(self.le)
            }

            /// Decodes the big-endian copy.
            pub const fn get_be(self) -> $int {
                <$int>::from_be_bytes(self.be)
            }

            /// Whether both copies hold the same value, as the
            /// specification requires.
            pub const fn is_consistent(self) -> bool {
                self.get() == self.get_be()
            }

            /// Replaces the value in both byte orders.
            pub fn set(&mut self, value: $int) {
                *self = Self::new(value);
            }
        }

        impl fmt::Debug for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                if self.is_consistent() {
                    fmt::Debug::fmt(&self.get(), f)
                } else {
                    write!(f, "{}/{}", self.get(), self.get_be())
                }
            }
        }
    };
}

both_int!(
    /// A 16-bit field stored in both byte orders (ECMA-119 7.2.3).
    U16Both, u16, 2
);
both_int!(
    /// A 32-bit field stored in both byte orders (ECMA-119 7.3.3).
    U32Both, u32, 4
);

/// A fixed-size identifier padded with spaces, as in the volume descriptors.
///
/// The bytes are whatever the image holds: a- or d-characters in the primary
/// descriptor, UCS-2 in a Joliet descriptor, or anything a producer chose.
#[repr(transparent)]
#[derive(Clone, Copy, PartialEq, Eq, Hash, bytemuck::Pod, bytemuck::Zeroable)]
pub struct IsoStr<const N: usize>([u8; N]);

impl<const N: usize> IsoStr<N> {
    /// All spaces.
    pub const fn empty() -> Self {
        Self([b' '; N])
    }

    /// The field holding exactly `bytes`.
    pub const fn from_bytes(bytes: [u8; N]) -> Self {
        Self(bytes)
    }

    /// `text` padded with spaces, or `None` when it is longer than `N`
    /// bytes. The text is stored as given.
    pub fn padded(text: &[u8]) -> Option<Self> {
        let mut bytes = [b' '; N];
        bytes.get_mut(..text.len())?.copy_from_slice(text);
        Some(Self(bytes))
    }

    /// The raw field.
    pub const fn as_bytes(&self) -> &[u8; N] {
        &self.0
    }

    /// The field without trailing spaces and NUL bytes.
    pub fn trimmed(&self) -> &[u8] {
        let end = self
            .0
            .iter()
            .rposition(|&byte| byte != b' ' && byte != 0)
            .map_or(0, |pos| pos + 1);
        &self.0[..end]
    }

    /// The trimmed field as UTF-8. Disk bytes need not be UTF-8, so this can
    /// fail.
    pub fn as_str(&self) -> Result<&str, core::str::Utf8Error> {
        core::str::from_utf8(self.trimmed())
    }
}

impl<const N: usize> Default for IsoStr<N> {
    fn default() -> Self {
        Self::empty()
    }
}

impl<const N: usize> fmt::Debug for IsoStr<N> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.as_str() {
            Ok(text) => fmt::Debug::fmt(text, f),
            Err(_) => fmt::Debug::fmt(self.trimmed(), f),
        }
    }
}

fn digits<const N: usize>(mut value: u32) -> [u8; N] {
    let mut out = [b'0'; N];
    for byte in out.iter_mut().rev() {
        *byte = b'0' + (value % 10) as u8;
        value /= 10;
    }
    out
}

fn parse_digits(bytes: &[u8]) -> Option<u32> {
    bytes.iter().try_fold(0u32, |acc, &byte| {
        byte.is_ascii_digit()
            .then(|| acc * 10 + u32::from(byte - b'0'))
    })
}

/// A date and time written as digits, used in volume descriptors
/// (ECMA-119 8.4.26.1).
///
/// All-zero digits with a zero offset mean "not specified".
#[repr(C)]
#[derive(Clone, Copy, PartialEq, Eq, Hash, bytemuck::Pod, bytemuck::Zeroable)]
pub struct DecDateTime {
    /// Year, 1 to 9999.
    pub year: [u8; 4],
    /// Month, 1 to 12.
    pub month: [u8; 2],
    /// Day of the month, 1 to 31.
    pub day: [u8; 2],
    /// Hour, 0 to 23.
    pub hour: [u8; 2],
    /// Minute, 0 to 59.
    pub minute: [u8; 2],
    /// Second, 0 to 59.
    pub second: [u8; 2],
    /// Hundredths of a second.
    pub hundredths: [u8; 2],
    /// Offset from UTC in 15-minute intervals, -48 to 52.
    pub offset: i8,
}

impl DecDateTime {
    /// "Not specified": zero digits and offset.
    pub const UNSPECIFIED: Self = Self {
        year: *b"0000",
        month: *b"00",
        day: *b"00",
        hour: *b"00",
        minute: *b"00",
        second: *b"00",
        hundredths: *b"00",
        offset: 0,
    };

    /// `time` in UTC.
    pub fn from_datetime(time: DateTime) -> Self {
        let (date, clock) = time.to_civil_utc();
        let year = u32::try_from(date.year()).unwrap_or(0).min(9999);
        Self {
            year: digits(year),
            month: digits(u32::from(date.month())),
            day: digits(u32::from(date.day())),
            hour: digits(u32::from(clock.hour())),
            minute: digits(u32::from(clock.minute())),
            second: digits(u32::from(clock.second())),
            hundredths: digits(time.nanoseconds() / 10_000_000),
            offset: 0,
        }
    }

    /// The instant, or `None` when unspecified or invalid.
    pub fn to_datetime(&self) -> Option<DateTime> {
        let year = parse_digits(&self.year)?;
        if year == 0 {
            return None;
        }
        let date = hadris_fs::CivilDate::new(
            i32::try_from(year).ok()?,
            parse_digits(&self.month)? as u8,
            parse_digits(&self.day)? as u8,
        )
        .ok()?;
        let clock = hadris_fs::CivilTime::new(
            parse_digits(&self.hour)? as u8,
            parse_digits(&self.minute)? as u8,
            parse_digits(&self.second)? as u8,
        )
        .ok()?;
        let offset = i16::from(self.offset).checked_mul(15)?;
        let time = DateTime::from_civil(date, clock, Some(offset)).ok()?;
        time.with_nanoseconds(parse_digits(&self.hundredths)? * 10_000_000)
            .ok()
    }
}

impl Default for DecDateTime {
    fn default() -> Self {
        Self::UNSPECIFIED
    }
}

impl fmt::Debug for DecDateTime {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.to_datetime() {
            Some(time) => write!(f, "DecDateTime({}s)", time.unix_seconds()),
            None => f.write_str("DecDateTime(unspecified)"),
        }
    }
}

/// A date and time in seven binary bytes, used in directory records and
/// Rock Ridge `TF` entries (ECMA-119 9.1.5).
#[repr(C)]
#[derive(Clone, Copy, Default, PartialEq, Eq, Hash, bytemuck::Pod, bytemuck::Zeroable)]
pub struct DirDateTime {
    /// Years since 1900.
    pub year: u8,
    /// Month, 1 to 12.
    pub month: u8,
    /// Day of the month, 1 to 31.
    pub day: u8,
    /// Hour, 0 to 23.
    pub hour: u8,
    /// Minute, 0 to 59.
    pub minute: u8,
    /// Second, 0 to 59.
    pub second: u8,
    /// Offset from UTC in 15-minute intervals, -48 to 52.
    pub offset: i8,
}

impl DirDateTime {
    /// `time` in UTC. Years before 1900 or after 2155 are clamped.
    pub fn from_datetime(time: DateTime) -> Self {
        let (date, clock) = time.to_civil_utc();
        Self {
            year: (date.year() - 1900).clamp(0, 255) as u8,
            month: date.month(),
            day: date.day(),
            hour: clock.hour(),
            minute: clock.minute(),
            second: clock.second(),
            offset: 0,
        }
    }

    /// The instant, or `None` when all fields are zero or invalid.
    pub fn to_datetime(&self) -> Option<DateTime> {
        if *self == Self::default() {
            return None;
        }
        let date =
            hadris_fs::CivilDate::new(1900 + i32::from(self.year), self.month, self.day).ok()?;
        let clock = hadris_fs::CivilTime::new(self.hour, self.minute, self.second).ok()?;
        let offset = i16::from(self.offset).checked_mul(15)?;
        DateTime::from_civil(date, clock, Some(offset)).ok()
    }

    /// The seven bytes.
    pub fn to_bytes(self) -> [u8; 7] {
        bytemuck::cast(self)
    }
}

impl fmt::Debug for DirDateTime {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}-{:02}-{:02}T{:02}:{:02}:{:02}{:+}",
            1900 + u32::from(self.year),
            self.month,
            self.day,
            self.hour,
            self.minute,
            self.second,
            i32::from(self.offset) * 15
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn both_endian_fields_detect_mismatches() {
        let good = U32Both::new(0x1234_5678);
        assert!(good.is_consistent());
        let mut bytes: [u8; 8] = bytemuck::cast(good);
        bytes[7] ^= 1;
        let bad: U32Both = bytemuck::cast(bytes);
        assert!(!bad.is_consistent());
        assert_eq!(bad.get(), 0x1234_5678);
    }

    #[test]
    fn iso_str_as_str_rejects_non_utf8_bytes() {
        let mut bytes = [b' '; 16];
        bytes[0] = 0xFF;
        let text = IsoStr::<16>::from_bytes(bytes);
        assert!(text.as_str().is_err());
        assert_eq!(text.trimmed(), &[0xFF]);
        let _ = alloc_free_debug(&text);
        assert_eq!(IsoStr::<8>::padded(b"CDROM").unwrap().as_str(), Ok("CDROM"));
        assert!(IsoStr::<2>::padded(b"TOO LONG").is_none());
    }

    fn alloc_free_debug(value: &impl fmt::Debug) -> usize {
        struct Count(usize);
        impl fmt::Write for Count {
            fn write_str(&mut self, s: &str) -> fmt::Result {
                self.0 += s.len();
                Ok(())
            }
        }
        let mut count = Count(0);
        fmt::write(&mut count, format_args!("{value:?}")).unwrap();
        count.0
    }

    #[test]
    fn dates_round_trip() {
        let time = DateTime::from_unix_seconds(1_700_000_000).unwrap();
        assert_eq!(
            DecDateTime::from_datetime(time)
                .to_datetime()
                .map(|t| t.unix_seconds()),
            Some(1_700_000_000)
        );
        assert_eq!(
            DirDateTime::from_datetime(time)
                .to_datetime()
                .map(|t| t.unix_seconds()),
            Some(1_700_000_000)
        );
        assert!(DecDateTime::UNSPECIFIED.to_datetime().is_none());
        assert!(DirDateTime::default().to_datetime().is_none());
    }
}
