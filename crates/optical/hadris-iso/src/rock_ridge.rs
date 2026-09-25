use hadris_fs::{DateTime, DeviceNumber, ErrorKind, FileType};

use crate::raw::{
    ContinuationArea, DecDateTime, DirDateTime, NmFlags, PnEntry, PxEntry, RRIP_IDENTIFIERS,
    SlComponentFlags, SuspEntries, TfFlags,
};

pub(crate) const S_IFMT: u32 = 0o170_000;
pub(crate) const S_IFSOCK: u32 = 0o140_000;
pub(crate) const S_IFLNK: u32 = 0o120_000;
pub(crate) const S_IFREG: u32 = 0o100_000;
pub(crate) const S_IFBLK: u32 = 0o060_000;
pub(crate) const S_IFDIR: u32 = 0o040_000;
pub(crate) const S_IFCHR: u32 = 0o020_000;
pub(crate) const S_IFIFO: u32 = 0o010_000;

/// The Rock Ridge entries of one node, as `IsoFs::rock_ridge` reads
/// them.
///
/// Fields a node has no entry for are `None`. Names and symlink targets are
/// not kept here, so the type needs no allocator: the listing gives the
/// name, and `read_link` the target.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct RockRidgeInfo {
    mode: Option<u32>,
    links: Option<u32>,
    owner: Option<(u32, u32)>,
    serial: Option<u32>,
    times: [Option<DateTime>; 4],
    device: Option<DeviceNumber>,
    has_name: bool,
    symlink: bool,
    child_link: Option<u32>,
    parent_link: Option<u32>,
    relocated: bool,
}

impl RockRidgeInfo {
    /// `st_mode` from the `PX` entry, with the file type bits.
    pub const fn mode(&self) -> Option<u32> {
        self.mode
    }

    /// `st_nlink` from the `PX` entry.
    pub const fn links(&self) -> Option<u32> {
        self.links
    }

    /// The owner user and group from the `PX` entry.
    pub const fn owner(&self) -> Option<(u32, u32)> {
        self.owner
    }

    /// `st_ino` from a RRIP 1.12 `PX` entry.
    pub const fn serial(&self) -> Option<u32> {
        self.serial
    }

    /// The creation time from the `TF` entry.
    pub const fn created(&self) -> Option<DateTime> {
        self.times[0]
    }

    /// The modification time from the `TF` entry.
    pub const fn modified(&self) -> Option<DateTime> {
        self.times[1]
    }

    /// The access time from the `TF` entry.
    pub const fn accessed(&self) -> Option<DateTime> {
        self.times[2]
    }

    /// The attribute change time from the `TF` entry.
    pub const fn changed(&self) -> Option<DateTime> {
        self.times[3]
    }

    /// Whether the node has a `TF` entry with any of the four times.
    pub(crate) fn has_times(&self) -> bool {
        self.times.iter().any(Option::is_some)
    }

    /// The device number from the `PN` entry.
    pub const fn device(&self) -> Option<DeviceNumber> {
        self.device
    }

    /// Whether an `NM` entry gives the node a POSIX name.
    pub const fn has_name(&self) -> bool {
        self.has_name
    }

    /// Whether an `SL` entry makes the node a symbolic link.
    pub const fn is_symlink(&self) -> bool {
        self.symlink
    }

    /// The logical block of the directory a `CL` entry relocates here.
    pub const fn child_link(&self) -> Option<u32> {
        self.child_link
    }

    /// The logical block of the real parent from a `PL` entry.
    pub const fn parent_link(&self) -> Option<u32> {
        self.parent_link
    }

    /// Whether an `RE` entry marks the directory as relocated, so it is
    /// hidden from its physical parent.
    pub const fn is_relocated(&self) -> bool {
        self.relocated
    }

    /// The file type the `PX` mode names.
    pub(crate) fn file_type(&self) -> Option<FileType> {
        Some(match self.mode? & S_IFMT {
            S_IFDIR => FileType::Dir,
            S_IFLNK => FileType::Symlink,
            S_IFCHR => FileType::CharDevice,
            S_IFBLK => FileType::BlockDevice,
            S_IFIFO => FileType::Fifo,
            S_IFSOCK => FileType::Socket,
            S_IFREG => FileType::File,
            _ => return None,
        })
    }
}

/// A byte buffer that entries are joined into.
pub(crate) struct Sink<'a> {
    pub(crate) buf: &'a mut [u8],
    pub(crate) len: usize,
    pub(crate) overflow: bool,
}

impl<'a> Sink<'a> {
    pub(crate) fn new(buf: &'a mut [u8]) -> Self {
        Self {
            buf,
            len: 0,
            overflow: false,
        }
    }

