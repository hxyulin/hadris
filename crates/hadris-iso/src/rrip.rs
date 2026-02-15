//! Rock Ridge Interchange Protocol (RRIP)
//!
//! Rock Ridge extends ISO 9660 with POSIX filesystem semantics including:
//! - Long filenames (NM entries)
//! - POSIX permissions, ownership, timestamps (PX, TF entries)
//! - Symbolic links (SL entries)
//! - Device files (PN entries)
//! - Deep directory relocation (CL, PL, RE entries)

use crate::susp::{SystemUseEntry, SystemUseHeader};
use crate::types::U32LsbMsb;
use hadris_io::{self as io, Read, Writable, Write};

#[cfg(feature = "alloc")]
bitflags::bitflags! {
    /// POSIX file mode bits for PX entries
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct PosixFileMode: u32 {
        // File type bits (high nibble of mode)
        /// Socket
        const S_IFSOCK = 0o140000;
        /// Symbolic link
        const S_IFLNK  = 0o120000;
        /// Regular file
        const S_IFREG  = 0o100000;
        /// Block device
        const S_IFBLK  = 0o060000;
        /// Directory
        const S_IFDIR  = 0o040000;
        /// Character device
        const S_IFCHR  = 0o020000;
        /// FIFO (named pipe)
        const S_IFIFO  = 0o010000;

        // Permission bits
        /// Set user ID on execution
        const S_ISUID = 0o4000;
        /// Set group ID on execution
        const S_ISGID = 0o2000;
        /// Sticky bit
        const S_ISVTX = 0o1000;

        /// Owner read
        const S_IRUSR = 0o0400;
        /// Owner write
        const S_IWUSR = 0o0200;
        /// Owner execute
        const S_IXUSR = 0o0100;

        /// Group read
        const S_IRGRP = 0o0040;
        /// Group write
        const S_IWGRP = 0o0020;
        /// Group execute
        const S_IXGRP = 0o0010;

        /// Others read
        const S_IROTH = 0o0004;
        /// Others write
        const S_IWOTH = 0o0002;
        /// Others execute
        const S_IXOTH = 0o0001;
    }
}

#[cfg(feature = "alloc")]
impl PosixFileMode {
    /// Create mode for a regular file with given permissions
    pub fn regular(perms: u32) -> Self {
        Self::S_IFREG | Self::from_bits_truncate(perms & 0o7777)
    }

    /// Create mode for a directory with given permissions
    pub fn directory(perms: u32) -> Self {
        Self::S_IFDIR | Self::from_bits_truncate(perms & 0o7777)
    }

    /// Create mode for a symbolic link
    pub fn symlink() -> Self {
        Self::S_IFLNK | Self::from_bits_truncate(0o777)
    }

    /// Default permissions for files (0644)
    pub fn default_file() -> Self {
        Self::regular(0o644)
    }

    /// Default permissions for directories (0755)
    pub fn default_dir() -> Self {
        Self::directory(0o755)
    }
}

/// PX - POSIX file attributes entry
///
/// Contains mode, nlink, uid, gid, and optionally serial number
#[repr(C)]
#[derive(Debug, Clone, Copy, bytemuck::Zeroable, bytemuck::Pod)]
pub struct PxEntry {
    /// File mode (both-endian)
    pub file_mode: U32LsbMsb,
    /// Number of links (both-endian)
    pub file_links: U32LsbMsb,
    /// User ID (both-endian)
    pub file_uid: U32LsbMsb,
    /// Group ID (both-endian)
    pub file_gid: U32LsbMsb,
    /// Serial number / inode (both-endian) - optional, RRIP 1.12+
    pub file_serial: U32LsbMsb,
}

impl PxEntry {
    /// Create a new PX entry
    pub fn new(mode: u32, nlink: u32, uid: u32, gid: u32, serial: u32) -> Self {
        Self {
            file_mode: U32LsbMsb::new(mode),
            file_links: U32LsbMsb::new(nlink),
            file_uid: U32LsbMsb::new(uid),
            file_gid: U32LsbMsb::new(gid),
            file_serial: U32LsbMsb::new(serial),
        }
    }
}

impl SystemUseEntry for PxEntry {
    const SIG: &'static [u8; 2] = b"PX";

    fn header(&self) -> SystemUseHeader {
        SystemUseHeader {
            sig: *Self::SIG,
            length: 44, // 4 + 8*5 = 44 bytes with serial, 36 without
            version: 1,
        }
    }

    fn parse_data<R: Read>(header: SystemUseHeader, data: &mut R) -> io::Result<Self> {
        debug_assert_eq!(&header.sig, Self::SIG);
        let mut buf = [0u8; 40];
        let len = (header.length as usize).saturating_sub(4).min(40);
        data.read_exact(&mut buf[..len])?;

        Ok(Self {
            file_mode: *bytemuck::from_bytes(&buf[0..8]),
            file_links: *bytemuck::from_bytes(&buf[8..16]),
            file_uid: *bytemuck::from_bytes(&buf[16..24]),
            file_gid: *bytemuck::from_bytes(&buf[24..32]),
            file_serial: if len >= 40 {
                *bytemuck::from_bytes(&buf[32..40])
            } else {
                U32LsbMsb::new(0)
            },
        })
    }
}

