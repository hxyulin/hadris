//! Volume state read at mount time, shared by the modes.

use hadris_fs::{Capabilities, CaseSensitivity, NameCharset, NodeId};

use crate::error::Detail;
use crate::raw;
use crate::record::{self, Body, MAX_RECORD, NonResident};

/// The largest device block the reader handles.
pub(crate) const MAX_BLOCK: usize = 4096;
/// Mapping pairs of `$UpCase` kept in memory.
pub(crate) const UPCASE_RUNS: usize = 512;
/// Extents of `$MFT` kept in memory.
pub(crate) const MFT_SPANS: usize = 32;

/// Where `$MFT` is.
pub(crate) type MftExtents = Extents<MAX_RECORD, MFT_SPANS>;
/// Where `$UpCase` is.
pub(crate) type UpcaseExtents = Extents<UPCASE_RUNS, 4>;
/// Code units per page of the `$UpCase` cache.
pub(crate) const PAGE: usize = 256;

/// Where a stream's clusters are, kept after its records are gone: the
/// mapping pairs of each extent, from the base record and from extension
/// records named by an `$ATTRIBUTE_LIST`.
#[derive(Debug, Clone)]
pub(crate) struct Extents<const N: usize, const S: usize> {
    bytes: [u8; N],
    used: usize,
    spans: [(u64, usize, usize); S],
    count: usize,
    pub(crate) size: u64,
    pub(crate) initialized: u64,
}

impl<const N: usize, const S: usize> Extents<N, S> {
    /// A stream whose first extent is `attr`.
    pub(crate) fn new(attr: &NonResident<'_>) -> Option<Self> {
        let mut extents = Self {
            bytes: [0; N],
            used: 0,
            spans: [(0, 0, 0); S],
            count: 0,
            size: attr.data_size,
            initialized: attr.initialized_size.min(attr.data_size),
        };
        if attr.start_vcn != 0 {
            return None;
        }
        extents.push(attr)?;
        Some(extents)
    }

    pub(crate) const fn empty() -> Self {
        Self {
            bytes: [0; N],
            used: 0,
            spans: [(0, 0, 0); S],
            count: 0,
            size: 0,
            initialized: 0,
        }
    }

    /// Appends the next extent, which must start after the last one.
    pub(crate) fn push(&mut self, attr: &NonResident<'_>) -> Option<()> {
        if self.count > 0 && attr.start_vcn <= self.spans[self.count - 1].0 {
            return None;
        }
        let end = self.used.checked_add(attr.runs.len())?;
        self.bytes
            .get_mut(self.used..end)?
            .copy_from_slice(attr.runs);
        *self.spans.get_mut(self.count)? = (attr.start_vcn, self.used, attr.runs.len());
        self.used = end;
        self.count += 1;
        Some(())
    }

    /// The extent at `index`: its first VCN and its mapping pairs.
    pub(crate) fn span(&self, index: usize) -> Option<(u64, &[u8])> {
        let (vcn, at, len) = *self.spans[..self.count].get(index)?;
        Some((vcn, &self.bytes[at..at + len]))
    }
}

/// The boot sector's geometry, checked.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Geometry {
    pub(crate) sector_size: u32,
    pub(crate) cluster_size: u64,
    pub(crate) mft_record_size: usize,
    pub(crate) index_record_size: usize,
    pub(crate) total_sectors: u64,
    pub(crate) total_clusters: u64,
    pub(crate) mft_offset: u64,
    pub(crate) serial: u64,
    /// Bytes on the device.
    pub(crate) device_len: u64,
}

