use alloc::{collections::BTreeMap, string::String, sync::Arc, vec::Vec};
use hadris_common::types::endian::EndianType;
use hadris_common::types::file::FixedFilename;
use hadris_io::{self as io, Write};

use crate::file::{EntryType, FilenameL1, FilenameL2, FilenameL3};
use crate::path::PathTableEntryHeader;
use crate::read::PathSeparator;
use crate::types::{Charset, CharsetD, CharsetD1};

use crate::joliet::JolietLevel;
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

    pub fn find_file(&self, name: &str, _sep: PathSeparator) -> Option<DirectoryRef> {
        let mut current_dir = DirectoryId {
            indices: Vec::new(),
        };
        // Split on both separators for cross-platform compatibility
        let mut parts: Vec<&str> = name
            .split(|c| c == '/' || c == '\\')
            .filter(|s| !s.is_empty())
            .collect();
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

impl From<BaseIsoLevel> for EntryType {
    fn from(value: BaseIsoLevel) -> Self {
        match value {
            BaseIsoLevel::Level1 {
                supports_lowercase,
                supports_rrip,
            } => Self::Level1 {
                supports_lowercase,
                supports_rrip,
            },
            BaseIsoLevel::Level2 {
                supports_lowercase,
                supports_rrip,
            } => Self::Level2 {
                supports_lowercase,
                supports_rrip,
            },
        }
    }
}

impl From<JolietLevel> for EntryType {
    fn from(value: JolietLevel) -> Self {
        Self::Joliet {
            level: value,
            supports_rrip: false,
        }
    }
}

pub enum ConvertedName {
    Level1(FilenameL1),
    Level2(FilenameL2),
    Level3(FilenameL3),
    Joliet(FixedFilename<207>),
}

impl ConvertedName {
    pub fn as_bytes(&self) -> &[u8] {
        match self {
            Self::Level1(name) => name.as_bytes(),
            Self::Level2(name) => name.as_bytes(),
            Self::Level3(name) => name.as_bytes(),
            Self::Joliet(name) => name.as_bytes(),
        }
    }
}

impl EntryType {
    pub fn convert_name(self, name: &str) -> ConvertedName {
        match self {
            Self::Level1 {
                supports_lowercase, ..
            } => ConvertedName::Level1(convert_l1(name, supports_lowercase)),
            Self::Level2 {
                supports_lowercase, ..
            } => ConvertedName::Level2(convert_l2(name, supports_lowercase)),
            Self::Level3 {
                supports_lowercase, ..
            } => ConvertedName::Level3(convert_l3(name, supports_lowercase)),
            Self::Joliet { level, .. } => match level {
                // All Joliet levels use UTF-16 BE encoding
                JolietLevel::Level1 | JolietLevel::Level2 | JolietLevel::Level3 => {
                    ConvertedName::Joliet(convert_joliet3(name))
                }
            },
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
    // Max: 30 bytes for name (reserve 2 for ";1")
    const MAX_NAME_LEN: usize = 30;

    match name.find('.') {
        Some(index) => {
            let basename_end = index.min(MAX_NAME_LEN);
            let basename = l2.push_slice(&name_bytes[0..basename_end]);
            let basename = l2.data[basename].iter_mut();
            if supports_lowercase {
                CharsetD1::substitute_invalid(basename);
            } else {
                CharsetD::substitute_invalid(basename);
            }

            // Calculate remaining space for extension (subtract basename length and 1 for dot)
            let remaining = MAX_NAME_LEN.saturating_sub(basename_end + 1);
            if remaining > 0 {
                l2.push_byte(b'.');
                let ext_end = (index + 1 + remaining).min(name.len());
                let ext = l2.push_slice(&name_bytes[index + 1..ext_end]);
                let ext = l2.data[ext].iter_mut();
                if supports_lowercase {
                    CharsetD1::substitute_invalid(ext);
                } else {
                    CharsetD::substitute_invalid(ext);
                }
            }
        }
        None => {
            let len = name.len().min(MAX_NAME_LEN);
            let basename = l2.push_slice(&name_bytes[0..len]);
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
    // Max: 207 bytes for name (no version suffix in L3)
    const MAX_NAME_LEN: usize = 207;

    match name.find('.') {
        Some(index) => {
            let basename_end = index.min(MAX_NAME_LEN);
            let basename = l3.push_slice(&name_bytes[0..basename_end]);
            let basename = l3.data[basename].iter_mut();
            if supports_lowercase {
                CharsetD1::substitute_invalid(basename);
            } else {
                CharsetD::substitute_invalid(basename);
            }

            // Calculate remaining space for extension (subtract basename length and 1 for dot)
            let remaining = MAX_NAME_LEN.saturating_sub(basename_end + 1);
            if remaining > 0 {
                l3.push_byte(b'.');
                let ext_end = (index + 1 + remaining).min(name.len());
                let ext = l3.push_slice(&name_bytes[index + 1..ext_end]);
                let ext = l3.data[ext].iter_mut();
                if supports_lowercase {
                    CharsetD1::substitute_invalid(ext);
                } else {
                    CharsetD::substitute_invalid(ext);
                }
            }
        }
        None => {
            let len = name.len().min(MAX_NAME_LEN);
            let basename = l3.push_slice(&name_bytes[0..len]);
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

pub fn convert_joliet3(name: &str) -> FixedFilename<207> {
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
        let current_number = 1;
        let dir_id = self.written_files.root_dir();
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
    use alloc::format;
    use super::*;

    #[test]
    fn test_convert_l1() {
        let orig = "this-is-the-original-file.@very-long-ext";
        let converted = convert_l1(orig, false);
        assert_eq!(converted.as_str(), "THIS_IS_._VE;1");
        let converted = convert_l1(orig, true);
        assert_eq!(converted.as_str(), "this_is_._ve;1");
    }

    #[test]
    fn test_convert_l2_short_name() {
        let orig = "readme.txt";
        let converted = convert_l2(orig, false);
        assert_eq!(converted.as_str(), "README.TXT;1");
    }

    #[test]
    fn test_convert_l2_long_name_truncation() {
        // Max is 30 bytes for name + 2 for ";1" = 32 total
        let orig = "this-is-a-very-long-filename-that-should-be-truncated.extension";
        let converted = convert_l2(orig, false);
        // Should be truncated to 30 bytes total (basename + dot + ext) + ";1"
        assert!(converted.len() <= 32, "L2 name too long: {}", converted.len());
        assert!(converted.as_str().ends_with(";1"));
    }

    #[test]
    fn test_convert_l2_no_extension() {
        let orig = "this-is-a-very-long-directory-name-without-extension";
        let converted = convert_l2(orig, false);
        assert!(converted.len() <= 32, "L2 name too long: {}", converted.len());
        assert!(converted.as_str().ends_with(";1"));
        // First 30 characters + ";1"
        assert_eq!(converted.as_str(), "THIS_IS_A_VERY_LONG_DIRECTORY_;1");
    }

    #[test]
    fn test_convert_l3_short_name() {
        let orig = "readme.txt";
        let converted = convert_l3(orig, false);
        assert_eq!(converted.as_str(), "README.TXT");
    }

    #[test]
    fn test_convert_l3_long_name_truncation() {
        // Max is 207 bytes for L3
        let long_name = "a".repeat(250);
        let converted = convert_l3(&long_name, false);
        assert!(converted.len() <= 207, "L3 name too long: {}", converted.len());
        assert_eq!(converted.len(), 207);
    }

    #[test]
    fn test_convert_l3_with_extension() {
        // Create a name that exceeds 207 bytes with extension
        let basename = "a".repeat(200);
        let orig = format!("{}.txt", basename);
        let converted = convert_l3(&orig, false);
        assert!(converted.len() <= 207, "L3 name too long: {}", converted.len());
    }
}
