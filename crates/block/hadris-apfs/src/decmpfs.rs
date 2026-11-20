use crate::{ApfsError, Result};

pub(crate) const ATTRIBUTE: &str = "com.apple.decmpfs";
pub(crate) const HEADER_SIZE: usize = 16;
#[cfg(feature = "compression")]
pub(crate) const RESOURCE_FORK: &str = "com.apple.ResourceFork";
#[cfg(feature = "compression")]
pub(crate) const CHUNK_SIZE: usize = 65536;

#[derive(Debug, Clone, Copy)]
pub(crate) struct Header {
    pub(crate) size: u64,
    #[cfg(feature = "compression")]
    method: u32,
}

impl Header {
    pub(crate) fn parse(bytes: &[u8; HEADER_SIZE]) -> Result<Self> {
        if &bytes[..4] != b"fpmc" {
            return Err(invalid());
        }
        Ok(Self {
            size: u64::from_le_bytes(crate::types::take(bytes, 8)?),
            #[cfg(feature = "compression")]
            method: crate::types::le_u32(bytes, 4)?,
        })
    }

    #[cfg(feature = "compression")]
    pub(crate) fn encoding(self) -> Result<(Codec, Layout)> {
        match self.method {
            1 => Ok((Codec::Stored, Layout::Attribute)),
            3 => Ok((Codec::Zlib, Layout::Attribute)),
            4 => Ok((Codec::Zlib, Layout::ZlibFork)),
            7 if cfg!(feature = "lzfse") => Ok((Codec::Lzvn, Layout::Attribute)),
            8 if cfg!(feature = "lzfse") => Ok((Codec::Lzvn, Layout::OffsetFork)),
            9 => Ok((Codec::Marked, Layout::Attribute)),
            10 => Ok((Codec::Marked, Layout::OffsetFork)),
            11 if cfg!(feature = "lzfse") => Ok((Codec::Lzfse, Layout::Attribute)),
            12 if cfg!(feature = "lzfse") => Ok((Codec::Lzfse, Layout::OffsetFork)),
            _ => Err(ApfsError::Unsupported("decmpfs compression method")),
        }
    }
}

pub(crate) const fn invalid() -> ApfsError {
    ApfsError::InvalidValue("decmpfs data")
}

#[cfg(feature = "compression")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Layout {
    Attribute,
    ZlibFork,
    OffsetFork,
}

#[cfg(feature = "compression")]
#[derive(Debug, Clone, Copy)]
pub(crate) enum Codec {
    Stored,
    Marked,
    Zlib,
    Lzvn,
    Lzfse,
}

#[cfg(feature = "compression")]
impl Codec {
    pub(crate) fn decode(self, input: &[u8], output: &mut [u8]) -> Result<()> {
        let stored = match (self, input.split_first()) {
            (Self::Stored, _) => Some(input),
            (Self::Marked, Some((0xCC, data)))
            | (Self::Lzvn, Some((0x06, data)))
            | (Self::Lzfse, Some((0xFF, data))) => Some(data),
            (Self::Zlib, Some((marker, data))) if marker & 0x0F == 0x0F => Some(data),
            _ => None,
        };
        if let Some(stored) = stored {
            if stored.len() != output.len() {
                return Err(invalid());
            }
            output.copy_from_slice(stored);
            return Ok(());
        }
        match self {
            Self::Zlib => {
                let bytes =
                    miniz_oxide::inflate::decompress_to_vec_zlib_with_limit(input, output.len())
                        .map_err(|_| invalid())?;
                if bytes.len() != output.len() {
                    return Err(invalid());
                }
                output.copy_from_slice(&bytes);
                Ok(())
            }
            #[cfg(feature = "lzfse")]
            Self::Lzvn | Self::Lzfse => self.decode_apple(input, output),
            _ => Err(invalid()),
        }
    }

    #[cfg(feature = "lzfse")]
    fn decode_apple(self, mut input: &[u8], output: &mut [u8]) -> Result<()> {
        use std::io::Read;

        let mut decoder = lzfse_rust::LzfseRingDecoder::default();
        let output_len = output.len();
        let mut sink = output;
        let consumed = if matches!(self, Self::Lzvn) {
            let mut header = *b"bvxn\0\0\0\0\0\0\0\0";
            header[4..8].copy_from_slice(&(output_len as u32).to_le_bytes());
            header[8..12].copy_from_slice(&(input.len() as u32).to_le_bytes());
            let mut framed = (&header[..]).chain(input).chain(&b"bvx$"[..]);
            let (read, _) = decoder
                .decode(&mut framed, &mut sink)
                .map_err(|_| invalid())?;
            read == (input.len() + 16) as u64
        } else {
            let len = input.len() as u64;
            let (read, _) = decoder
                .decode(&mut input, &mut sink)
                .map_err(|_| invalid())?;
            read == len
        };
        if !consumed || !sink.is_empty() {
            return Err(invalid());
        }
        Ok(())
    }
}