impl Writable for PxEntry {
    fn write<W: Write>(&self, writer: &mut W) -> io::Result<()> {
        self.header().write(writer)?;
        writer.write_all(bytemuck::bytes_of(self))?;
        Ok(())
    }
}

/// PN - POSIX device number entry
///
/// Contains major and minor device numbers for character/block devices
#[repr(C)]
#[derive(Debug, Clone, Copy, bytemuck::Zeroable, bytemuck::Pod)]
pub struct PnEntry {
    /// Device high (major) number
    pub dev_high: U32LsbMsb,
    /// Device low (minor) number
    pub dev_low: U32LsbMsb,
}

impl PnEntry {
    /// Create a new PN entry
    pub fn new(major: u32, minor: u32) -> Self {
        Self {
            dev_high: U32LsbMsb::new(major),
            dev_low: U32LsbMsb::new(minor),
        }
    }

    /// Get the device number as a combined value
    pub fn dev(&self) -> u64 {
        ((self.dev_high.read() as u64) << 32) | (self.dev_low.read() as u64)
    }
}

impl SystemUseEntry for PnEntry {
    const SIG: &'static [u8; 2] = b"PN";

    fn header(&self) -> SystemUseHeader {
        SystemUseHeader {
            sig: *Self::SIG,
            length: 20, // 4 + 8 + 8
            version: 1,
        }
    }

    fn parse_data<R: Read>(header: SystemUseHeader, data: &mut R) -> io::Result<Self> {
        debug_assert_eq!(&header.sig, Self::SIG);
        let mut buf = [0u8; 16];
        data.read_exact(&mut buf)?;
        Ok(Self {
            dev_high: *bytemuck::from_bytes(&buf[0..8]),
            dev_low: *bytemuck::from_bytes(&buf[8..16]),
        })
    }
}

impl Writable for PnEntry {
    fn write<W: Write>(&self, writer: &mut W) -> io::Result<()> {
        self.header().write(writer)?;
        writer.write_all(bytemuck::bytes_of(self))?;
        Ok(())
    }
}

#[cfg(feature = "alloc")]
bitflags::bitflags! {
    /// Flags for NM (alternate name) entries
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct NmFlags: u8 {
        /// Name continues in next NM entry
        const CONTINUE = 0x01;
        /// Refers to current directory (.)
        const CURRENT = 0x02;
        /// Refers to parent directory (..)
        const PARENT = 0x04;
        /// Reserved
        const RESERVED3 = 0x08;
        /// Reserved
        const RESERVED4 = 0x10;
        /// Network node name (host)
        const HOST = 0x20;
    }
}

/// NM - Alternate name entry
///
/// Contains the real (POSIX) filename
#[derive(Debug, Clone)]
#[cfg(feature = "alloc")]
pub struct NmEntry {
    /// Flags indicating special name handling
    pub flags: NmFlags,
    /// The name content (may be partial if CONTINUE is set)
    pub name: alloc::vec::Vec<u8>,
}

#[cfg(feature = "alloc")]
impl NmEntry {
    /// Create a new NM entry with the given name
    pub fn new(name: &[u8]) -> Self {
        Self {
            flags: NmFlags::empty(),
            name: name.to_vec(),
        }
    }

    /// Create an NM entry for current directory (.)
    pub fn current() -> Self {
        Self {
            flags: NmFlags::CURRENT,
            name: alloc::vec::Vec::new(),
        }
    }

    /// Create an NM entry for parent directory (..)
    pub fn parent() -> Self {
        Self {
            flags: NmFlags::PARENT,
            name: alloc::vec::Vec::new(),
        }
    }

    /// Calculate the size of this entry
    pub fn size(&self) -> usize {
        5 + self.name.len()
    }
}

#[cfg(feature = "alloc")]
impl SystemUseEntry for NmEntry {
    const SIG: &'static [u8; 2] = b"NM";

    fn header(&self) -> SystemUseHeader {
        SystemUseHeader {
            sig: *Self::SIG,
            length: self.size() as u8,
            version: 1,
        }
    }

    fn parse_data<R: Read>(header: SystemUseHeader, data: &mut R) -> io::Result<Self> {
        debug_assert_eq!(&header.sig, Self::SIG);
        let mut flags_byte = [0u8; 1];
        data.read_exact(&mut flags_byte)?;
        let name_len = (header.length as usize).saturating_sub(5);
        let mut name = alloc::vec![0u8; name_len];
        data.read_exact(&mut name)?;
        Ok(Self {
            flags: NmFlags::from_bits_truncate(flags_byte[0]),
            name,
        })
    }
}

#[cfg(feature = "alloc")]
impl Writable for NmEntry {
    fn write<W: Write>(&self, writer: &mut W) -> io::Result<()> {
        self.header().write(writer)?;
        writer.write_all(&[self.flags.bits()])?;
        writer.write_all(&self.name)?;
        Ok(())
    }
}

