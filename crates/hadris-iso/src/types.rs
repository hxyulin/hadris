use core::marker::PhantomData;
pub use hadris_common::types::{endian::*, number::*};
use std::time::SystemTime;

use alloc::{format, string::ToString, vec, vec::Vec};

pub trait Charset: Copy {
    fn is_valid<'a>(bytes: impl Iterator<Item = &'a u8>) -> bool;
}

/// The `a-characters` character set.
/// This supports `a-z`, `A-Z`, `0-9` and `!"%$'()*+,-./:;<=>?`.
#[derive(Copy, Clone, PartialEq, Eq)]
pub struct CharsetA;

#[derive(Copy, Clone, PartialEq, Eq)]
pub struct CharsetD;

impl CharsetA {
    const VALID_SYMBOLS: &[u8] = b"!\"%$'()*+,-./:;<=>?";
}

impl CharsetD {
    const SPECIAL_CHARS: &[u8] = b"0123456789_";
}

impl Charset for CharsetA {
    fn is_valid<'a>(mut bytes: impl Iterator<Item = &'a u8>) -> bool {
        bytes.all(|b| b.is_ascii_alphanumeric() || Self::VALID_SYMBOLS.contains(b))
    }
}

impl Charset for CharsetD {
    fn is_valid<'a>(mut bytes: impl Iterator<Item = &'a u8>) -> bool {
        bytes.all(|b| b.is_ascii_uppercase() || Self::SPECIAL_CHARS.contains(b))
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

        if !C::is_valid(s.as_bytes().iter()) {
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

impl<C: Charset> From<Vec<u8>> for IsoString<C> {
    fn from(value: Vec<u8>) -> Self {
        // TODO: We should probably check, but it can also be \x00, or \x01
        //assert!(C::is_valid(value.iter()));
        Self {
            chars: value,
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
            chars: bytes.to_vec(),
            _marker: PhantomData,
        }
    }

    pub fn from_utf8(str: &str) -> Self {
        Self {
            chars: str.as_bytes().to_vec(),
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
pub type IsoStrD<const N: usize> = IsoStr<CharsetA, N>;
pub type IsoStringA = IsoString<CharsetA>;
pub type IsoStringD = IsoString<CharsetD>;

pub trait StdNum: Copy {
    type LsbType: bytemuck::Pod + bytemuck::Zeroable + Endian<Output = Self>;
    type MsbType: bytemuck::Pod + bytemuck::Zeroable + Endian<Output = Self>;
}

impl StdNum for u16 {
    type LsbType = U16<LittleEndian>;
    type MsbType = U16<BigEndian>;
}

impl StdNum for u32 {
    type LsbType = U32<LittleEndian>;
    type MsbType = U32<BigEndian>;
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct LsbMsb<T: StdNum> {
    lsb: T::LsbType,
    msb: T::MsbType,
}

impl<T> core::fmt::Debug for LsbMsb<T>
where
    T: StdNum + core::fmt::Debug,
{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        core::fmt::Debug::fmt(&self.read(), f)
    }
}

unsafe impl<T: StdNum> bytemuck::Zeroable for LsbMsb<T> {}
unsafe impl<T: StdNum + Copy + 'static> bytemuck::Pod for LsbMsb<T> {}

impl<T: StdNum> LsbMsb<T> {
    pub fn new(value: T) -> Self {
        Self {
            lsb: Endian::new(value),
            msb: Endian::new(value),
        }
    }

    pub fn read(&self) -> T {
        #[cfg(target_endian = "little")]
        {
            self.lsb.get()
        }
        #[cfg(target_endian = "big")]
        {
            self.msb.get()
        }
    }

    pub fn write(&mut self, value: T) {
        self.lsb.set(value);
        self.msb.set(value);
    }
}

pub type U16LsbMsb = LsbMsb<u16>;
pub type U32LsbMsb = LsbMsb<u32>;

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
    #[cfg(feature = "std")]
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
