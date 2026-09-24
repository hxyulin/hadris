//! System use areas for Rock Ridge, split into the inline part and a
//! continuation area.

use alloc::vec::Vec;

use crate::raw::{PnEntry, PxEntry, U32Both};

const CE_LEN: usize = 28;

const ER_ID: &str = "RRIP_1991A";
const ER_DESCRIPTOR: &str =
    "THE ROCK RIDGE INTERCHANGE PROTOCOL PROVIDES SUPPORT FOR POSIX FILE SYSTEM SEMANTICS";
const ER_SOURCE: &str = "PLEASE CONTACT DISC PUBLISHER FOR SPECIFICATION SOURCE. SEE PUBLISHER IDENTIFIER IN PRIMARY VOLUME DESCRIPTOR FOR CONTACT INFORMATION.";

/// The entries of one system use area, most important first.
#[derive(Debug, Default)]
pub(crate) struct SuBuilder {
    entries: Vec<Vec<u8>>,
}

fn entry(signature: &[u8; 2], body: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(4 + body.len());
    out.extend_from_slice(signature);
    out.push((4 + body.len()) as u8);
    out.push(1);
    out.extend_from_slice(body);
    out
}

impl SuBuilder {
    pub(crate) fn sp(&mut self) {
        self.entries.push(entry(b"SP", &[0xBE, 0xEF, 0]));
    }

    pub(crate) fn er(&mut self) {
        let mut body = Vec::new();
        body.extend_from_slice(&[
            ER_ID.len() as u8,
            ER_DESCRIPTOR.len() as u8,
            ER_SOURCE.len() as u8,
            1,
        ]);
        body.extend_from_slice(ER_ID.as_bytes());
        body.extend_from_slice(ER_DESCRIPTOR.as_bytes());
        body.extend_from_slice(ER_SOURCE.as_bytes());
        self.entries.push(entry(b"ER", &body));
    }

    pub(crate) fn px(&mut self, mode: u32, links: u32, uid: u32, gid: u32, serial: u32) {
        let px = PxEntry {
            mode: U32Both::new(mode),
            links: U32Both::new(links),
            uid: U32Both::new(uid),
            gid: U32Both::new(gid),
            serial: U32Both::new(serial),
        };
        self.entries.push(entry(b"PX", bytemuck::bytes_of(&px)));
    }

    pub(crate) fn pn(&mut self, high: u32, low: u32) {
        let pn = PnEntry {
            high: U32Both::new(high),
            low: U32Both::new(low),
        };
        self.entries.push(entry(b"PN", bytemuck::bytes_of(&pn)));
    }

    /// `NM` entries for `name`, split every 250 bytes with `CONTINUE`.
    pub(crate) fn nm(&mut self, name: &[u8]) {
        let mut rest = name;
        while !rest.is_empty() {
            let (chunk, tail) = rest.split_at(rest.len().min(250));
            rest = tail;
            let mut body = Vec::with_capacity(1 + chunk.len());
            body.push(u8::from(!rest.is_empty()));
            body.extend_from_slice(chunk);
            self.entries.push(entry(b"NM", &body));
        }
    }

    pub(crate) fn nm_current(&mut self) {
        self.entries.push(entry(b"NM", &[0x02]));
    }

    pub(crate) fn nm_parent(&mut self) {
        self.entries.push(entry(b"NM", &[0x04]));
    }

    /// `SL` entries for `target`, split into components at `/`.
    pub(crate) fn sl(&mut self, target: &[u8]) {
        let mut components: Vec<(u8, &[u8])> = Vec::new();
        if target.first() == Some(&b'/') {
            components.push((0x08, &[]));
        }
        for part in target.split(|&b| b == b'/').filter(|part| !part.is_empty()) {
            match part {
                b"." => components.push((0x02, &[])),
                b".." => components.push((0x04, &[])),
                part => {
                    let mut chunks = part.chunks(248).peekable();
                    while let Some(chunk) = chunks.next() {
                        components.push((u8::from(chunks.peek().is_some()), chunk));
                    }
                }
            }
        }
        let mut body = alloc::vec![0u8];
        let mut pushed = false;
        for (flags, content) in components {
            if 4 + body.len() + 2 + content.len() > 255 {
                body[0] = 0x01;
                self.entries.push(entry(b"SL", &body));
                body = alloc::vec![0u8];
            }
            body.push(flags);
            body.push(content.len() as u8);
            body.extend_from_slice(content);
            pushed = true;
        }
        if pushed || body.len() == 1 {
            self.entries.push(entry(b"SL", &body));
        }
    }