#[cfg(feature = "alloc")]
bitflags::bitflags! {
    /// Flags for SL (symbolic link) component records
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct SlComponentFlags: u8 {
        /// Component continues in next record
        const CONTINUE = 0x01;
        /// Current directory (.)
        const CURRENT = 0x02;
        /// Parent directory (..)
        const PARENT = 0x04;
        /// Root directory (/)
        const ROOT = 0x08;
        /// Reserved
        const RESERVED4 = 0x10;
        /// Network mount point
        const VOLROOT = 0x20;
    }
}

/// A component of a symbolic link path
#[derive(Debug, Clone)]
#[cfg(feature = "alloc")]
pub struct SlComponent {
    /// Component flags
    pub flags: SlComponentFlags,
    /// Component content (path segment)
    pub content: alloc::vec::Vec<u8>,
}

#[cfg(feature = "alloc")]
impl SlComponent {
    /// Create a component for a path segment
    pub fn new(segment: &[u8]) -> Self {
        Self {
            flags: SlComponentFlags::empty(),
            content: segment.to_vec(),
        }
    }

    /// Create a root component (/)
    pub fn root() -> Self {
        Self {
            flags: SlComponentFlags::ROOT,
            content: alloc::vec::Vec::new(),
        }
    }

    /// Create a current directory component (.)
    pub fn current() -> Self {
        Self {
            flags: SlComponentFlags::CURRENT,
            content: alloc::vec::Vec::new(),
        }
    }

    /// Create a parent directory component (..)
    pub fn parent() -> Self {
        Self {
            flags: SlComponentFlags::PARENT,
            content: alloc::vec::Vec::new(),
        }
    }

    /// Size of this component record
    pub fn size(&self) -> usize {
        2 + self.content.len()
    }
}

/// SL - Symbolic link entry
#[derive(Debug, Clone)]
#[cfg(feature = "alloc")]
pub struct SlEntry {
    /// Flags for the overall SL entry
    pub flags: u8,
    /// Components of the symbolic link path
    pub components: alloc::vec::Vec<SlComponent>,
}

#[cfg(feature = "alloc")]
impl SlEntry {
    /// Create a symbolic link entry from a target path
    pub fn from_path(target: &str) -> Self {
        let mut components = alloc::vec::Vec::new();

        if target.starts_with('/') {
            components.push(SlComponent::root());
        }

        for segment in target.split('/').filter(|s| !s.is_empty()) {
            match segment {
                "." => components.push(SlComponent::current()),
                ".." => components.push(SlComponent::parent()),
                _ => components.push(SlComponent::new(segment.as_bytes())),
            }
        }

        Self {
            flags: 0,
            components,
        }
    }

    /// Calculate the size of this entry
    pub fn size(&self) -> usize {
        5 + self.components.iter().map(|c| c.size()).sum::<usize>()
    }
}

#[cfg(feature = "alloc")]
impl SystemUseEntry for SlEntry {
    const SIG: &'static [u8; 2] = b"SL";

    fn header(&self) -> SystemUseHeader {
        SystemUseHeader {
            sig: *Self::SIG,
            length: self.size() as u8,
            version: 1,
        }
    }

    fn parse_data<R: Read>(header: SystemUseHeader, data: &mut R) -> io::Result<Self> {
        debug_assert_eq!(&header.sig, Self::SIG);
        let mut flags = [0u8; 1];
        data.read_exact(&mut flags)?;

        let mut remaining = (header.length as usize).saturating_sub(5);
        let mut components = alloc::vec::Vec::new();

        while remaining >= 2 {
            let mut comp_header = [0u8; 2];
            data.read_exact(&mut comp_header)?;
            remaining -= 2;

            let comp_flags = SlComponentFlags::from_bits_truncate(comp_header[0]);
            let comp_len = comp_header[1] as usize;

            let mut content = alloc::vec![0u8; comp_len.min(remaining)];
            data.read_exact(&mut content)?;
            remaining -= content.len();

            components.push(SlComponent {
                flags: comp_flags,
                content,
            });
        }

        Ok(Self {
            flags: flags[0],
            components,
        })
    }
}

#[cfg(feature = "alloc")]
impl Writable for SlEntry {
    fn write<W: Write>(&self, writer: &mut W) -> io::Result<()> {
        self.header().write(writer)?;
        writer.write_all(&[self.flags])?;
        for component in &self.components {
            writer.write_all(&[component.flags.bits(), component.content.len() as u8])?;
            writer.write_all(&component.content)?;
        }
        Ok(())
    }
}

#[cfg(feature = "alloc")]
bitflags::bitflags! {
    /// Flags for TF (timestamp) entries indicating which timestamps are present
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct TfFlags: u8 {
        /// Creation time present
        const CREATION = 0x01;
        /// Modification time present
        const MODIFY = 0x02;
        /// Access time present
        const ACCESS = 0x04;
        /// Attribute change time present
        const ATTRIBUTES = 0x08;
        /// Backup time present
        const BACKUP = 0x10;
        /// Expiration time present
        const EXPIRATION = 0x20;
        /// Effective time present
        const EFFECTIVE = 0x40;
        /// Long form timestamps (17 bytes vs 7 bytes)
        const LONG_FORM = 0x80;
    }
}

