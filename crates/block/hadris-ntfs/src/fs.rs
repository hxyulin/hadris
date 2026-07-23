//! NTFS filesystem handle — opening volumes, reading MFT records.

io_transform! {

use alloc::vec;
use alloc::vec::Vec;

use spin::Mutex;

use hadris_common::types::endian::Endian;

use crate::attr::{
    self, apply_fixups, decode_data_runs, decode_record_size, AttrBody, AttrIter, DataRun,
    ATTR_DATA, MFT_RECORD_ROOT_DIR, MFT_RECORD_UPCASE,
};
use crate::error::{NtfsError, Result};
use crate::raw::RawNtfsBootSector;
use super::dir::NtfsDir;
use super::io::{Read, ReadExt, Seek, SeekFrom, read_data_runs};
use super::read::FileReader;

/// Handle for a mounted NTFS filesystem.
///
/// Wraps the underlying data source in a `Mutex` so it can be shared
/// between the filesystem handle and readers/iterators.
pub struct NtfsFs<DATA: Seek> {
    pub(crate) data: Mutex<DATA>,
    pub(crate) cluster_size: usize,
    #[allow(dead_code)]
    pub(crate) sector_size: usize,
    pub(crate) mft_record_size: usize,
    pub(crate) index_record_size: usize,
    /// Decoded data runs for the `$MFT` data stream.
    pub(crate) mft_runs: Vec<DataRun>,
    pub(crate) upcase: Vec<u16>,
    volume_serial: u64,
    total_sectors: u64,
}

impl<DATA: Read + Seek> NtfsFs<DATA> {
    /// Open an NTFS filesystem from a data source.
    ///
    /// Reads the boot sector, locates `$MFT`, and caches its data runs so
    /// that any MFT record can subsequently be loaded on demand.
    ///
    /// @hadris-spec NTFS:Master-File-Table
    /// @hadris-compliance partial
    /// @hadris-tests read::open_blank_volume
    /// @hadris-note Reads the base `$MFT` extent and validates file references; attribute-list extents and `$MFTMirr` recovery are not supported.
    pub async fn open(mut data: DATA) -> Result<Self> {
        let boot = data.read_struct::<RawNtfsBootSector>().await?;

        // Validate OEM ID
        if &boot.oem_id != b"NTFS    " {
            return Err(NtfsError::InvalidOemId);
        }

        // Validate end-of-sector signature
        let sig = boot.signature.get();
        if sig != 0xAA55 {
            return Err(NtfsError::InvalidBootSignature { found: sig });
        }

        let sector_size_raw = boot.bytes_per_sector.get();
        if !(256..=4096).contains(&sector_size_raw) || !sector_size_raw.is_power_of_two() {
            return Err(NtfsError::InvalidSectorSize {
                found: sector_size_raw,
            });
        }
        let sector_size = sector_size_raw as usize;

        let sectors_per_cluster_raw = boot.sectors_per_cluster;
        if sectors_per_cluster_raw == 0 || !sectors_per_cluster_raw.is_power_of_two() {
            return Err(NtfsError::InvalidSectorsPerCluster {
                found: sectors_per_cluster_raw,
            });
        }
        let sectors_per_cluster = sectors_per_cluster_raw as usize;
        let cluster_size = sector_size
            .checked_mul(sectors_per_cluster)
            .ok_or(NtfsError::InvalidVolumeGeometry)?;
        let mft_record_size = decode_record_size(boot.clusters_per_mft_record, cluster_size)?;
        let index_record_size = decode_record_size(boot.clusters_per_index_record, cluster_size)?;
        if mft_record_size < 8 || index_record_size < 8 {
            return Err(NtfsError::InvalidRecordSize);
        }

        let total_sectors = boot.total_sectors.get();
        let mft_lcn = boot.mft_lcn.get();
        let total_clusters = total_sectors / sectors_per_cluster as u64;
        if total_clusters == 0 || mft_lcn >= total_clusters {
            return Err(NtfsError::InvalidVolumeGeometry);
        }
        let mft_byte_offset = mft_lcn
            .checked_mul(cluster_size as u64)
            .ok_or(NtfsError::InvalidVolumeGeometry)?;

        // Read MFT record 0 ($MFT itself) directly from the known LCN.
        data.seek(SeekFrom::Start(mft_byte_offset)).await?;
        let mut record0 = vec![0u8; mft_record_size];
        data.read_exact(&mut record0).await?;

        if record0.len() < 4 || &record0[0..4] != b"FILE" {
            return Err(NtfsError::InvalidMftMagic);
        }
        apply_fixups(&mut record0, sector_size)?;

        // Find the unnamed $DATA attribute in record 0.
        let attrs = AttrIter::new(&record0)?;
        let mut mft_runs: Vec<DataRun> = Vec::new();

        for a in attrs {
            let a = a?;
            if a.attr_type == ATTR_DATA && a.name.is_none() {
                match a.body {
                    AttrBody::NonResident { data_runs, .. } => {
                        mft_runs = decode_data_runs(data_runs)?;
                    }
                    AttrBody::Resident(_) => {
                        // $MFT is always non-resident on valid volumes.
                        return Err(NtfsError::InvalidAttribute);
                    }
                }
                break;
            }
        }

        if mft_runs.is_empty() {
            return Err(NtfsError::AttributeNotFound {
                attr_type: ATTR_DATA,
            });
        }

        let mut fs = Self {
            data: Mutex::new(data),
            cluster_size,
            sector_size,
            mft_record_size,
            index_record_size,
            mft_runs,
            upcase: Vec::new(),
            volume_serial: boot.volume_serial.get(),
            total_sectors,
        };
        fs.load_upcase().await?;
        Ok(fs)
    }

    async fn load_upcase(&mut self) -> Result<()> {
        let bytes = {
            let mut reader = FileReader::open_by_mft(self, MFT_RECORD_UPCASE).await?;
            reader.read_to_vec().await?
        };
        if bytes.len() != (u16::MAX as usize + 1) * size_of::<u16>() {
            return Err(NtfsError::InvalidUpcaseTable);
        }

        self.upcase = bytes
            .chunks_exact(2)
            .map(|unit| u16::from_le_bytes([unit[0], unit[1]]))
            .collect();
        Ok(())
    }

    pub(crate) fn names_equal(&self, left: &str, right: &str) -> bool {
        left.encode_utf16()
            .map(|unit| self.upcase[unit as usize])
            .eq(right
                .encode_utf16()
                .map(|unit| self.upcase[unit as usize]))
    }

    /// Read an MFT record by its record number.
    ///
    /// The returned buffer has had fixups applied and is ready for
    /// attribute iteration via [`AttrIter`].
    pub async fn read_mft_record(&self, index: u64) -> Result<Vec<u8>> {
        self.read_mft_record_ref(index, 0).await
    }

    pub(crate) async fn read_mft_record_ref(
        &self,
        index: u64,
        expected_sequence: u16,
    ) -> Result<Vec<u8>> {
        let byte_offset = index
            .checked_mul(self.mft_record_size as u64)
            .ok_or(NtfsError::MftRecordOutOfBounds { index })?;
        let mut record = vec![0u8; self.mft_record_size];

        {
            let mut data = self.data.lock();
            read_data_runs(
                &mut *data,
                &self.mft_runs,
                byte_offset,
                &mut record,
                self.cluster_size as u64,
            )
            .await?;
        }

        if record.len() < 4 || &record[0..4] != b"FILE" {
            return Err(NtfsError::InvalidMftMagic);
        }
        apply_fixups(&mut record, self.sector_size)?;

        // Verify the record is in use
        if record.len() >= 0x18 {
            let sequence = u16::from_le_bytes([record[0x10], record[0x11]]);
            let flags = u16::from_le_bytes([record[0x16], record[0x17]]);
            if flags & attr::MFT_RECORD_IN_USE == 0 {
                return Err(NtfsError::MftRecordOutOfBounds { index });
            }
            if expected_sequence != 0 && sequence != expected_sequence {
                return Err(NtfsError::StaleFileReference {
                    index,
                    expected: expected_sequence,
                    found: sequence,
                });
            }
        }

        Ok(record)
    }

    /// Get a handle to the root directory (MFT record 5).
    pub fn root_dir(&self) -> NtfsDir<'_, DATA> {
        NtfsDir {
            fs: self,
            mft_index: MFT_RECORD_ROOT_DIR,
            mft_seq: 0,
        }
    }

    /// Volume serial number from the boot sector.
    pub fn volume_serial(&self) -> u64 {
        self.volume_serial
    }

    /// Total number of sectors in the volume.
    pub fn total_sectors(&self) -> u64 {
        self.total_sectors
    }

    /// Bytes per cluster.
    pub fn cluster_size(&self) -> usize {
        self.cluster_size
    }

    /// Bytes per MFT record.
    pub fn mft_record_size(&self) -> usize {
        self.mft_record_size
    }

    /// Open a file or directory by path (e.g., "/dir/subdir/file.txt").
    ///
    /// Paths can use forward or back slashes as separators.
    /// Leading separators are optional.
    pub async fn open_path(&self, path: &str) -> Result<super::dir::NtfsEntry> {
        let path = path.trim_start_matches(['/', '\\']);
        if path.is_empty() {
            return Err(NtfsError::InvalidPath);
        }

        let mut components = path
            .split(['/', '\\'])
            .filter(|s| !s.is_empty())
            .peekable();

        if components.peek().is_none() {
            return Err(NtfsError::InvalidPath);
        }

        let mut current_dir = self.root_dir();
        let mut last_component = None;

        for component in components {
            if let Some(prev) = last_component.take() {
                current_dir = current_dir.open_dir(prev).await?;
            }
            last_component = Some(component);
        }

        let final_name = last_component.unwrap();
        current_dir
            .find(final_name)
            .await?
            .ok_or(NtfsError::EntryNotFound)
    }
}

} // end io_transform!
