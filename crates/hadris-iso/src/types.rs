use core::marker::PhantomData;
pub use hadris_common::types::{endian::*, number::*};
use std::time::SystemTime;

pub trait Charset: Copy + PartialEq + Eq {
    fn is_valid(chars: &[u8]) -> bool;
}

/// The `a-characters` character set.
/// This supports `a-z`, `A-Z`, `0-9` and `!"%$'()*+,-./:;<=>?`.
#[derive(Copy, Clone, PartialEq, Eq)]
pub struct CharsetA;
#[derive(Copy, Clone, PartialEq, Eq)]
pub struct CharsetD;
#[derive(Copy, Clone, PartialEq, Eq)]
/// The `file-name` character set, it is CharsetD with the following characters allowed:
pub struct CharsetFile;

impl Charset for CharsetA {
    fn is_valid(chars: &[u8]) -> bool {
        const VALID_SYMBOLS: &[u8] = b"!\"%$'()*+,-./:;<=>?";
        chars
            .iter()
            .all(|c| c.is_ascii_alphanumeric() || VALID_SYMBOLS.contains(c))
    }
}

impl Charset for CharsetD {
    fn is_valid(chars: &[u8]) -> bool {
        const SPECIAL_CHARS: &[u8] = b"0123456789_";
        chars
            .iter()
            .all(|c| c.is_ascii_uppercase() || SPECIAL_CHARS.contains(c))
    }
}

impl Charset for CharsetFile {
    fn is_valid(chars: &[u8]) -> bool {
        const SPECIAL_CHARS: &[u8] = b"./";
        chars
            .iter()
            .all(|c| c.is_ascii_alphanumeric() || SPECIAL_CHARS.contains(c))
    }
}

/// A space padded string with a fixed length.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct IsoStr<C: Charset, const N: usize> {
    chars: [u8; N],
    _marker: PhantomData<C>,
}

unsafe impl<C: Charset, const N: usize> bytemuck::Zeroable for IsoStr<C, N> {}
unsafe impl<C: Charset + 'static, const N: usize> bytemuck::Pod for IsoStr<C, N> {}

impl<C: Charset, const N: usize> IsoStr<C, N> {
    pub fn empty() -> Self {
        Self {
            chars: [b' '; N],
            _marker: core::marker::PhantomData,
        }
    }

    pub fn max_len() -> usize {
        N
    }

    pub fn len(&self) -> usize {
        self.chars.iter().position(|&c| c == b' ').unwrap_or(N)
    }

    pub const fn from_bytes_exact(bytes: [u8; N]) -> Self {
        Self {
            chars: bytes,
            _marker: core::marker::PhantomData,
        }
    }

    // TODO: Error type
    pub fn from_str(s: &str) -> Result<Self, ()> {
        let mut chars = [b' '; N];
        if s.len() > N {
            return Err(());
        }

        if !C::is_valid(s.as_bytes()) {
            return Err(());
        }

        for (i, c) in s.bytes().enumerate() {
            chars[i] = c;
        }
        Ok(Self {
            chars,
            _marker: core::marker::PhantomData,
        })
    }

    pub fn to_str(&self) -> &str {
        if self.chars.len() == 1 {
            match self.chars[0] {
                b'\x00' => return "\\x00",
                b'\x01' => return "\\x01",
                _ => {}
            }
        }
        // SAFETY: The string is constructed from valid ASCII characters.
        unsafe { core::str::from_utf8_unchecked(&self.chars[..self.len()]) }
    }
}

impl<C: Charset, const N: usize> core::fmt::Display for IsoStr<C, N> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{}", self.to_str())
    }
}

impl<C: Charset, const N: usize> core::fmt::Debug for IsoStr<C, N> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "\"{}\"", self.to_str())
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct IsoString<C: Charset> {
    chars: Vec<u8>,
    _marker: PhantomData<C>,
}

impl From<Vec<u8>> for IsoString<CharsetFile> {
    fn from(chars: Vec<u8>) -> Self {
        Self {
            chars,
            _marker: PhantomData,
        }
    }
}

impl<C: Charset> IsoString<C> {
    pub const fn empty() -> Self {
        Self {
            chars: Vec::new(),
            _marker: PhantomData,
        }
    }

    pub fn with_size(size: usize) -> Self {
        Self {
            // TODO: Does the spec want spaces or nulls?
            chars: vec![b' '; size],
            _marker: PhantomData,
        }
    }

    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            chars: Vec::with_capacity(capacity),
            _marker: PhantomData,
        }
    }

    pub fn from_bytes(bytes: &[u8]) -> Self {
        Self {
            chars: bytes.iter().map(|&c| c).collect(),
            _marker: PhantomData,
        }
    }

    pub fn len(&self) -> usize {
        self.chars
            .iter()
            .position(|&c| c == b' ')
            .unwrap_or(self.chars.len())
    }

    pub fn bytes(&self) -> &[u8] {
        &self.chars
    }

    pub fn as_str(&self) -> &str {
        if self.chars.len() == 1 {
            match self.chars[0] {
                b'\x00' => return "\\x00",
                b'\x01' => return "\\x01",
                _ => {}
            }
        }
        // SAFETY: The string is constructed from valid ASCII characters.
        unsafe { core::str::from_utf8_unchecked(&self.chars[..self.len()]) }
    }
}

