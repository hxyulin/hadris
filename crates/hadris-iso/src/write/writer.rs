use std::{collections::HashMap, vec::Vec};

use core::ops::Range;

use crate::read::PathSeparator;
use crate::types::{Charset, CharsetD, CharsetD1};

use crate::write::options::JolietLevel;
use crate::{directory::DirectoryRef, write::options::BaseIsoLevel};

pub struct DirectoryId {
    indices: Vec<usize>,
}

impl DirectoryId {
    pub fn push(&mut self, index: usize) {
        self.indices.push(index);
    }

    pub fn pop(&mut self) -> usize {
        self.indices.pop().expect("directory underflow")
    }
}

#[derive(Debug)]
pub struct WrittenFiles<'a> {
    root: WrittenDirectory<'a>,
}

impl<'a> WrittenFiles<'a> {
    pub fn new() -> Self {
        static ROOT: &'static str = "";
        Self {
            root: WrittenDirectory::new(ROOT),
        }
    }

    pub fn find_file(&self, name: &str, sep: PathSeparator) -> Option<DirectoryRef> {
        let mut current_dir = DirectoryId {
            indices: Vec::new(),
        };
        let mut parts = name.split(sep.as_char()).collect::<Vec<_>>();
        // Empty path, not a valid file
        let filename = parts.pop()?;
        'outer: for part in parts {
            let dir = self.get(&current_dir);
            for (idx, dir) in dir.dirs.iter().enumerate() {
                if dir.name == part {
                    current_dir.push(idx);
                    continue 'outer;
                } 
            } 
            // didn't find
            return None;
        }

        let dir = self.get(&current_dir);
        dir.files.iter().find(|f| f.name == filename).map(|f| f.entry)
    }

    pub fn root_dir(&self) -> DirectoryId {
        DirectoryId {
            indices: Vec::new(),
        }
    }

    pub fn root_refs(&self) -> &HashMap<EntryType, DirectoryRef> {
        &self.root.entries
    }

    pub fn get_parent(&self, id: &DirectoryId) -> &WrittenDirectory<'a> {
        let mut dir = &self.root;
        for index in &id.indices[0..id.indices.len() - 1] {
            dir = &dir.dirs[*index];
        }
        dir
    }

    pub fn get(&self, id: &DirectoryId) -> &WrittenDirectory<'a> {
        let mut dir = &self.root;
        for index in &id.indices {
            dir = &dir.dirs[*index];
        }
        dir
    }

    pub fn get_mut(&mut self, id: &DirectoryId) -> &mut WrittenDirectory<'a> {
        let mut dir = &mut self.root;
        for index in &id.indices {
            dir = &mut dir.dirs[*index];
        }
        dir
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EntryType {
    Level1 { supports_lowercase: bool },
    Level2 { supports_lowercase: bool },
    Level3 { supports_lowercase: bool },
    Joliet(JolietLevel),
}

impl From<BaseIsoLevel> for EntryType {
    fn from(value: BaseIsoLevel) -> Self {
        match value {
            BaseIsoLevel::Level1 { supports_lowercase } => Self::Level1 { supports_lowercase },
            BaseIsoLevel::Level2 { supports_lowercase } => Self::Level2 { supports_lowercase },
        }
    }
}

impl From<JolietLevel> for EntryType {
    fn from(value: JolietLevel) -> Self {
        Self::Joliet(value)
    }
}

pub enum ConvertedName {
    Level1(FilenameL1),
    Level2(FilenameL2),
    Level3(FilenameL3),
    Joliet1(FixedFilename<207>),
}

impl ConvertedName {
    pub fn as_bytes(&self) -> &[u8] {
        match self {
            Self::Level1(name) => name.as_bytes(),
            Self::Level2(name) => name.as_bytes(),
            Self::Level3(name) => name.as_bytes(),
            Self::Joliet1(name) => name.as_bytes(),
        }
    }
}

impl EntryType {
    pub fn convert_name(self, name: &str) -> ConvertedName {
        match self {
            Self::Level1 { supports_lowercase } => {
                ConvertedName::Level1(convert_l1(name, supports_lowercase))
            }
            Self::Level2 { supports_lowercase } => {
                ConvertedName::Level2(convert_l2(name, supports_lowercase))
            }
            Self::Level3 { supports_lowercase } => {
                ConvertedName::Level3(convert_l3(name, supports_lowercase))
            }
            Self::Joliet(JolietLevel::Level1) => ConvertedName::Joliet1(convert_joliet1(name)),
        }
    }
}

#[derive(Debug)]
pub struct WrittenDirectory<'a> {
    pub name: &'a str,
    pub entries: HashMap<EntryType, DirectoryRef>,
    pub dirs: Vec<WrittenDirectory<'a>>,
    pub files: Vec<WrittenFile<'a>>,
}

impl<'a> WrittenDirectory<'a> {
    pub fn new(name: &'a str) -> Self {
        Self {
            name,
            entries: HashMap::new(),
            dirs: Vec::new(),
            files: Vec::new(),
        }
    }

    pub fn push_dir(&mut self, name: &'a str) -> usize {
        self.dirs.push(Self::new(name));
        self.dirs.len() - 1
    }
}

#[derive(Debug)]
pub struct WrittenFile<'a> {
    pub name: &'a str,
    pub entry: DirectoryRef,
}

