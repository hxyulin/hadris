//! Detection of ISO 9660 and UDF volumes.
//!
//! Detection reads the volume descriptors of a block device. It does not
//! validate the volumes; `OpenOpticalImage` or the format crate does that
//! when it opens them.

const SECTOR_SIZE: usize = 2048;
const FIRST_DESCRIPTOR_SECTOR: u64 = 16;
const DESCRIPTORS_TO_SCAN: usize = 16;

/// UDF Volume Recognition Sequence generation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum UdfVrs {
    /// NSR02, used by UDF 1.02 through 1.50.
    Nsr02,
    /// NSR03, used by UDF 2.00 and later.
    Nsr03,
}

/// Filesystems recognized in one optical image.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct OpticalFormats {
    iso9660: bool,
    udf: Option<UdfVrs>,
}

impl OpticalFormats {
    #[cfg(test)]
    pub(crate) const fn new(iso9660: bool, udf: Option<UdfVrs>) -> Self {
        Self { iso9660, udf }
    }

    /// Returns whether an ISO 9660 volume descriptor was found.
    pub const fn has_iso9660(self) -> bool {
        self.iso9660
    }

    /// Returns the recognized UDF VRS generation.
    pub const fn udf(self) -> Option<UdfVrs> {
        self.udf
    }

    /// Returns whether both ISO 9660 and UDF were recognized.
    pub const fn is_bridge(self) -> bool {
        self.iso9660 && self.udf.is_some()
    }

    /// Returns whether no supported filesystem was recognized.
    pub const fn is_empty(self) -> bool {
        !self.iso9660 && self.udf.is_none()
    }
}

#[derive(Default)]
struct ScanState {
    formats: OpticalFormats,
    found_bea: bool,
    pending_udf: Option<UdfVrs>,
}

impl ScanState {
    fn inspect(&mut self, sector: &[u8; SECTOR_SIZE]) {
        if sector[6] != 1 {
            return;
        }
        match &sector[1..6] {
            b"CD001" => self.formats.iso9660 = true,
            b"BEA01" if sector[0] == 0 => {
                self.found_bea = true;
                self.pending_udf = None;
            }
            b"NSR02" if sector[0] == 0 && self.found_bea => self.pending_udf = Some(UdfVrs::Nsr02),
            b"NSR03" if sector[0] == 0 && self.found_bea => self.pending_udf = Some(UdfVrs::Nsr03),
            b"TEA01" if sector[0] == 0 && self.pending_udf.is_some() => {
                self.formats.udf = self.pending_udf;
            }
            _ => {}
        }
    }
}

/// The largest device block the detectors read.
#[cfg(any(feature = "sync", feature = "async"))]
const MAX_BLOCK: usize = 4096;

#[cfg(any(feature = "sync", feature = "async"))]
macro_rules! scan {
    ($dev:ident $(, $aw:tt)?) => {{
        let block = $dev.block_size().get() as usize;
        if block > MAX_BLOCK {
            return Ok(None);
        }
        let len = $dev.block_count().saturating_mul(block as u64);
        let mut state = ScanState::default();
        let mut buf = [0u8; MAX_BLOCK];
        let mut sector = [0u8; SECTOR_SIZE];
        for i in 0..DESCRIPTORS_TO_SCAN as u64 {
            let at = (FIRST_DESCRIPTOR_SECTOR + i) * SECTOR_SIZE as u64;
            if at + SECTOR_SIZE as u64 > len {
                break;
            }
            let first = BlockIndex::new(at / block as u64);
            if block >= SECTOR_SIZE {
                $dev.read_blocks(first, &mut buf[..block])$(.$aw)??;
                let within = (at % block as u64) as usize;
                sector.copy_from_slice(&buf[within..within + SECTOR_SIZE]);
            } else {
                $dev.read_blocks(first, &mut sector)$(.$aw)??;
            }
            state.inspect(&sector);
        }
        Ok((!state.formats.is_empty()).then_some(state.formats))
    }};
}

#[cfg(feature = "sync")]
#[cfg_attr(docsrs, doc(cfg(feature = "sync")))]
/// Blocking detection.
pub mod sync {
    use super::{
        DESCRIPTORS_TO_SCAN, FIRST_DESCRIPTOR_SECTOR, MAX_BLOCK, OpticalFormats, SECTOR_SIZE,
        ScanState,
    };
    use hadris_storage::BlockIndex;
    use hadris_storage::sync::BlockDevice;

    /// Detects the ISO 9660 and UDF volumes of `dev` from the volume
    /// descriptors in the 16 sectors of 2048 bytes after byte 32768.
    ///
    /// Devices whose blocks are larger than 4096 bytes give `None`.
    pub fn detect<D: BlockDevice + ?Sized>(
        dev: &mut D,
    ) -> hadris_fs::FsResult<Option<OpticalFormats>, D::Error> {
        scan!(dev)
    }
}

#[cfg(feature = "async")]
#[cfg_attr(docsrs, doc(cfg(feature = "async")))]
/// Asynchronous detection over the `Send` devices of
/// `hadris_storage::r#async`.
pub mod r#async {
    use super::{
        DESCRIPTORS_TO_SCAN, FIRST_DESCRIPTOR_SECTOR, MAX_BLOCK, OpticalFormats, SECTOR_SIZE,
        ScanState,
    };
    use hadris_storage::BlockIndex;
    use hadris_storage::r#async::BlockDevice;

    /// Detects the ISO 9660 and UDF volumes of `dev` from the volume
    /// descriptors in the 16 sectors of 2048 bytes after byte 32768.
    ///
    /// Devices whose blocks are larger than 4096 bytes give `None`.
    pub async fn detect<D: BlockDevice + ?Sized>(
        dev: &mut D,
    ) -> hadris_fs::FsResult<Option<OpticalFormats>, D::Error> {
        scan!(dev, await)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn descriptor(id: &[u8; 5]) -> [u8; SECTOR_SIZE] {
        let mut sector = [0_u8; SECTOR_SIZE];
        sector[1..6].copy_from_slice(id);
        sector[6] = 1;
        sector
    }

    #[test]
    fn recognizes_iso_udf_and_bridge_sequences() {
        let mut state = ScanState::default();
        state.inspect(&descriptor(b"CD001"));
        assert!(state.formats.has_iso9660());
        assert_eq!(state.formats.udf(), None);
        state.inspect(&descriptor(b"BEA01"));
        state.inspect(&descriptor(b"NSR03"));
        assert_eq!(state.formats.udf(), None);
        state.inspect(&descriptor(b"TEA01"));
        assert_eq!(state.formats.udf(), Some(UdfVrs::Nsr03));
        assert!(state.formats.is_bridge());
    }

    #[test]
    fn rejects_incomplete_or_out_of_order_udf_sequences() {
        let mut state = ScanState::default();
        state.inspect(&descriptor(b"NSR02"));
        state.inspect(&descriptor(b"TEA01"));
        assert!(state.formats.is_empty());
    }
}
