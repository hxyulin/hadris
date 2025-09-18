//! Hadris ISO
//! Terminology and spec are followed by the specifications described in
//! the [non official ISO9660 specification included](https://github.com/hxyulin/hadris/tree/main/crates/hadris-iso/spec)

// Known Bugs:
//  - Zero size files causes a lot of issues
//
//  TODO: There is a lot of bugs with mixing file interchanges!!!

#![no_std]

pub mod directory;
pub mod path;
pub mod types;
pub mod volume;

pub mod boot;
pub mod file;
pub mod read;

#[cfg(feature = "write")]
pub mod write;

#[cfg(feature = "alloc")]
extern crate alloc;

#[cfg(feature = "std")]
extern crate std;

pub mod joliet;
pub mod susp;
pub mod rrip;

use core::fmt;

pub use hadris_io as io;
use hadris_io::{Read, Seek, SeekFrom};
use spin::{Mutex, MutexGuard};

/// A Logical Sector, size has to be 2^n and > 2048
#[derive(Default, Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct LogicalSector(usize);

/// A Logical Sector, size has to be 2^n and > 512
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct LogicalBlock(usize);

struct LockedCursor<DATA: Seek> {
    data: Mutex<DATA>,
    sector_size: usize,
}

impl<DATA: Seek> LockedCursor<DATA> {
    pub fn seek_sector(&self, sector: LogicalSector) -> io::Result<u64> {
        self.data.lock().seek(SeekFrom::Start(
            (sector.0 as u64) * (self.sector_size as u64),
        ))
    }
}

impl<DATA: Read + Seek> Read for LockedCursor<DATA> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.data.lock().read(buf)
    }

    fn read_exact(&mut self, buf: &mut [u8]) -> io::Result<()> {
        self.data.lock().read_exact(buf)
    }
}

impl<DATA: Seek> Seek for LockedCursor<DATA> {
    fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
        self.data.lock().seek(pos)
    }

    fn stream_position(&mut self) -> io::Result<u64> {
        self.data.lock().stream_position()
    }

    fn seek_relative(&mut self, offset: i64) -> io::Result<()> {
        self.data.lock().seek_relative(offset)
    }
}

impl<DATA: Seek> LockedCursor<DATA> {
    pub fn pad_align_sector(&self) -> io::Result<LogicalSector> {
        let mut data = self.data.lock();
        let stream_pos = data.stream_position()?;
        let sector_size = self.sector_size as u64;
        let aligned_pos = (stream_pos & !(sector_size - 1)) + sector_size;
        if aligned_pos != stream_pos {
            data.seek_relative((aligned_pos - sector_size) as i64)?;
        }
        Ok(LogicalSector((aligned_pos / sector_size) as usize))
    }

    pub fn lock(&self) -> MutexGuard<'_, DATA> {
        self.data.lock()
    }
}

impl<DATA: Seek> fmt::Debug for LockedCursor<DATA> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("LockedCursor").finish()
    }
}