#[derive(Clone)]
pub struct FixedFilename<const N: usize> {
    data: [u8; N],
    len: usize,
}

impl<const N: usize> FixedFilename<N> {
    pub const fn empty() -> Self {
        Self {
            data: [0; N],
            len: 0,
        }
    }

    pub fn as_str(&self) -> &str {
        unsafe { core::str::from_utf8_unchecked(self.as_bytes()) }
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.data[0..self.len]
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn push_slice(&mut self, slice: &[u8]) -> Range<usize> {
        assert!(self.len + slice.len() <= self.data.len());
        let start = self.len;
        self.len += slice.len();
        self.data[start..self.len].copy_from_slice(slice);
        start..self.len
    }

    pub fn push_byte(&mut self, b: u8) -> usize {
        self.data[self.len] = b;
        self.len += 1;
        self.len - 1
    }
}

pub type FilenameL1 = FixedFilename<14>;
pub type FilenameL2 = FixedFilename<32>;
pub type FilenameL3 = FixedFilename<207>;

pub fn convert_l1(name: &str, supports_lowercase: bool) -> FixedFilename<14> {
    let mut l1 = FixedFilename::empty();
    let name_bytes = name.as_bytes();
    match name.find('.') {
        Some(index) => {
            // We copy the basename, at most 8 bytes
            let basename = l1.push_slice(&name_bytes[0..index.min(8)]);
            let basename = l1.data[basename].iter_mut();
            if supports_lowercase {
                CharsetD1::substitute_invalid(basename);
            } else {
                CharsetD::substitute_invalid(basename);
            }
            let ext_len = (name.len() - index).min(3);
            l1.push_byte(b'.');
            let ext = l1.push_slice(&name_bytes[index + 1..name.len().min(index + 1 + ext_len)]);
            let ext = l1.data[ext].iter_mut();
            if supports_lowercase {
                CharsetD1::substitute_invalid(ext);
            } else {
                CharsetD::substitute_invalid(ext);
            }
        }
        None => {
            let len = name.len().min(8);
            let basename = l1.push_slice(&name_bytes[0..len]);
            let basename = l1.data[basename].iter_mut();
            if supports_lowercase {
                CharsetD1::substitute_invalid(basename);
            } else {
                CharsetD::substitute_invalid(basename);
            }
        }
    }
    l1.push_slice(b";1");
    l1
}

pub fn convert_l2(name: &str, supports_lowercase: bool) -> FilenameL2 {
    let mut l2 = FilenameL2::empty();
    let name_bytes = name.as_bytes();
    match name.find('.') {
        Some(index) => {
            let basename = l2.push_slice(&name_bytes[0..index]);
            let basename = l2.data[basename].iter_mut();
            if supports_lowercase {
                CharsetD1::substitute_invalid(basename);
            } else {
                CharsetD::substitute_invalid(basename);
            }

            l2.push_byte(b'.');
            let ext = l2.push_slice(&name_bytes[index + 1..]);
            let ext = l2.data[ext].iter_mut();
            if supports_lowercase {
                CharsetD1::substitute_invalid(ext);
            } else {
                CharsetD::substitute_invalid(ext);
            }
        }
        None => {
            let basename = l2.push_slice(&name_bytes[0..name.len()]);
            let basename = l2.data[basename].iter_mut();
            if supports_lowercase {
                CharsetD1::substitute_invalid(basename);
            } else {
                CharsetD::substitute_invalid(basename);
            }
        }
    }
    l2.push_slice(b";1");
    l2
}

pub fn convert_l3(name: &str, supports_lowercase: bool) -> FilenameL3 {
    let mut l3 = FilenameL3::empty();
    let name_bytes = name.as_bytes();
    match name.find('.') {
        Some(index) => {
            let basename = l3.push_slice(&name_bytes[0..index]);
            let basename = l3.data[basename].iter_mut();
            if supports_lowercase {
                CharsetD1::substitute_invalid(basename);
            } else {
                CharsetD::substitute_invalid(basename);
            }
            l3.push_byte(b'.');
            let ext = l3.push_slice(&name_bytes[index + 1..]);
            let ext = l3.data[ext].iter_mut();
            if supports_lowercase {
                CharsetD1::substitute_invalid(ext);
            } else {
                CharsetD::substitute_invalid(ext);
            }
        }
        None => {
            let basename = l3.push_slice(&name_bytes[0..name.len()]);
            let basename = l3.data[basename].iter_mut();
            if supports_lowercase {
                CharsetD1::substitute_invalid(basename);
            } else {
                CharsetD::substitute_invalid(basename);
            }
        }
    }
    l3
}

pub fn convert_joliet1(name: &str) -> FixedFilename<207> {
    let mut j1 = FixedFilename::empty();
    let mut written = 0;
    for c in name.encode_utf16() {
        if written >= 206 / 2 {
            // We reached the maximum we can write
            break;
        }
        let bytes = c.to_be_bytes();
        j1.push_slice(&bytes);
        written += 1;
    }

    j1
}

#[cfg(all(test, feature = "std"))]
mod tests {
    use super::*;

    #[test]
    fn test_convert_l1() {
        let orig = "this-is-the-original-file.@very-long-ext";
        let converted = convert_l1(orig, false);
        assert_eq!(converted.as_str(), "THIS_IS_._VE;1");
        let converted = convert_l1(orig, true);
        assert_eq!(converted.as_str(), "this_is_._ve;1");
    }
}