impl<C: Charset> core::fmt::Display for IsoString<C> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

impl<C: Charset> core::fmt::Debug for IsoString<C> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "\"{}\"", self.as_str())
    }
}

pub type IsoStrA<const N: usize> = IsoStr<CharsetA, N>;
pub type IsoStrD<const N: usize> = IsoStr<CharsetD, N>;
pub type IsoStrFile<const N: usize> = IsoStr<CharsetFile, N>;

pub type IsoStringFile = IsoString<CharsetFile>;

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct LsbMsb<T: Endian> {
    lsb: T::LsbType,
    msb: T::MsbType,
}

unsafe impl<T: Endian> bytemuck::Zeroable for LsbMsb<T> {}
unsafe impl<T: Endian + Copy + 'static> bytemuck::Pod for LsbMsb<T> {}

impl<T: Endian> LsbMsb<T> {
    pub fn new(value: T::Output) -> Self {
        Self {
            lsb: Endian::new(value),
            msb: Endian::new(value),
        }
    }

    pub fn read(&self) -> T::Output {
        #[cfg(target_endian = "little")]
        {
            self.lsb.get()
        }
        #[cfg(target_endian = "big")]
        {
            self.msb.get()
        }
    }

    pub fn write(&mut self, value: T::Output) {
        self.lsb.set(value);
        self.msb.set(value);
    }
}

pub type U16LsbMsb = LsbMsb<U16<LittleEndian>>;
pub type U32LsbMsb = LsbMsb<U32<LittleEndian>>;
pub type U64LsbMsb = LsbMsb<U64<LittleEndian>>;

#[repr(C, packed)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct DecDateTime {
    pub year: IsoStrD<4>,
    pub month: IsoStrD<2>,
    pub day: IsoStrD<2>,
    pub hour: IsoStrD<2>,
    pub minute: IsoStrD<2>,
    pub second: IsoStrD<2>,
    pub hundredths: IsoStrD<2>,
    pub timezone: u8,
}

impl core::fmt::Debug for DecDateTime {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DecDateTime")
            .field(
                "date",
                &format!("{}-{}-{}", self.year, self.month, self.day),
            )
            .field(
                "time",
                &format!(
                    "{}:{}:{}.{:.3}",
                    self.hour, self.minute, self.second, self.hundredths
                ),
            )
            .field("timezone", &self.timezone)
            .finish_non_exhaustive()
    }
}

impl DecDateTime {
    pub fn now() -> Self {
        use chrono::{DateTime, Datelike, Timelike, Utc};
        let now: DateTime<Utc> = SystemTime::now().into();
        Self {
            year: IsoStrD::from_str(&now.year().to_string()).unwrap(),
            month: IsoStrD::from_str(&now.month().to_string()).unwrap(),
            day: IsoStrD::from_str(&now.day().to_string()).unwrap(),
            hour: IsoStrD::from_str(&now.hour().to_string()).unwrap(),
            minute: IsoStrD::from_str(&now.minute().to_string()).unwrap(),
            second: IsoStrD::from_str(&now.second().to_string()).unwrap(),
            hundredths: IsoStrD::from_str(&(now.nanosecond() / 10_000_000).to_string()).unwrap(),
            timezone: 0,
        }
    }
}

/// The file interchange level
///
/// For the ISO specification,
/// L1 is the 8.3 format with contiguous files,
/// L2 allows for 30 characters including the dot
/// L3 allows for 30 characters including the dot, but allows for multiple extents
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileInterchange {
    L1 = 1,
    L2 = 2,
    L3 = 3,
    // TODO: Add more unconformant variants, so that some can be 'semi-conformant'
    /// Non-conformant, allows anything less than 32 characters
    NonConformant = 255,
}

impl FileInterchange {
    pub fn from_str(&self, s: &str) -> Result<IsoStringFile, ()> {
        match self {
            FileInterchange::L1 => {
                let (base, ext) = s.split_once('.').unwrap_or((s, ""));
                // We dont want to truncate, because filenames are important (e.g. for booting)
                assert!(base.len() <= 8);
                assert!(ext.len() <= 3);
                // 1 for the dot, 2 for semicolon and version
                let mut bytes = Vec::with_capacity(base.len() + ext.len() + 3);
                bytes.extend_from_slice(base.as_bytes());
                bytes.push(b'.');
                bytes.extend_from_slice(ext.as_bytes());
                bytes.extend_from_slice(b";1");
                Ok(bytes.into())
            }
            FileInterchange::L2 | FileInterchange::L3 => {
                assert!(s.len() <= 30);
                let mut bytes = s.as_bytes().to_vec();
                bytes.extend_from_slice(b";1");
                Ok(bytes.into())
            }
            FileInterchange::NonConformant => {
                assert!(s.len() <= 32);
                Ok(IsoStringFile::from_bytes(&s.as_bytes()))
            }
        }
    }

    pub fn original(&self, s: &IsoStringFile) -> String {
        let mut chars = s.chars.iter();
        let mut out = String::new();
        while let Some(c) = chars.next() {
            if *c == b';' {
                // TODO: We need to check if the next character is a digit, otherwise it may be
                // invalid, and maybe we can use rfind instead
                break;
            }
            out.push(*c as char);
        }
        out
    }
}
