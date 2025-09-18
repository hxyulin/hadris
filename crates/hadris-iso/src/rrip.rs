//! Rock Ridege Interchange Protocol

use crate::susp::{SystemUseEntry, SystemUseHeader};
use hadris_common::types::{endian::LittleEndian, number::U64};
use hadris_io::{self as io, Writable, Write};

use crate::file::FixedFilename;

#[repr(C)]
#[derive(Debug, Clone, Copy, bytemuck::Zeroable, bytemuck::Pod)]
pub struct PXEntry {
    pub file_mode: U64<LittleEndian>,
    pub file_links: [u8; 8],
    pub file_user_id: [u8; 8],
    pub file_group_id: [u8; 8],
    pub file_serial_number: [u8; 8],
}

impl SystemUseEntry for PXEntry {
    fn header(&self) -> SystemUseHeader {
        SystemUseHeader {
            sig: *b"PX",
            length: 44,
            version: 1,
        }
    }
}

impl Writable for PXEntry {
    fn write<R: Write>(&self, writer: &mut R) -> io::Result<()> {
        self.header().write(writer)?;
        writer.write_all(bytemuck::bytes_of(self))?;
        Ok(())
    }
}

bitflags::bitflags! {
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct PosixFileMode: u64 {
        /// Read Permission (Owner)
        const S_IRUSR = 0x0000400;
        /// Write Perission (Owner)
        const S_IWUSR = 0x0000200;
        /// Execute Permission (Owner)
        const S_IXUSR = 0x0000100;
        /// Read Permission (Group)
        const S_IRGRP = 0x0000040;
        /// Write Permission (Group)
        const S_IWGRP = 0x0000020;
        /// Execute Permission (Group)
        const S_IXGRP = 0x0000010;
        /// Read Permission (Other)
        const S_IROTH = 0x0000004;
        /// Write Permission (Other)
        const S_IWOTH = 0x0000002;
        /// Execute Permission (Other)
        const S_IXOTH = 0x0000001;
        /// set user ID on execution
        const S_ISUID = 0x0002000;
        /// enforced file locking (shared w/ set group ID)
        const S_ENFMT = 0x0002000;
        /// save swapped text even after use
        const S_ISVTX = 0x0002000;
        /// socket
        const S_IFSOCK = 0x0002000;
        /// symbolic link
        const S_IFLNK = 0x0002000;
        /// regular
        const S_IFREG = 0x0002000;
        /// block special
        const S_IFBLK = 0x0002000;
        /// character special
        const S_IFCHR = 0x0002000;
        /// directory
        const S_IFDIR = 0x0002000;
        /// pipe or FIFO
        const S_IFIFO = 0x0002000;
    }
}

#[derive(Debug, Clone)]
pub struct NmEntry {
    pub flags: NmFlags,
    pub name: FixedFilename<255>,
}

impl NmEntry {
    pub fn new(flags: NmFlags, name: &[u8]) -> Self {
        Self {
            flags,
            name: FixedFilename::from(name),
        }
    }

    pub fn size(&self) -> usize {
        5 + self.name.len
    }
}

impl Writable for NmEntry {
    fn write<R: Write>(&self, writer: &mut R) -> std::io::Result<()> {
        self.header().write(writer)?;
        let mut buf = [0u8; 256];
        buf[0] = self.flags.bits();
        buf[1..self.name.len + 1].copy_from_slice(self.name.as_bytes());
        writer.write(&buf[0..self.name.len + 1])?;
        Ok(())
    }
}

impl SystemUseEntry for NmEntry {
    fn header(&self) -> SystemUseHeader {
        SystemUseHeader {
            sig: *b"NM",
            length: self.size() as u8,
            version: 1,
        }
    }
}

bitflags::bitflags! {
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct NmFlags: u8 {
        /// If set to ONE, the Alternate Name continues in next "NM"
        /// System Use Entry of this System Use Area.
        ///
        /// If set to ZERO, the Alternate Name does not continue.
        const CONTINUE = 1 << 0;
        /// If set to ONE, the Alternate Name refers to the current
        /// directory ("." in POSIX).
        ///
        ///If set to ZERO, the Alternate Name does not refer to the
        ///current directory.
        const CURRENT = 1 << 1;
        /// If set to ONE, the Alternate Name refers to the parent of
        /// the current directory (".." in POSIX)
        /// If set to ZERO, the Alternate Name does not refer to the
        /// parent of the current directory.
        const PARENT = 1 << 2;
        /// Should not be set
        const RESERVED1 = 1 << 3;
        /// Should not be set
        const RESERVED2 = 1 << 4;
        /// Use of this flag is implementation specific. Historically,
        /// this component has contained the network node name of
        /// the current system as defined in the uname structure of
        /// POSIX:4.4.1.2.
        const RESERVED3 = 1 << 5;
        const _ = !0;
    }
}

#[derive(Debug)]
pub enum RockRidgeEntry {
    PosixPermissions(PXEntry),
    LongName(NmEntry),
    Unknown,
}

pub struct SystemUseIter<'a> {
    data: &'a [u8],
    index: usize,
}

impl<'a> SystemUseIter<'a> {
    pub fn new(data: &'a [u8]) -> Self {
        Self { data, index: 0 }
    }
}

impl Iterator for SystemUseIter<'_> {
    type Item = RockRidgeEntry;

    fn next(&mut self) -> Option<Self::Item> {
        if self.index >= self.data.len() {
            return None;
        }
        let header: &SystemUseHeader =
            bytemuck::from_bytes(&self.data[self.index..self.index + size_of::<SystemUseHeader>()]);
        let ret = match header.sig {
            [b'N', b'M'] => {
                let nm = NmEntry::new(
                    NmFlags::from_bits_retain(self.data[self.index + 4]),
                    &self.data[self.index + 5..self.index + header.length as usize],
                );
                RockRidgeEntry::LongName(nm)
            }
            _ => RockRidgeEntry::Unknown,
        };
        self.index += header.length as usize;
        Some(ret)
    }
}
