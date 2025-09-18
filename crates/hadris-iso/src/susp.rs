//! System Use Sharing Protocol

use hadris_io::{self as io, Parsable, ReadExt, Writable};

use crate::types::U32LsbMsb;

#[repr(C)]
#[derive(Debug, Clone, Copy, bytemuck::Zeroable, bytemuck::Pod)]
pub struct SystemUseHeader {
    pub sig: [u8; 2],
    pub length: u8,
    pub version: u8,
}

impl Writable for SystemUseHeader {
    fn write<R: io::Write>(&self, writer: &mut R) -> io::Result<()> {
        writer.write_all(bytemuck::bytes_of(self))?;
        Ok(())
    }
}

pub trait SystemUseEntry: Writable {
    fn header(&self) -> SystemUseHeader;
    //fn parse_data(header: SystemUseHeader, data: &[u8]) -> Self;
}

/// SPEC: SUSP 5.1 Description of the CE System Use Field
///
/// If this field is specified, then the System Use area is specified in another sector
#[repr(C)]
#[derive(Debug, Clone, Copy, bytemuck::Zeroable, bytemuck::Pod)]
pub struct ContinuationArea {
    /// The Sector of the Continuation System Use area
    pub sector: U32LsbMsb,
    /// The offset within the sector for the start of the SU
    pub offset: U32LsbMsb,
    pub length: U32LsbMsb,
}

#[derive(Debug, Clone, Copy)]
pub struct PaddingField {
    pub length: usize,
}

#[repr(C)]
#[derive(Debug, Clone, Copy, bytemuck::Zeroable, bytemuck::Pod)]
pub struct SuspIndetifier {
    pub check_bytes: [u8; 2],
    pub bytes_skipped: u8,
}

#[derive(Debug, Clone, Copy)]
pub struct SuspTerminator;