/// TF - Timestamps entry
#[derive(Debug, Clone)]
#[cfg(feature = "alloc")]
pub struct TfEntry {
    /// Flags indicating which timestamps are present
    pub flags: TfFlags,
    /// Timestamps in order (creation, modify, access, attributes, backup, expiration, effective)
    /// Each is 7 bytes (short form) or 17 bytes (long form)
    pub timestamps: alloc::vec::Vec<u8>,
}

#[cfg(feature = "alloc")]
impl TfEntry {
    /// Create a new TF entry with modification and access times (short form)
    ///
    /// Times are in ISO 9660 7-byte format: YY MM DD HH MM SS TZ
    pub fn new_short(mtime: &[u8; 7], atime: &[u8; 7]) -> Self {
        let flags = TfFlags::MODIFY | TfFlags::ACCESS;
        let mut timestamps = alloc::vec::Vec::with_capacity(14);
        timestamps.extend_from_slice(mtime);
        timestamps.extend_from_slice(atime);
        Self { flags, timestamps }
    }

    /// Calculate the size of this entry
    pub fn size(&self) -> usize {
        5 + self.timestamps.len()
    }
}

#[cfg(feature = "alloc")]
impl SystemUseEntry for TfEntry {
    const SIG: &'static [u8; 2] = b"TF";

    fn header(&self) -> SystemUseHeader {
        SystemUseHeader {
            sig: *Self::SIG,
            length: self.size() as u8,
            version: 1,
        }
    }

    fn parse_data<R: Read>(header: SystemUseHeader, data: &mut R) -> io::Result<Self> {
        debug_assert_eq!(&header.sig, Self::SIG);
        let mut flags_byte = [0u8; 1];
        data.read_exact(&mut flags_byte)?;
        let flags = TfFlags::from_bits_truncate(flags_byte[0]);

        let ts_len = (header.length as usize).saturating_sub(5);
        let mut timestamps = alloc::vec![0u8; ts_len];
        data.read_exact(&mut timestamps)?;

        Ok(Self { flags, timestamps })
    }
}

#[cfg(feature = "alloc")]
impl Writable for TfEntry {
    fn write<W: Write>(&self, writer: &mut W) -> io::Result<()> {
        self.header().write(writer)?;
        writer.write_all(&[self.flags.bits()])?;
        writer.write_all(&self.timestamps)?;
        Ok(())
    }
}

/// CL - Child link entry (for relocated directories)
#[repr(C)]
#[derive(Debug, Clone, Copy, bytemuck::Zeroable, bytemuck::Pod)]
pub struct ClEntry {
    /// Location of the actual directory
    pub child_directory_location: U32LsbMsb,
}

impl ClEntry {
    /// Create a new CL entry
    pub fn new(location: u32) -> Self {
        Self {
            child_directory_location: U32LsbMsb::new(location),
        }
    }
}

impl SystemUseEntry for ClEntry {
    const SIG: &'static [u8; 2] = b"CL";

    fn header(&self) -> SystemUseHeader {
        SystemUseHeader {
            sig: *Self::SIG,
            length: 12,
            version: 1,
        }
    }

    fn parse_data<R: Read>(header: SystemUseHeader, data: &mut R) -> io::Result<Self> {
        debug_assert_eq!(&header.sig, Self::SIG);
        let mut buf = [0u8; 8];
        data.read_exact(&mut buf)?;
        Ok(Self {
            child_directory_location: *bytemuck::from_bytes(&buf),
        })
    }
}

impl Writable for ClEntry {
    fn write<W: Write>(&self, writer: &mut W) -> io::Result<()> {
        self.header().write(writer)?;
        writer.write_all(bytemuck::bytes_of(&self.child_directory_location))?;
        Ok(())
    }
}

/// PL - Parent link entry (for relocated directories)
#[repr(C)]
#[derive(Debug, Clone, Copy, bytemuck::Zeroable, bytemuck::Pod)]
pub struct PlEntry {
    /// Location of the original parent directory
    pub parent_directory_location: U32LsbMsb,
}

impl PlEntry {
    /// Create a new PL entry
    pub fn new(location: u32) -> Self {
        Self {
            parent_directory_location: U32LsbMsb::new(location),
        }
    }
}

impl SystemUseEntry for PlEntry {
    const SIG: &'static [u8; 2] = b"PL";

    fn header(&self) -> SystemUseHeader {
        SystemUseHeader {
            sig: *Self::SIG,
            length: 12,
            version: 1,
        }
    }

    fn parse_data<R: Read>(header: SystemUseHeader, data: &mut R) -> io::Result<Self> {
        debug_assert_eq!(&header.sig, Self::SIG);
        let mut buf = [0u8; 8];
        data.read_exact(&mut buf)?;
        Ok(Self {
            parent_directory_location: *bytemuck::from_bytes(&buf),
        })
    }
}

impl Writable for PlEntry {
    fn write<W: Write>(&self, writer: &mut W) -> io::Result<()> {
        self.header().write(writer)?;
        writer.write_all(bytemuck::bytes_of(&self.parent_directory_location))?;
        Ok(())
    }
}

