//! File content reading for NTFS.

io_transform! {

use alloc::vec;
use alloc::vec::Vec;

use crate::attr::{decode_data_runs, AttrBody, AttrIter, DataRun, ATTR_DATA, ATTR_FLAG_COMPRESSED, ATTR_FLAG_ENCRYPTED};
use crate::error::{NtfsError, Result};
use super::dir::NtfsEntry;
use super::fs::NtfsFs;
use super::io::{Read, Seek, read_data_runs};

/// Backing storage for a file's data — either inline in the MFT record
/// (resident) or spread across clusters (non-resident).
enum FileData {
    Resident(Vec<u8>),
    NonResident {
        runs: Vec<DataRun>,
        initialized_size: u64,
    },
}

/// A reader for file content on an NTFS volume.
///
/// Created via [`NtfsFsReadExt::read_file`] or
/// [`NtfsDir::open_file`](super::dir::NtfsDir::open_file).
///
/// @hadris-spec NTFS:Data-Stream
/// @hadris-compliance partial
/// @hadris-tests read::read_large_nonresident_file
/// @hadris-note Reads resident, non-resident, sparse, and uninitialized unnamed data; compressed, encrypted, named, and attribute-list streams are unsupported.
pub struct FileReader<'a, DATA: Read + Seek> {
    fs: &'a NtfsFs<DATA>,
    data: FileData,
    data_size: u64,
    position: u64,
}

impl<'a, DATA: Read + Seek> FileReader<'a, DATA> {
    /// Open a file for reading from an [`NtfsEntry`].
    ///
    /// Loads the entry's MFT record and locates the unnamed `$DATA`
    /// attribute.
    pub(crate) async fn open(fs: &'a NtfsFs<DATA>, entry: &NtfsEntry) -> Result<Self> {
        if entry.is_directory() {
            return Err(NtfsError::NotAFile);
        }
        Self::open_by_mft_ref(fs, entry.mft_index(), entry.mft_seq()).await
    }

    /// Open a file for reading given its MFT record number directly.
    pub(crate) async fn open_by_mft(fs: &'a NtfsFs<DATA>, mft_index: u64) -> Result<Self> {
        Self::open_by_mft_ref(fs, mft_index, 0).await
    }

    async fn open_by_mft_ref(
        fs: &'a NtfsFs<DATA>,
        mft_index: u64,
        expected_sequence: u16,
    ) -> Result<Self> {
        let record = fs
            .read_mft_record_ref(mft_index, expected_sequence)
            .await?;
        let attrs = AttrIter::new(&record)?;

        for a in attrs {
            let a = a?;
            if a.attr_type != ATTR_DATA || a.name.is_some() {
                continue;
            }

            if a.flags & ATTR_FLAG_COMPRESSED != 0 {
                return Err(NtfsError::UnsupportedCompression);
            }
            if a.flags & ATTR_FLAG_ENCRYPTED != 0 {
                return Err(NtfsError::UnsupportedEncryption);
            }

            return match a.body {
                AttrBody::Resident(value) => Ok(Self {
                    fs,
                    data: FileData::Resident(value.to_vec()),
                    data_size: value.len() as u64,
                    position: 0,
                }),
                AttrBody::NonResident {
                    data_runs,
                    data_size,
                    initialized_size,
                    ..
                } => {
                    if initialized_size > data_size {
                        return Err(NtfsError::InvalidAttribute);
                    }
                    let runs = decode_data_runs(data_runs)?;
                    Ok(Self {
                        fs,
                        data: FileData::NonResident {
                            runs,
                            initialized_size,
                        },
                        data_size,
                        position: 0,
                    })
                }
            };
        }

        Err(NtfsError::AttributeNotFound {
            attr_type: ATTR_DATA,
        })
    }

    /// Total size of the file in bytes.
    pub fn size(&self) -> u64 {
        self.data_size
    }

    /// Bytes remaining from the current read position.
    pub fn remaining(&self) -> u64 {
        self.data_size.saturating_sub(self.position)
    }

    /// Read up to `buf.len()` bytes from the current position.
    pub async fn read(&mut self, buf: &mut [u8]) -> Result<usize> {
        let remaining = self.remaining();
        if remaining == 0 {
            return Ok(0);
        }
        let to_read = (buf.len() as u64).min(remaining) as usize;
        let buf = &mut buf[..to_read];

        match &self.data {
            FileData::Resident(resident) => {
                let start = self.position as usize;
                buf.copy_from_slice(&resident[start..start + to_read]);
            }
            FileData::NonResident {
                runs,
                initialized_size,
            } => {
                let initialized_remaining = initialized_size.saturating_sub(self.position);
                let stored_len = (to_read as u64).min(initialized_remaining) as usize;
                if stored_len > 0 {
                    let mut data = self.fs.data.lock();
                    read_data_runs(
                        &mut *data,
                        runs,
                        self.position,
                        &mut buf[..stored_len],
                        self.fs.cluster_size as u64,
                    )
                    .await?;
                }
                buf[stored_len..].fill(0);
            }
        }

        self.position += to_read as u64;
        Ok(to_read)
    }

    /// Read the entire remaining file content into a `Vec<u8>`.
    pub async fn read_to_vec(&mut self) -> Result<Vec<u8>> {
        let remaining =
            usize::try_from(self.remaining()).map_err(|_| NtfsError::InvalidAttribute)?;
        let mut buf = vec![0u8; remaining];
        let n = self.read(&mut buf).await?;
        buf.truncate(n);
        Ok(buf)
    }
}

/// Extension trait for reading files through [`NtfsFs`].
pub trait NtfsFsReadExt<DATA: Read + Seek> {
    /// Create a reader for a file described by an [`NtfsEntry`].
    async fn read_file<'a>(&'a self, entry: &NtfsEntry) -> Result<FileReader<'a, DATA>>
    where
        DATA: 'a;
}

impl<DATA: Read + Seek> NtfsFsReadExt<DATA> for NtfsFs<DATA> {
    async fn read_file<'a>(&'a self, entry: &NtfsEntry) -> Result<FileReader<'a, DATA>>
    where
        DATA: 'a,
    {
        FileReader::open(self, entry).await
    }
}

} // end io_transform!
