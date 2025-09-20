use core::{
    fmt,
    ops::{Index, IndexMut},
};

use hadris_common::types::file::FixedFilename;

/// A type representing a short filename
#[repr(transparent)]
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct ShortFileName(FixedFilename<12>);

impl fmt::Debug for ShortFileName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("ShortFileName")
            .field(&self.as_str())
            .finish()
    }
}

#[derive(Debug, thiserror::Error)]
#[error("disallowed characters in short file name")]
pub struct CreateShortFileNameError;

impl ShortFileName {
    pub const ALLOWED_SYMBOLS: &'static [u8] = b"$%'-_@~`!(){}^#&";

    pub fn new(bytes: [u8; 11]) -> Result<Self, CreateShortFileNameError> {
        for byte in &bytes {
            if byte.is_ascii_uppercase()
                || Self::ALLOWED_SYMBOLS.contains(byte)
                || byte.is_ascii_digit()
                || *byte == b' '
                || *byte > 127
            {
                continue;
            }
            return Err(CreateShortFileNameError);
        }

        let mut name = FixedFilename::empty();
        name.push_slice(&bytes[0..8]);
        name.push_byte(b'.');
        name.push_slice(&bytes[8..11]);
        Ok(Self(name))
    }

    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }
}

/// A Long File Name
#[repr(C)]
#[derive(Clone, Copy, PartialEq, Eq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct LongFileName {
    bytes: [u8; 255],
    len: u8,
}
