//! NTFS directory traversal — listing entries, finding files by name.

io_transform! {

use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

use crate::attr::{
    apply_fixups, is_i30_name, parse_index_entries, AttrBody, AttrIter, DataRun,
    DataRunDecoder, IndexEntryInfo, ATTR_INDEX_ALLOCATION, ATTR_INDEX_ROOT,
};
use crate::error::{NtfsError, Result};
use super::fs::NtfsFs;
use super::io::{Read, Seek, read_data_runs};
use super::read::FileReader;

/// Handle to an NTFS directory.
///
/// Created via [`NtfsFs::root_dir`] or [`NtfsDir::open_dir`].
pub struct NtfsDir<'a, DATA: Read + Seek> {
    pub(crate) fs: &'a NtfsFs<DATA>,
    pub(crate) mft_index: u64,
}

/// A single entry from a directory listing.
#[derive(Debug, Clone)]
pub struct NtfsEntry {
    name: String,
    mft_index: u64,
    mft_seq: u16,
    is_directory: bool,
    data_size: u64,
    namespace: u8,
}

impl NtfsEntry {
    /// The filename (UTF-8).
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Whether this entry is a directory.
    pub fn is_directory(&self) -> bool {
        self.is_directory
    }

    /// Whether this entry is a regular file.
    pub fn is_file(&self) -> bool {
        !self.is_directory
    }

    /// File size in bytes (from the `$FILE_NAME` attribute copy in the index;
    /// may be stale for files that are actively written to).
    pub fn size(&self) -> u64 {
        self.data_size
    }

    /// MFT record number for this entry.
    pub fn mft_index(&self) -> u64 {
        self.mft_index
    }

    /// MFT sequence number (for stale-reference detection).
    pub fn mft_seq(&self) -> u16 {
        self.mft_seq
    }

    /// File name namespace.
    pub fn namespace(&self) -> u8 {
        self.namespace
    }
}

// -------------------------------------------------------------------------

impl<'a, DATA: Read + Seek> NtfsDir<'a, DATA> {
    /// List all entries in this directory.
    ///
    /// Reads the `$INDEX_ROOT` and, if present, all `$INDEX_ALLOCATION`
    /// records for the `$I30` filename index.  DOS-only names are
    /// automatically filtered out to avoid duplicates.
    pub async fn entries(&self) -> Result<Vec<NtfsEntry>> {
        let record = self.fs.read_mft_record(self.mft_index).await?;

        let mut entries = Vec::new();
        let mut alloc_info: Option<(Vec<DataRun>, u64)> = None;
        let mut index_record_size = self.fs.index_record_size;

        let mut attrs = AttrIter::new(&record)?;
        while let Some(a) = attrs.next() {
            match a.attr_type {
                ATTR_INDEX_ROOT if is_i30_name(a.name) => {
                    if let AttrBody::Resident(value) = a.body {
                        // Grab the per-index record size from the header
                        if value.len() >= 12 {
                            let irs = u32::from_le_bytes([
                                value[8], value[9], value[10], value[11],
                            ]) as usize;
                            if irs > 0 {
                                index_record_size = irs;
                            }
                        }

                        // Node header starts at offset 0x10 within the value
                        let raw = parse_index_entries(value, 0x10)?;
                        append_entries(&raw, &mut entries);
                    }
                }
                ATTR_INDEX_ALLOCATION if is_i30_name(a.name) => {
                    if let AttrBody::NonResident {
                        data_runs,
                        allocated_size,
                        ..
                    } = a.body
                    {
                        let runs: Vec<DataRun> = DataRunDecoder::new(data_runs).collect();
                        alloc_info = Some((runs, allocated_size));
                    }
                }
                _ => {}
            }
        }

        // Read INDEX_ALLOCATION blocks (if the directory is large enough
        // to spill out of INDEX_ROOT).
        if let Some((runs, alloc_size)) = alloc_info {
            let num_blocks = alloc_size as usize / index_record_size;
            for i in 0..num_blocks {
                let offset = (i * index_record_size) as u64;
                let mut block = vec![0u8; index_record_size];

                {
                    let mut data = self.fs.data.lock();
                    if read_data_runs(
                        &mut *data,
                        &runs,
                        offset,
                        &mut block,
                        self.fs.cluster_size as u64,
                    )
                    .await
                    .is_err()
                    {
                        continue;
                    }
                }

                // Validate INDX magic — freed blocks won't match.
                if block.len() < 4 || &block[0..4] != b"INDX" {
                    continue;
                }

                if apply_fixups(&mut block).is_err() {
                    continue;
                }

                // Node header is at offset 0x18 within the INDX record.
                if let Ok(raw) = parse_index_entries(&block, 0x18) {
                    append_entries(&raw, &mut entries);
                }
            }
        }

        Ok(entries)
    }

    /// Find an entry by name (case-insensitive for Win32/DOS names).
    pub async fn find(&self, name: &str) -> Result<Option<NtfsEntry>> {
        let entries = self.entries().await?;
        Ok(entries.into_iter().find(|e| {
            e.name().eq_ignore_ascii_case(name)
        }))
    }

    /// Open a subdirectory by name.
    pub async fn open_dir(&self, name: &str) -> Result<NtfsDir<'a, DATA>> {
        let entry = self.find(name).await?.ok_or(NtfsError::EntryNotFound)?;
        if !entry.is_directory() {
            return Err(NtfsError::NotADirectory);
        }
        Ok(NtfsDir {
            fs: self.fs,
            mft_index: entry.mft_index,
        })
    }

    /// Open a file for reading by name.
    pub async fn open_file(&self, name: &str) -> Result<FileReader<'a, DATA>> {
        let entry = self.find(name).await?.ok_or(NtfsError::EntryNotFound)?;
        FileReader::open(self.fs, &entry).await
    }
}

/// Directory flag from $FILE_NAME flags field.
const FILE_ATTR_DIRECTORY: u32 = 0x1000_0000;

fn append_entries(raw: &[IndexEntryInfo], out: &mut Vec<NtfsEntry>) {
    for info in raw {
        out.push(NtfsEntry {
            name: info.file_name.name.clone(),
            mft_index: info.mft_index,
            mft_seq: info.mft_seq,
            is_directory: info.file_name.flags & FILE_ATTR_DIRECTORY != 0,
            data_size: info.file_name.data_size,
            namespace: info.file_name.namespace,
        });
    }
}

} // end io_transform!
