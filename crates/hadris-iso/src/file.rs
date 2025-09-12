use core::ops::Range;

use crate::types::{Charset, CharsetD};

pub enum Filename {
    L1(FilenameL1),
}

impl Filename {
    pub fn as_str(&self) -> &str {
        match self {
            Self::L1(file) => file.as_str(),
        }
    }

    pub fn as_bytes(&self) -> &[u8] {
        match self {
            Self::L1(file) => file.as_bytes(),
        }
    }
}

#[derive(Clone)]
pub struct FilenameL1 {
    // The full data store:
    // 8.3 format: 8 + 3 + 1
    // ;1 version 12 + 2 = 14
    data: [u8; 14],
    len: usize,
}

impl FilenameL1 {
    pub const fn empty() -> Self {
        Self {
            data: [0; 14],
            len: 0,
        }
    }

    pub fn as_str(&self) -> &str {
        unsafe { core::str::from_utf8_unchecked(self.as_bytes()) }
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.data[0..self.len]
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn push_slice(&mut self, slice: &[u8]) -> Range<usize> {
        assert!(self.len + slice.len() <= self.data.len());
        let start = self.len;
        self.len += slice.len();
        self.data[start..self.len].copy_from_slice(slice);
        start..self.len
    }

    pub fn push_byte(&mut self, b: u8) -> usize {
        self.data[self.len] = b;
        self.len += 1;
        self.len - 1
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FilenameLevel {
    /// L1 Filenames
    /// Supports only uppercase and useing the 8.3 format
    Level1,
    /// L2 Filenames
    /// Supports up to 30 characters
    Level2,
    /// L3 Filenames
    /// Supports up to 207 characters
    Level3,
}

fn convert_l1(name: &str) -> FilenameL1 {
    let mut l1 = FilenameL1::empty();
    let name_bytes = name.as_bytes();
    match name.find('.') {
        Some(index) => {
            // We copy the basename, at most 8 bytes
            let basename = l1.push_slice(&name_bytes[0..index.min(8)]);
            CharsetD::substitute_invalid(l1.data[basename].iter_mut());
            let ext_len = (name.len() - index).min(3);
            l1.push_byte(b'.');
            let ext = l1.push_slice(&name_bytes[index + 1..index + 1 + ext_len]);
            CharsetD::substitute_invalid(l1.data[ext].iter_mut());
        }
        None => {
            let len = name.len().min(8);
            let basename = l1.push_slice(&name_bytes[0..len]);
            CharsetD::substitute_invalid(l1.data[basename].iter_mut());
        }
    }
    l1.push_slice(b";1");
    l1
}

impl FilenameLevel {
    pub fn convert(self, name: &str) -> Filename {
        match self {
            Self::Level1 => Filename::L1(convert_l1(name)),
            Self::Level2 => todo!(),
            Self::Level3 => todo!(),
        }
    }
}

#[cfg(all(test, feature = "std"))]
mod tests {
    use super::*;

    #[test]
    fn test_convert_l1() {
        let orig = "this-is-the-original-file.@very-long-ext";
        let converted = convert_l1(orig);
        assert_eq!(converted.as_str(), "THIS_IS_._VE;1");
    }
}