    /// A short-form `TF` entry: creation when given, modification, access.
    pub(crate) fn tf(&mut self, created: Option<[u8; 7]>, modified: [u8; 7], accessed: [u8; 7]) {
        let mut body = alloc::vec![0x06u8];
        if let Some(created) = created {
            body[0] |= 0x01;
            body.extend_from_slice(&created);
        }
        body.extend_from_slice(&modified);
        body.extend_from_slice(&accessed);
        self.entries.push(entry(b"TF", &body));
    }

    pub(crate) fn cl(&mut self, block: u32) {
        self.entries
            .push(entry(b"CL", bytemuck::bytes_of(&U32Both::new(block))));
    }

    pub(crate) fn pl(&mut self, block: u32) {
        self.entries
            .push(entry(b"PL", bytemuck::bytes_of(&U32Both::new(block))));
    }

    pub(crate) fn re(&mut self) {
        self.entries.push(entry(b"RE", &[]));
    }

    fn size(&self) -> usize {
        self.entries.iter().map(Vec::len).sum()
    }

    /// Keeps whole entries inline while they fit `max_inline` bytes, leaving
    /// room for a `CE` entry that points at the rest.
    pub(crate) fn split(self, max_inline: usize) -> SplitSu {
        if self.size() <= max_inline {
            return SplitSu {
                inline: self.entries.concat(),
                overflow: Vec::new(),
                ce_offset: None,
            };
        }
        let budget = max_inline.saturating_sub(CE_LEN);
        let mut used = 0;
        let mut split = self.entries.len();
        for (index, entry) in self.entries.iter().enumerate() {
            if used + entry.len() > budget {
                split = index;
                break;
            }
            used += entry.len();
        }
        let mut inline = self.entries[..split].concat();
        let ce_offset = inline.len();
        inline.extend_from_slice(&entry(b"CE", &[0; 24]));
        SplitSu {
            inline,
            overflow: self.entries[split..].concat(),
            ce_offset: Some(ce_offset),
        }
    }
}

/// A system use area: the inline bytes and the continuation bytes.
#[derive(Debug, Clone, Default)]
pub(crate) struct SplitSu {
    pub(crate) inline: Vec<u8>,
    pub(crate) overflow: Vec<u8>,
    ce_offset: Option<usize>,
}

impl SplitSu {
    pub(crate) fn has_overflow(&self) -> bool {
        !self.overflow.is_empty()
    }

    /// Points the `CE` entry at `offset` in `block`.
    pub(crate) fn patch_ce(&mut self, block: u32, offset: u32) {
        if let Some(at) = self.ce_offset {
            let body = [
                U32Both::new(block),
                U32Both::new(offset),
                U32Both::new(self.overflow.len() as u32),
            ];
            self.inline[at + 4..at + 28].copy_from_slice(bytemuck::cast_slice(&body));
        }
    }
}

/// The system use bytes a record with an identifier of `name_len` bytes
/// can hold inline.
pub(crate) const fn inline_space(name_len: usize) -> usize {
    let used = (33 + name_len + 1) & !1;
    256usize.saturating_sub(used)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rock_ridge::Scan;

    #[test]
    fn long_areas_split_at_entry_boundaries() {
        let mut builder = SuBuilder::default();
        builder.sp();
        builder.px(0o40755, 2, 0, 0, 1);
        builder.nm_current();
        builder.er();
        let mut split = builder.split(inline_space(1));
        assert!(split.has_overflow());
        split.patch_ce(30, 12);
        let mut scan = Scan::new();
        scan.feed(&split.inline, 0);
        let ce = scan.next.unwrap();
        assert_eq!((ce.block.get(), ce.offset.get()), (30, 12));
        assert_eq!(ce.length.get() as usize, split.overflow.len());
        scan.feed(&split.overflow, 0);
        assert!(scan.rrip);
    }

    #[test]
    fn symlinks_round_trip_through_the_reader() {
        for target in [&b"/usr/lib/x"[..], b"../a/./b", b"plain"] {
            let mut builder = SuBuilder::default();
            builder.sl(target);
            let split = builder.split(250);
            let mut buf = [0u8; 64];
            let mut scan = Scan::new().with_link(&mut buf);
            scan.feed(&split.inline, 0);
            let len = scan.link_len().unwrap();
            assert_eq!(&buf[..len], target);
        }
        let long = alloc::vec![b'x'; 600];
        let mut builder = SuBuilder::default();
        builder.sl(&long);
        let split = builder.split(10_000);
        let mut buf = [0u8; 700];
        let mut scan = Scan::new().with_link(&mut buf);
        scan.feed(&split.inline, 0);
        let len = scan.link_len().unwrap();
        assert_eq!(&buf[..len], &long[..]);
    }
}
