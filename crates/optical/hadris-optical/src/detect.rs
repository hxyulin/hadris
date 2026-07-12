//! Non-destructive optical-filesystem detection.

const SECTOR_SIZE: usize = 2048;
const FIRST_DESCRIPTOR_SECTOR: u64 = 16;
const DESCRIPTORS_TO_SCAN: usize = 16;

/// UDF Volume Recognition Sequence generation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum UdfVrs {
    /// NSR02, used by UDF 1.02 through 1.50.
    Nsr02,
    /// NSR03, used by UDF 2.00 and later.
    Nsr03,
}

/// Filesystems recognized in one optical image.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct OpticalFormats {
    iso9660: bool,
    udf: Option<UdfVrs>,
}

impl OpticalFormats {
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

#[cfg(feature = "sync")]
pub mod sync {
    use super::*;
    use hadris_io::sync::{Read, Seek};
    use hadris_io::{ErrorKind, Result, SeekFrom};

    /// Detects every recognized optical filesystem and restores source position.
    pub fn detect<R>(source: &mut R) -> Result<Option<OpticalFormats>>
    where
        R: Read + Seek<Error = <R as Read>::Error>,
    {
        let original = source.stream_position().map_err(hadris_io::Error::erase)?;
        let result = detect_at_descriptors(source);
        source
            .seek(SeekFrom::Start(original))
            .map_err(hadris_io::Error::erase)?;
        result
    }

    fn detect_at_descriptors<R>(source: &mut R) -> Result<Option<OpticalFormats>>
    where
        R: Read + Seek<Error = <R as Read>::Error>,
    {
        source
            .seek(SeekFrom::Start(
                FIRST_DESCRIPTOR_SECTOR * SECTOR_SIZE as u64,
            ))
            .map_err(hadris_io::Error::erase)?;
        let mut state = ScanState::default();
        let mut sector = [0_u8; SECTOR_SIZE];
        for _ in 0..DESCRIPTORS_TO_SCAN {
            if let Err(error) = source.read_exact(&mut sector) {
                if error.kind() == ErrorKind::UnexpectedEof {
                    break;
                }
                return Err(error);
            }
            state.inspect(&sector);
        }
        Ok((!state.formats.is_empty()).then_some(state.formats))
    }
}

#[cfg(feature = "async")]
pub mod r#async {
    use super::*;
    use hadris_io::r#async::{Read, Seek};
    use hadris_io::{ErrorKind, Result, SeekFrom};

    /// Asynchronously detects all optical filesystems and restores source position.
    pub async fn detect<R>(source: &mut R) -> Result<Option<OpticalFormats>>
    where
        R: Read + Seek<Error = <R as Read>::Error>,
    {
        let original = source
            .stream_position()
            .await
            .map_err(hadris_io::Error::erase)?;
        let result = detect_at_descriptors(source).await;
        source
            .seek(SeekFrom::Start(original))
            .await
            .map_err(hadris_io::Error::erase)?;
        result
    }

    async fn detect_at_descriptors<R>(source: &mut R) -> Result<Option<OpticalFormats>>
    where
        R: Read + Seek<Error = <R as Read>::Error>,
    {
        source
            .seek(SeekFrom::Start(
                FIRST_DESCRIPTOR_SECTOR * SECTOR_SIZE as u64,
            ))
            .await
            .map_err(hadris_io::Error::erase)?;
        let mut state = ScanState::default();
        let mut sector = [0_u8; SECTOR_SIZE];
        for _ in 0..DESCRIPTORS_TO_SCAN {
            if let Err(error) = source.read_exact(&mut sector).await {
                if error.kind() == ErrorKind::UnexpectedEof {
                    break;
                }
                return Err(error);
            }
            state.inspect(&sector);
        }
        Ok((!state.formats.is_empty()).then_some(state.formats))
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
