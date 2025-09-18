use alloc::{collections::BTreeMap, string::String, sync::Arc, vec::Vec};
use hadris_common::types::endian::EndianType;
use hadris_io::{self as io, Write};

use crate::file::{FilenameL1, FilenameL2, FilenameL3, FixedFilename};
use crate::path::{PathTableEntry, PathTableEntryHeader};
use crate::read::PathSeparator;
use crate::types::{Charset, CharsetD, CharsetD1};

use crate::write::options::JolietLevel;
use crate::{directory::DirectoryRef, write::options::BaseIsoLevel};

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
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
pub struct WrittenFiles {
    root: WrittenDirectory,
}

impl WrittenFiles {
    pub fn new() -> Self {
        Self {
            root: WrittenDirectory::new(Arc::new(String::new())),
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
                if dir.name.as_str() == part {
                    current_dir.push(idx);
                    continue 'outer;
                }
            }
            // didn't find
            return None;
        }

        let dir = self.get(&current_dir);
        dir.files
            .iter()
            .find(|f| f.name.as_str() == filename)
            .map(|f| f.entry)
    }

    pub fn root_dir(&self) -> DirectoryId {
        DirectoryId {
            indices: Vec::new(),
        }
    }

    pub fn root_refs(&self) -> &BTreeMap<EntryType, DirectoryRef> {
        &self.root.entries
    }

    pub fn get(&self, id: &DirectoryId) -> &WrittenDirectory {
        let mut dir = &self.root;
        for index in &id.indices {
            dir = &dir.dirs[*index];
        }
        dir
    }

    pub fn get_mut(&mut self, id: &DirectoryId) -> &mut WrittenDirectory {
        let mut dir = &mut self.root;
        for index in &id.indices {
            dir = &mut dir.dirs[*index];
        }
        dir
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
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
pub struct WrittenDirectory {
    pub name: Arc<String>,
    pub entries: BTreeMap<EntryType, DirectoryRef>,
    pub dirs: Vec<WrittenDirectory>,
    pub files: Vec<WrittenFile>,
}

impl WrittenDirectory {
    pub fn new(name: Arc<String>) -> Self {
        Self {
            name,
            entries: BTreeMap::new(),
            dirs: Vec::new(),
            files: Vec::new(),
        }
    }

    pub fn push_dir(&mut self, name: Arc<String>) -> usize {
        self.dirs.push(Self::new(name));
        self.dirs.len() - 1
    }
}

#[derive(Debug)]
pub struct WrittenFile {
    pub name: Arc<String>,
    pub entry: DirectoryRef,
}

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

pub(crate) struct PathTableWriter<'a> {
    pub written_files: &'a mut WrittenFiles,
    pub ty: EntryType,
    pub endian: EndianType,
}

impl PathTableWriter<'_> {
    pub fn write<DATA: Write>(&mut self, data: &mut DATA) -> io::Result<()> {
        let mut current_number = 1;
        let mut dir_id = self.written_files.root_dir();
        {
            // Write root directory
            let dir = self.written_files.get(&dir_id);
            let header = PathTableEntryHeader {
                len: 1,
                extended_attr_record: 0,
                parent_directory_number: self.endian.u16_bytes(current_number),
                parent_lba: self
                    .endian
                    .u32_bytes(dir.entries.get(&self.ty).unwrap().extent.0 as u32),
            };
            data.write_all(bytemuck::bytes_of(&header))?;
            // '\x00' for the root directory, and one byte for padding
            data.write_all(&[0x00, 0x00])?;
        }
        Ok(())
    }
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
