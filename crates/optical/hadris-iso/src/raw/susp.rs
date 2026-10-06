use core::iter::FusedIterator;

use super::U32Both;

/// The identifiers of Rock Ridge in an `ER` entry: RRIP 1.09, 1.10 and 1.12.
pub const RRIP_IDENTIFIERS: [&[u8]; 3] = [b"RRIP_1991A", b"IEEE_P1282", b"IEEE_1282"];

/// A SUSP `CE` entry's body (SUSP 5.1): where the system use area continues.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct ContinuationArea {
    /// The logical block of the continuation area.
    pub block: U32Both,
    /// The byte offset within that block.
    pub offset: U32Both,
    /// The length of the continuation area.
    pub length: U32Both,
}

/// A Rock Ridge `PX` entry's body: POSIX file attributes.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct PxEntry {
    /// `st_mode`, with the file type bits.
    pub mode: U32Both,
    /// `st_nlink`.
    pub links: U32Both,
    /// `st_uid`.
    pub uid: U32Both,
    /// `st_gid`.
    pub gid: U32Both,
    /// `st_ino`. RRIP 1.12 only; RRIP 1.10 entries are 8 bytes shorter.
    pub serial: U32Both,
}

/// A Rock Ridge `PN` entry's body: a device number.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct PnEntry {
    /// The high 32 bits of the device number, the major on most systems.
    pub high: U32Both,
    /// The low 32 bits of the device number, the minor on most systems.
    pub low: U32Both,
}

bitflags::bitflags! {
    /// Flags of a Rock Ridge `NM` entry.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
    pub struct NmFlags: u8 {
        /// The name continues in the next `NM` entry.
        const CONTINUE = 0x01;
        /// The name is `.`.
        const CURRENT = 0x02;
        /// The name is `..`.
        const PARENT = 0x04;
        /// The name is the host's name.
        const HOST = 0x20;
    }
}

bitflags::bitflags! {
    /// Flags of a component record in a Rock Ridge `SL` entry.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
    pub struct SlComponentFlags: u8 {
        /// The component continues in the next record.
        const CONTINUE = 0x01;
        /// The component is `.`.
        const CURRENT = 0x02;
        /// The component is `..`.
        const PARENT = 0x04;
        /// The component is the root, `/`.
        const ROOT = 0x08;
        /// The component is the volume root.
        const VOLROOT = 0x10;
        /// The component is the host name.
        const HOST = 0x20;
    }
}

bitflags::bitflags! {
    /// Flags of a Rock Ridge `TF` entry: which times follow, and their form.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
    pub struct TfFlags: u8 {
        /// Creation time.
        const CREATION = 0x01;
        /// Modification time.
        const MODIFY = 0x02;
        /// Access time.
        const ACCESS = 0x04;
        /// Attribute change time.
        const ATTRIBUTES = 0x08;
        /// Backup time.
        const BACKUP = 0x10;
        /// Expiration time.
        const EXPIRATION = 0x20;
        /// Effective time.
        const EFFECTIVE = 0x40;
        /// Times use the 17-byte descriptor form instead of 7 bytes.
        const LONG_FORM = 0x80;
    }
}

/// One entry of a system use area (SUSP 4.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SuspEntry<'a> {
    /// The two-letter signature.
    pub signature: [u8; 2],
    /// The entry version.
    pub version: u8,
    /// The bytes after the four-byte header.
    pub data: &'a [u8],
}

/// The entries of one system use area, up to an `ST` entry, the end, or an
/// entry whose length does not fit.
#[derive(Debug, Clone)]
pub struct SuspEntries<'a> {
    rest: &'a [u8],
}

impl<'a> SuspEntries<'a> {
    /// The entries of `area`, after skipping its first `skip` bytes (the
    /// `SP` entry's skip count; zero in the root's `.` record).
    pub fn new(area: &'a [u8], skip: usize) -> Self {
        Self {
            rest: area.get(skip..).unwrap_or(&[]),
        }
    }
}

impl<'a> Iterator for SuspEntries<'a> {
    type Item = SuspEntry<'a>;

    fn next(&mut self) -> Option<SuspEntry<'a>> {
        let rest = self.rest;
        if rest.len() < 4 {
            self.rest = &[];
            return None;
        }
        let len = usize::from(rest[2]);
        if len < 4 || len > rest.len() || &rest[..2] == b"ST" {
            self.rest = &[];
            return None;
        }
        self.rest = &rest[len..];
        Some(SuspEntry {
            signature: [rest[0], rest[1]],
            version: rest[3],
            data: &rest[4..len],
        })
    }
}

impl FusedIterator for SuspEntries<'_> {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entries_stop_at_terminators_and_bad_lengths() {
        let area = b"PX\x05\x01\x07NM\x06\x01\x00aST\x04\x01ZZ\x04\x01";
        let entries: [_; 2] = [*b"PX", *b"NM"];
        assert!(SuspEntries::new(area, 0).map(|e| e.signature).eq(entries));
        assert_eq!(SuspEntries::new(b"PX\x03\x01", 0).count(), 0);
        assert_eq!(SuspEntries::new(b"PX\x09\x01", 0).count(), 0);
        let skipped = SuspEntries::new(b"xxNM\x05\x01\x00", 2).next().unwrap();
        assert_eq!((skipped.signature, skipped.data), (*b"NM", &[0][..]));
    }
}