/// RE - Relocated directory marker
///
/// This entry marks a directory as being relocated from its original position
#[derive(Debug, Clone, Copy)]
pub struct ReEntry;

impl SystemUseEntry for ReEntry {
    const SIG: &'static [u8; 2] = b"RE";

    fn header(&self) -> SystemUseHeader {
        SystemUseHeader {
            sig: *Self::SIG,
            length: 4,
            version: 1,
        }
    }

    fn parse_data<R: Read>(_header: SystemUseHeader, _data: &mut R) -> io::Result<Self> {
        Ok(Self)
    }
}

impl Writable for ReEntry {
    fn write<W: Write>(&self, writer: &mut W) -> io::Result<()> {
        self.header().write(writer)
    }
}

/// Options for Rock Ridge extension support
#[derive(Debug, Clone, Copy)]
#[cfg(feature = "alloc")]
pub struct RripOptions {
    /// Enable Rock Ridge extensions
    pub enabled: bool,
    /// Automatically relocate directories deeper than 8 levels
    pub relocate_deep_dirs: bool,
    /// Preserve POSIX permissions
    pub preserve_permissions: bool,
    /// Preserve owner/group information
    pub preserve_ownership: bool,
    /// Preserve timestamps
    pub preserve_timestamps: bool,
    /// Preserve symbolic links
    pub preserve_symlinks: bool,
    /// Preserve device files (char/block)
    pub preserve_devices: bool,
}

#[cfg(feature = "alloc")]
impl Default for RripOptions {
    fn default() -> Self {
        Self {
            enabled: true,
            relocate_deep_dirs: true,
            preserve_permissions: true,
            preserve_ownership: true,
            preserve_timestamps: true,
            preserve_symlinks: true,
            preserve_devices: true,
        }
    }
}

#[cfg(feature = "alloc")]
impl RripOptions {
    /// Create options with all features disabled
    pub fn disabled() -> Self {
        Self {
            enabled: false,
            relocate_deep_dirs: false,
            preserve_permissions: false,
            preserve_ownership: false,
            preserve_timestamps: false,
            preserve_symlinks: false,
            preserve_devices: false,
        }
    }
}

/// Builder for Rock Ridge system use entries
#[cfg(feature = "alloc")]
pub struct RripBuilder {
    builder: crate::susp::SystemUseBuilder,
}

#[cfg(feature = "alloc")]
impl RripBuilder {
    /// Create a new RRIP builder
    pub fn new() -> Self {
        Self {
            builder: crate::susp::SystemUseBuilder::new(),
        }
    }

    /// Add the SP entry (for root directory only)
    pub fn add_sp(&mut self, bytes_skipped: u8) -> &mut Self {
        self.builder.add_sp(bytes_skipped);
        self
    }

    /// Add the ER entry declaring RRIP support (for root directory only)
    pub fn add_rrip_er(&mut self) -> &mut Self {
        // RRIP 1.12 extension reference
        self.builder.add_er(
            "RRIP_1991A",
            "THE ROCK RIDGE INTERCHANGE PROTOCOL PROVIDES SUPPORT FOR POSIX FILE SYSTEM SEMANTICS",
            "PLEASE CONTACT DISC PUBLISHER FOR SPECIFICATION SOURCE. SEE PUBLISHER IDENTIFIER IN PRIMARY VOLUME DESCRIPTOR FOR CONTACT INFORMATION.",
            1,
        );
        self
    }

    /// Add a PX entry for POSIX attributes
    pub fn add_px(&mut self, mode: u32, nlink: u32, uid: u32, gid: u32, serial: u32) -> &mut Self {
        let px = PxEntry::new(mode, nlink, uid, gid, serial);
        let mut buf = alloc::vec![0u8; 44];
        buf[0..2].copy_from_slice(b"PX");
        buf[2] = 44;
        buf[3] = 1;
        buf[4..44].copy_from_slice(bytemuck::bytes_of(&px));
        self.builder.add_raw(buf);
        self
    }

    /// Add a PN entry for device files
    pub fn add_pn(&mut self, major: u32, minor: u32) -> &mut Self {
        let pn = PnEntry::new(major, minor);
        let mut buf = alloc::vec![0u8; 20];
        buf[0..2].copy_from_slice(b"PN");
        buf[2] = 20;
        buf[3] = 1;
        buf[4..20].copy_from_slice(bytemuck::bytes_of(&pn));
        self.builder.add_raw(buf);
        self
    }

    /// Add an NM entry for the alternate name
    pub fn add_nm(&mut self, name: &[u8]) -> &mut Self {
        // Handle names that are too long by splitting into multiple NM entries
        let mut remaining = name;
        let max_per_entry = 250; // Leave room for header + flags

        while !remaining.is_empty() {
            let chunk_len = remaining.len().min(max_per_entry);
            let (chunk, rest) = remaining.split_at(chunk_len);
            remaining = rest;

            let flags = if remaining.is_empty() { 0 } else { 0x01 }; // CONTINUE flag
            let total_len = 5 + chunk.len();
            let mut buf = alloc::vec![0u8; total_len];
            buf[0..2].copy_from_slice(b"NM");
            buf[2] = total_len as u8;
            buf[3] = 1;
            buf[4] = flags;
            buf[5..].copy_from_slice(chunk);
            self.builder.add_raw(buf);
        }
        self
    }

