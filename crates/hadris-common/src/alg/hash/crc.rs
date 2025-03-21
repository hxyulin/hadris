use crc::{CRC_32_ISO_HDLC, Crc};

const HASHER_ISO_HDLC: Crc<u32> = Crc::<u32>::new(&CRC_32_ISO_HDLC);
pub struct Crc32HasherIsoHdlc;

impl Crc32HasherIsoHdlc {
    pub fn checksum(data: &[u8]) -> u32 {
        HASHER_ISO_HDLC.checksum(data)
    }
}
