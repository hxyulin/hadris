//! Hadris IO
//!
//! This provides the std::io implementations for no-std environments.
//! For use with std, the standard library types are re-exported.

#![no_std]

extern crate alloc;

#[cfg(feature = "std")]
extern crate std;
#[cfg(feature = "std")]
pub use std::io::{Error, ErrorKind, Read, Result, Seek, SeekFrom, Write};
#[cfg(feature = "std")]
pub use std::path::{Path, PathBuf};

#[cfg(not(feature = "std"))]
mod error;
#[cfg(not(feature = "std"))]
pub use error::Error;

#[macro_export]
macro_rules! try_io_result_option {
    ($expr:expr) => {
        match $expr {
            Ok(val) => val,
            Err(err) => return Some(Err(err)),
        }
    };
}

pub trait Parsable: Sized {
    fn parse<R: Read>(reader: &mut R) -> Result<Self>;
}

pub trait Writable: Sized {
    fn write<R: Write>(&self, writer: &mut R) -> Result<()>;
}

pub trait ReadExt {
    fn read_struct<T: bytemuck::Zeroable + bytemuck::NoUninit + bytemuck::AnyBitPattern>(
        &mut self,
    ) -> Result<T>;

    fn parse<T: Parsable>(&mut self) -> Result<T>;
}

impl<T: Read> ReadExt for T {
    fn read_struct<S: bytemuck::Zeroable + bytemuck::NoUninit + bytemuck::AnyBitPattern>(
        &mut self,
    ) -> Result<S> {
        let mut temp = S::zeroed();
        self.read_exact(bytemuck::bytes_of_mut(&mut temp))?;
        Ok(temp)
    }

    fn parse<S: Parsable>(&mut self) -> Result<S> {
        S::parse(self)
    }
}