    /// Add an NM entry for current directory (.)
    pub fn add_nm_current(&mut self) -> &mut Self {
        let buf = alloc::vec![b'N', b'M', 5, 1, 0x02]; // CURRENT flag
        self.builder.add_raw(buf);
        self
    }

    /// Add an NM entry for parent directory (..)
    pub fn add_nm_parent(&mut self) -> &mut Self {
        let buf = alloc::vec![b'N', b'M', 5, 1, 0x04]; // PARENT flag
        self.builder.add_raw(buf);
        self
    }

    /// Add an SL entry for a symbolic link
    pub fn add_sl(&mut self, target: &str) -> &mut Self {
        let sl = SlEntry::from_path(target);
        let size = sl.size();
        let mut buf = alloc::vec![0u8; size];
        buf[0..2].copy_from_slice(b"SL");
        buf[2] = size as u8;
        buf[3] = 1;
        buf[4] = sl.flags;

        let mut offset = 5;
        for component in &sl.components {
            buf[offset] = component.flags.bits();
            buf[offset + 1] = component.content.len() as u8;
            buf[offset + 2..offset + 2 + component.content.len()]
                .copy_from_slice(&component.content);
            offset += 2 + component.content.len();
        }

        self.builder.add_raw(buf);
        self
    }

    /// Add a TF entry for timestamps
    pub fn add_tf_short(&mut self, mtime: &[u8; 7], atime: &[u8; 7]) -> &mut Self {
        let mut buf = alloc::vec![0u8; 19]; // 5 + 7 + 7
        buf[0..2].copy_from_slice(b"TF");
        buf[2] = 19;
        buf[3] = 1;
        buf[4] = 0x06; // MODIFY | ACCESS flags
        buf[5..12].copy_from_slice(mtime);
        buf[12..19].copy_from_slice(atime);
        self.builder.add_raw(buf);
        self
    }

    /// Add a CL entry for child link (relocated directory)
    pub fn add_cl(&mut self, child_location: u32) -> &mut Self {
        let cl = ClEntry::new(child_location);
        let mut buf = alloc::vec![0u8; 12];
        buf[0..2].copy_from_slice(b"CL");
        buf[2] = 12;
        buf[3] = 1;
        buf[4..12].copy_from_slice(bytemuck::bytes_of(&cl.child_directory_location));
        self.builder.add_raw(buf);
        self
    }

    /// Add a PL entry for parent link (relocated directory)
    pub fn add_pl(&mut self, parent_location: u32) -> &mut Self {
        let pl = PlEntry::new(parent_location);
        let mut buf = alloc::vec![0u8; 12];
        buf[0..2].copy_from_slice(b"PL");
        buf[2] = 12;
        buf[3] = 1;
        buf[4..12].copy_from_slice(bytemuck::bytes_of(&pl.parent_directory_location));
        self.builder.add_raw(buf);
        self
    }

    /// Add a RE entry (marks a relocated directory)
    pub fn add_re(&mut self) -> &mut Self {
        let buf = alloc::vec![b'R', b'E', 4, 1];
        self.builder.add_raw(buf);
        self
    }

    /// Add a CE entry pointing to a continuation area
    pub fn add_ce(&mut self, ce: crate::susp::ContinuationArea) -> &mut Self {
        self.builder.add_ce(ce);
        self
    }

    /// Add the ST terminator
    pub fn add_st(&mut self) -> &mut Self {
        self.builder.add_st();
        self
    }

    /// Get the total size of the system use area
    pub fn size(&self) -> usize {
        self.builder.size()
    }

    /// Build the system use area
    pub fn build(&self) -> alloc::vec::Vec<u8> {
        self.builder.build()
    }

    /// Split entries across inline and overflow areas.
    ///
    /// Delegates to [`SystemUseBuilder::build_split`].
    pub fn build_split(&self, max_inline: usize) -> crate::susp::SplitSu {
        self.builder.build_split(max_inline)
    }

    /// Check if the builder is empty
    pub fn is_empty(&self) -> bool {
        self.builder.is_empty()
    }
}

#[cfg(feature = "alloc")]
impl Default for RripBuilder {
    fn default() -> Self {
        Self::new()
    }
}

/// Parsed Rock Ridge entry
#[derive(Debug, Clone)]
#[cfg(feature = "alloc")]
pub enum RockRidgeEntry {
    /// PX - POSIX attributes
    PosixAttributes(PxEntry),
    /// PN - Device numbers
    DeviceNumber(PnEntry),
    /// NM - Alternate name
    AlternateName(NmEntry),
    /// SL - Symbolic link
    SymbolicLink(SlEntry),
    /// TF - Timestamps
    Timestamps(TfEntry),
    /// CL - Child link
    ChildLink(ClEntry),
    /// PL - Parent link
    ParentLink(PlEntry),
    /// RE - Relocated marker
    Relocated,
    /// Unknown entry
    Unknown(SystemUseHeader),
}

#[cfg(all(feature = "std", test))]
mod tests {
    use super::*;
    use alloc::vec;