    fn push(&mut self, bytes: &[u8]) {
        match self.buf.get_mut(self.len..self.len + bytes.len()) {
            Some(dst) => {
                dst.copy_from_slice(bytes);
                self.len += bytes.len();
            }
            None => self.overflow = true,
        }
    }
}

/// Collects the SUSP and Rock Ridge entries of a node across its system use
/// area and continuation areas.
pub(crate) struct Scan<'a> {
    pub(crate) info: RockRidgeInfo,
    pub(crate) next: Option<ContinuationArea>,
    pub(crate) sp: Option<u8>,
    pub(crate) rrip: bool,
    name: Option<Sink<'a>>,
    name_done: bool,
    link: Option<Sink<'a>>,
    link_done: bool,
    component_open: bool,
    pub(crate) malformed: bool,
}

impl<'a> Scan<'a> {
    pub(crate) fn new() -> Self {
        Self {
            info: RockRidgeInfo::default(),
            next: None,
            sp: None,
            rrip: false,
            name: None,
            name_done: false,
            link: None,
            link_done: false,
            component_open: false,
            malformed: false,
        }
    }

    pub(crate) fn with_name(mut self, buf: &'a mut [u8]) -> Self {
        self.name = Some(Sink::new(buf));
        self
    }

    pub(crate) fn with_link(mut self, buf: &'a mut [u8]) -> Self {
        self.link = Some(Sink::new(buf));
        self
    }

    /// The joined `NM` name, if there was one and it fit.
    pub(crate) fn name(&self) -> Result<Option<&[u8]>, ErrorKind> {
        match &self.name {
            Some(sink) if sink.overflow => Err(ErrorKind::NameTooLong),
            Some(sink) if self.info.has_name && sink.len > 0 => Ok(Some(&sink.buf[..sink.len])),
            _ => Ok(None),
        }
    }

    /// The length of the joined `SL` target.
    pub(crate) fn link_len(&self) -> Result<usize, ErrorKind> {
        match &self.link {
            Some(sink) if sink.overflow => Err(ErrorKind::LimitExceeded),
            Some(sink) => Ok(sink.len),
            None => Ok(0),
        }
    }

    /// Takes the entries of one area. A `CE` entry leaves the next area in
    /// [`next`](Self::next); the caller reads it and feeds it here.
    pub(crate) fn feed(&mut self, area: &[u8], skip: usize) {
        self.next = None;
        for entry in SuspEntries::new(area, skip) {
            let data = entry.data;
            match &entry.signature {
                b"SP" if data.len() >= 3 && data[..2] == [0xBE, 0xEF] => self.sp = Some(data[2]),
                b"CE" if data.len() >= 24 => {
                    self.next = Some(bytemuck::pod_read_unaligned(&data[..24]));
                }
                b"ER" if data.len() >= 4 => {
                    let id_len = usize::from(data[0]);
                    let id = data.get(4..4 + id_len).unwrap_or(&[]);
                    if RRIP_IDENTIFIERS.contains(&id) {
                        self.rrip = true;
                    }
                }
                b"RR" => self.rrip = true,
                b"PX" if data.len() >= 32 => {
                    let mut px = [0u8; 40];
                    let len = data.len().min(40);
                    px[..len].copy_from_slice(&data[..len]);
                    let px: PxEntry = bytemuck::cast(px);
                    self.info.mode = Some(px.mode.get());
                    self.info.links = Some(px.links.get());
                    self.info.owner = Some((px.uid.get(), px.gid.get()));
                    if len == 40 {
                        self.info.serial = Some(px.serial.get());
                    }
                }
                b"PN" if data.len() >= 16 => {
                    let pn: PnEntry = bytemuck::pod_read_unaligned(&data[..16]);
                    self.info.device = Some(DeviceNumber::new(pn.high.get(), pn.low.get()));
                }
                b"TF" if !data.is_empty() => self.times(data),
                b"NM" if !data.is_empty() => self.alternate_name(data),
                b"SL" if !data.is_empty() => self.symlink(data),
                b"CL" if data.len() >= 8 => {
                    let block: crate::raw::U32Both = bytemuck::pod_read_unaligned(&data[..8]);
                    self.info.child_link = Some(block.get());
                }
                b"PL" if data.len() >= 8 => {
                    let block: crate::raw::U32Both = bytemuck::pod_read_unaligned(&data[..8]);
                    self.info.parent_link = Some(block.get());
                }
                b"RE" => self.info.relocated = true,
                b"PX" | b"PN" | b"CL" | b"PL" | b"CE" | b"SP" => self.malformed = true,
                _ => {}
            }
        }
    }

