#![cfg_attr(not(feature = "std"), no_std)]

#[cfg(feature = "alloc")]
extern crate alloc;

pub type UtcTime = chrono::DateTime<chrono::Utc>;

pub mod internal;
pub mod str;
pub mod file;
use file::FileAttributes;
pub use file::{File, OpenOptions};

pub trait FileSystem {
    fn create(&mut self, path: &str, attributes: FileAttributes) -> Result<File, ()>;
    fn open(&mut self, path: &str, options: OpenOptions) -> Result<File, ()>;
}