    static_assertions::const_assert_eq!(size_of::<PxEntry>(), 40);
    static_assertions::const_assert_eq!(size_of::<PnEntry>(), 16);
    static_assertions::const_assert_eq!(size_of::<ClEntry>(), 8);
    static_assertions::const_assert_eq!(size_of::<PlEntry>(), 8);

    #[test]
    fn test_px_entry_new() {
        let px = PxEntry::new(0o100644, 1, 1000, 1000, 12345);
        assert_eq!(px.file_mode.read(), 0o100644);
        assert_eq!(px.file_links.read(), 1);
        assert_eq!(px.file_uid.read(), 1000);
        assert_eq!(px.file_gid.read(), 1000);
        assert_eq!(px.file_serial.read(), 12345);
    }

    #[test]
    fn test_px_entry_header() {
        let px = PxEntry::new(0o100644, 1, 0, 0, 0);
        let header = px.header();
        assert_eq!(&header.sig, b"PX");
        assert_eq!(header.length, 44);
        assert_eq!(header.version, 1);
    }

    #[test]
    fn test_pn_entry_new() {
        let pn = PnEntry::new(8, 1);
        assert_eq!(pn.dev_high.read(), 8);
        assert_eq!(pn.dev_low.read(), 1);
    }

    #[test]
    fn test_pn_entry_dev() {
        let pn = PnEntry::new(0x12345678, 0xABCDEF01);
        let dev = pn.dev();
        assert_eq!(dev, (0x12345678u64 << 32) | 0xABCDEF01u64);
    }

    #[test]
    fn test_cl_entry_new() {
        let cl = ClEntry::new(100);
        assert_eq!(cl.child_directory_location.read(), 100);
    }

    #[test]
    fn test_pl_entry_new() {
        let pl = PlEntry::new(50);
        assert_eq!(pl.parent_directory_location.read(), 50);
    }

    #[test]
    fn test_posix_file_mode_regular() {
        let mode = PosixFileMode::regular(0o644);
        assert!(mode.contains(PosixFileMode::S_IFREG));
        assert!(mode.contains(PosixFileMode::S_IRUSR));
        assert!(mode.contains(PosixFileMode::S_IWUSR));
        assert!(mode.contains(PosixFileMode::S_IRGRP));
        assert!(mode.contains(PosixFileMode::S_IROTH));
        assert!(!mode.contains(PosixFileMode::S_IXUSR));
    }

    #[test]
    fn test_posix_file_mode_directory() {
        let mode = PosixFileMode::directory(0o755);
        assert!(mode.contains(PosixFileMode::S_IFDIR));
        assert!(mode.contains(PosixFileMode::S_IRUSR));
        assert!(mode.contains(PosixFileMode::S_IWUSR));
        assert!(mode.contains(PosixFileMode::S_IXUSR));
        assert!(mode.contains(PosixFileMode::S_IRGRP));
        assert!(mode.contains(PosixFileMode::S_IXGRP));
    }

    #[test]
    fn test_posix_file_mode_symlink() {
        let mode = PosixFileMode::symlink();
        assert!(mode.contains(PosixFileMode::S_IFLNK));
    }

    #[test]
    fn test_nm_entry_new() {
        let nm = NmEntry::new(b"test_file.txt");
        assert!(nm.flags.is_empty());
        assert_eq!(nm.name, b"test_file.txt");
        assert_eq!(nm.size(), 5 + 13);
    }

    #[test]
    fn test_nm_entry_special() {
        let current = NmEntry::current();
        assert!(current.flags.contains(NmFlags::CURRENT));
        assert!(current.name.is_empty());

        let parent = NmEntry::parent();
        assert!(parent.flags.contains(NmFlags::PARENT));
        assert!(parent.name.is_empty());
    }

    #[test]
    fn test_sl_component_new() {
        let comp = SlComponent::new(b"subdir");
        assert!(comp.flags.is_empty());
        assert_eq!(comp.content, b"subdir");
        assert_eq!(comp.size(), 2 + 6);
    }

    #[test]
    fn test_sl_component_special() {
        let root = SlComponent::root();
        assert!(root.flags.contains(SlComponentFlags::ROOT));
        assert!(root.content.is_empty());

        let current = SlComponent::current();
        assert!(current.flags.contains(SlComponentFlags::CURRENT));

        let parent = SlComponent::parent();
        assert!(parent.flags.contains(SlComponentFlags::PARENT));
    }

    #[test]
    fn test_sl_entry_from_path_absolute() {
        let sl = SlEntry::from_path("/usr/bin/test");
        assert_eq!(sl.components.len(), 4); // root + usr + bin + test
        assert!(sl.components[0].flags.contains(SlComponentFlags::ROOT));
        assert_eq!(sl.components[1].content, b"usr");
        assert_eq!(sl.components[2].content, b"bin");
        assert_eq!(sl.components[3].content, b"test");
    }

    #[test]
    fn test_sl_entry_from_path_relative() {
        let sl = SlEntry::from_path("../lib/libtest.so");
        assert_eq!(sl.components.len(), 3); // .. + lib + libtest.so
        assert!(sl.components[0].flags.contains(SlComponentFlags::PARENT));
        assert_eq!(sl.components[1].content, b"lib");
        assert_eq!(sl.components[2].content, b"libtest.so");
    }