impl Geometry {
    /// Checks a boot sector.
    pub(crate) fn new(boot: &raw::BootSector, device_len: u64) -> Result<Self, Detail> {
        if boot.oem_id != raw::OEM_ID || boot.signature_value() != raw::BOOT_SIGNATURE {
            return Err(Detail::BootSector);
        }
        let sector_size = u32::from(boot.sector_size());
        if !(256..=4096).contains(&sector_size) || !sector_size.is_power_of_two() {
            return Err(Detail::Geometry);
        }
        let per_cluster = match boot.sectors_per_cluster {
            0 => return Err(Detail::Geometry),
            n if n <= 0x80 && n.is_power_of_two() => u64::from(n),
            n if n > 0x80 && 256 - u32::from(n) <= 12 => 1u64 << (256 - u32::from(n)),
            _ => return Err(Detail::Geometry),
        };
        let cluster_size = u64::from(sector_size) * per_cluster;
        if cluster_size > 2 * 1024 * 1024 {
            return Err(Detail::Geometry);
        }
        let mft_record_size = record::record_size(boot.clusters_per_mft_record, cluster_size)?;
        let index_record_size = record::record_size(boot.clusters_per_index_record, cluster_size)?;
        if mft_record_size > MAX_RECORD as u64 || index_record_size > MAX_RECORD as u64 {
            return Err(Detail::RecordSize);
        }
        let total_sectors = boot.sector_count();
        let total_clusters = total_sectors / per_cluster;
        let mft_cluster = boot.mft_cluster();
        if total_clusters == 0 || mft_cluster >= total_clusters {
            return Err(Detail::Geometry);
        }
        let mft_offset = mft_cluster
            .checked_mul(cluster_size)
            .ok_or(Detail::Geometry)?;
        Ok(Self {
            sector_size,
            cluster_size,
            mft_record_size: mft_record_size as usize,
            index_record_size: index_record_size as usize,
            total_sectors,
            total_clusters,
            mft_offset,
            serial: boot.serial(),
            device_len,
        })
    }
}

/// A page cache of `$UpCase`: the first page stays, one other is loaded on
/// demand.
#[derive(Debug, Clone)]
pub(crate) struct Upcase {
    pub(crate) stream: UpcaseExtents,
    pub(crate) low: [u16; PAGE],
    pub(crate) page: [u16; PAGE],
    pub(crate) page_index: Option<u16>,
}

/// Everything the driver keeps between calls.
#[derive(Debug, Clone)]
pub(crate) struct Info {
    pub(crate) geo: Geometry,
    pub(crate) mft: MftExtents,
    pub(crate) upcase: Upcase,
    pub(crate) root_sequence: u16,
    pub(crate) free_clusters: Option<u64>,
}

impl Info {
    pub(crate) fn root(&self) -> NodeId {
        node_id(raw::RECORD_ROOT, self.root_sequence)
    }

    pub(crate) fn capabilities(&self) -> Capabilities {
        Capabilities::new()
            .with_hard_links()
            .with_case_sensitivity(CaseSensitivity::InsensitivePreserving)
            .with_max_name_len(765)
            .with_name_charset(NameCharset::Utf16)
            .with_timestamp_resolution_ns(100)
    }
}

/// The node id of a file reference: the record number, with the sequence
/// number in the top 16 bits.
pub(crate) const fn node_id(record: u64, sequence: u16) -> NodeId {
    NodeId::new(record | (sequence as u64) << 48)
}

/// The `$INDEX_ROOT` of a directory's `$I30` index, borrowed from its
/// record.
pub(crate) struct DirIndex<'a> {
    pub(crate) root: &'a [u8],
    /// The size of an index block, when the directory has any.
    pub(crate) block_size: usize,
}

/// Index blocks a directory may have, so a cursor fits in 56 bits.
pub(crate) const MAX_BLOCKS: u64 = 1 << 40;

impl<'a> DirIndex<'a> {
    /// Reads a directory's `$INDEX_ROOT` attribute.
    pub(crate) fn new(attr: &record::Attr<'a>) -> Result<Self, Detail> {
        let Body::Resident(root) = attr.body else {
            return Err(Detail::Index);
        };
        if root.len() < 0x20 {
            return Err(Detail::Index);
        }
        Ok(Self {
            root,
            block_size: record::u32_at(root, 8) as usize,
        })
    }

    /// Checks the block size against the allocation's length and returns
    /// the number of blocks.
    pub(crate) fn blocks(&self, allocation_len: u64) -> Result<u64, Detail> {
        let size = self.block_size;
        if !size.is_power_of_two() || size < record::FIXUP_STRIDE {
            return Err(Detail::Index);
        }
        if size > MAX_RECORD {
            return Err(Detail::RecordSize);
        }
        if allocation_len % size as u64 != 0 || allocation_len / size as u64 >= MAX_BLOCKS {
            return Err(Detail::Index);
        }
        Ok(allocation_len / size as u64)
    }
}