    fn times(&mut self, data: &[u8]) {
        let flags = TfFlags::from_bits_retain(data[0]);
        let long = flags.contains(TfFlags::LONG_FORM);
        let size = if long { 17 } else { 7 };
        let mut rest = &data[1..];
        let mut times = self.info.times;
        for flag in [
            TfFlags::CREATION,
            TfFlags::MODIFY,
            TfFlags::ACCESS,
            TfFlags::ATTRIBUTES,
            TfFlags::BACKUP,
            TfFlags::EXPIRATION,
            TfFlags::EFFECTIVE,
        ] {
            if !flags.contains(flag) {
                continue;
            }
            let Some(stamp) = rest.get(..size) else {
                self.malformed = true;
                break;
            };
            rest = &rest[size..];
            let time = if long {
                bytemuck::pod_read_unaligned::<DecDateTime>(stamp).to_datetime()
            } else {
                bytemuck::pod_read_unaligned::<DirDateTime>(stamp).to_datetime()
            };
            let slot = match flag {
                TfFlags::CREATION => 0,
                TfFlags::MODIFY => 1,
                TfFlags::ACCESS => 2,
                TfFlags::ATTRIBUTES => 3,
                _ => continue,
            };
            times[slot] = time;
        }
        self.info.times = times;
    }

    fn alternate_name(&mut self, data: &[u8]) {
        let flags = NmFlags::from_bits_retain(data[0]);
        if flags.intersects(NmFlags::CURRENT | NmFlags::PARENT) || self.name_done {
            return;
        }
        self.info.has_name = true;
        if let Some(sink) = &mut self.name {
            sink.push(&data[1..]);
        }
        if !flags.contains(NmFlags::CONTINUE) {
            self.name_done = true;
        }
    }

    fn symlink(&mut self, data: &[u8]) {
        if self.link_done {
            return;
        }
        self.info.symlink = true;
        let entry_continues = data[0] & 0x01 != 0;
        let mut rest = &data[1..];
        while rest.len() >= 2 {
            let flags = SlComponentFlags::from_bits_retain(rest[0]);
            let len = usize::from(rest[1]);
            let Some(content) = rest.get(2..2 + len) else {
                self.malformed = true;
                break;
            };
            rest = &rest[2 + len..];
            let Some(sink) = &mut self.link else {
                continue;
            };
            let continuing = core::mem::replace(
                &mut self.component_open,
                flags.contains(SlComponentFlags::CONTINUE),
            );
            if continuing {
                sink.push(content);
                continue;
            }
            if flags.contains(SlComponentFlags::ROOT) {
                if sink.len == 0 {
                    sink.push(b"/");
                }
                continue;
            }
            if sink.len > 0 && sink.buf[sink.len - 1] != b'/' {
                sink.push(b"/");
            }
            if flags.contains(SlComponentFlags::CURRENT) {
                sink.push(b".");
            } else if flags.contains(SlComponentFlags::PARENT) {
                sink.push(b"..");
            } else {
                sink.push(content);
            }
        }
        if !entry_continues {
            self.link_done = true;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sl(flags: u8, components: &[(u8, &[u8])]) -> alloc::vec::Vec<u8> {
        let mut out = alloc::vec![b'S', b'L', 0, 1, flags];
        for (component, content) in components {
            out.extend_from_slice(&[*component, content.len() as u8]);
            out.extend_from_slice(content);
        }
        out[2] = out.len() as u8;
        out
    }

    #[test]
    fn symlink_components_join_across_entries() {
        let mut area = sl(0x01, &[(0x08, b""), (0x00, b"usr"), (0x01, b"li")]);
        area.extend(sl(0x00, &[(0x00, b"b"), (0x04, b""), (0x00, b"x")]));
        let mut buf = [0u8; 64];
        let mut scan = Scan::new().with_link(&mut buf);
        scan.feed(&area, 0);
        let len = scan.link_len().unwrap();
        assert!(scan.info.is_symlink());
        assert_eq!(&buf[..len], b"/usr/lib/../x");
    }

    #[test]
    fn names_continue_and_times_parse() {
        let mut area = alloc::vec::Vec::new();
        area.extend_from_slice(b"NM\x08\x01\x01abc");
        area.extend_from_slice(b"NM\x07\x01\x00de");
        area.extend_from_slice(b"TF\x0C\x01\x02");
        area.extend_from_slice(&[80, 1, 1, 0, 0, 0, 0]);
        let mut name = [0u8; 8];
        let mut scan = Scan::new().with_name(&mut name);
        scan.feed(&area, 0);
        assert_eq!(scan.name().unwrap(), Some(&b"abcde"[..]));
        assert_eq!(
            scan.info.modified().map(|t| t.unix_seconds()),
            Some(315_532_800)
        );
        let mut small = [0u8; 2];
        let mut scan = Scan::new().with_name(&mut small);
        scan.feed(&area, 0);
        assert_eq!(scan.name(), Err(ErrorKind::NameTooLong));
    }
}