    #[test]
    fn test_sl_entry_from_path_current() {
        let sl = SlEntry::from_path("./local");
        assert_eq!(sl.components.len(), 2); // . + local
        assert!(sl.components[0].flags.contains(SlComponentFlags::CURRENT));
        assert_eq!(sl.components[1].content, b"local");
    }

    #[test]
    fn test_tf_entry_new_short() {
        let mtime = [126, 1, 15, 10, 30, 0, 0]; // 2026-01-15 10:30:00 UTC
        let atime = [126, 1, 15, 12, 0, 0, 0]; // 2026-01-15 12:00:00 UTC
        let tf = TfEntry::new_short(&mtime, &atime);
        assert!(tf.flags.contains(TfFlags::MODIFY));
        assert!(tf.flags.contains(TfFlags::ACCESS));
        assert!(!tf.flags.contains(TfFlags::LONG_FORM));
        assert_eq!(tf.timestamps.len(), 14);
    }

    #[test]
    fn test_rrip_options_default() {
        let opts = RripOptions::default();
        assert!(opts.enabled);
        assert!(opts.relocate_deep_dirs);
        assert!(opts.preserve_permissions);
        assert!(opts.preserve_ownership);
        assert!(opts.preserve_timestamps);
        assert!(opts.preserve_symlinks);
        assert!(opts.preserve_devices);
    }

    #[test]
    fn test_rrip_options_disabled() {
        let opts = RripOptions::disabled();
        assert!(!opts.enabled);
        assert!(!opts.relocate_deep_dirs);
        assert!(!opts.preserve_permissions);
    }

    #[test]
    fn test_rrip_builder_empty() {
        let builder = RripBuilder::new();
        assert!(builder.is_empty());
        assert_eq!(builder.size(), 0);
    }

    #[test]
    fn test_rrip_builder_px() {
        let mut builder = RripBuilder::new();
        builder.add_px(0o100644, 1, 1000, 1000, 12345);
        assert_eq!(builder.size(), 44);

        let data = builder.build();
        assert_eq!(&data[0..2], b"PX");
        assert_eq!(data[2], 44); // length
        assert_eq!(data[3], 1); // version
    }

    #[test]
    fn test_rrip_builder_pn() {
        let mut builder = RripBuilder::new();
        builder.add_pn(8, 1);
        assert_eq!(builder.size(), 20);

        let data = builder.build();
        assert_eq!(&data[0..2], b"PN");
        assert_eq!(data[2], 20);
    }

    #[test]
    fn test_rrip_builder_nm() {
        let mut builder = RripBuilder::new();
        builder.add_nm(b"test.txt");
        assert_eq!(builder.size(), 5 + 8); // header(5) + name(8)

        let data = builder.build();
        assert_eq!(&data[0..2], b"NM");
        assert_eq!(data[4], 0); // no flags
        assert_eq!(&data[5..], b"test.txt");
    }

    #[test]
    fn test_rrip_builder_nm_special() {
        let mut builder = RripBuilder::new();
        builder.add_nm_current();
        let data = builder.build();
        assert_eq!(data[4], 0x02); // CURRENT flag

        let mut builder = RripBuilder::new();
        builder.add_nm_parent();
        let data = builder.build();
        assert_eq!(data[4], 0x04); // PARENT flag
    }

    #[test]
    fn test_rrip_builder_sl() {
        let mut builder = RripBuilder::new();
        builder.add_sl("../lib");

        let data = builder.build();
        assert_eq!(&data[0..2], b"SL");
    }

    #[test]
    fn test_rrip_builder_cl_pl_re() {
        let mut builder = RripBuilder::new();
        builder.add_cl(100).add_pl(50).add_re();

        let data = builder.build();
        // CL
        assert_eq!(&data[0..2], b"CL");
        assert_eq!(data[2], 12);
        // PL starts at offset 12
        assert_eq!(&data[12..14], b"PL");
        assert_eq!(data[14], 12);
        // RE starts at offset 24
        assert_eq!(&data[24..26], b"RE");
        assert_eq!(data[26], 4);
    }

    #[test]
    fn test_rrip_builder_complete() {
        let mut builder = RripBuilder::new();
        builder
            .add_sp(0)
            .add_rrip_er()
            .add_px(0o100644, 1, 1000, 1000, 1)
            .add_nm(b"testfile.txt")
            .add_st();

        let data = builder.build();
        assert!(!data.is_empty());
        // Verify SP is first
        assert_eq!(&data[0..2], b"SP");
    }

    #[test]
    fn test_rrip_builder_long_name_split() {
        let mut builder = RripBuilder::new();
        // Create a name longer than 250 bytes
        let long_name = vec![b'a'; 300];
        builder.add_nm(&long_name);

        let data = builder.build();
        // Should have at least 2 NM entries
        assert_eq!(&data[0..2], b"NM");
        // First entry should have CONTINUE flag set
        assert_eq!(data[4] & 0x01, 0x01);
    }
}
