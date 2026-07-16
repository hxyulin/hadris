use super::super::io::{self, Write};
use alloc::{collections::BTreeMap, collections::VecDeque, string::String, sync::Arc, vec::Vec};
use hadris_common::types::endian::EndianType;
use hadris_path::{Component, Separators, VPath};

use super::super::io::LogicalSector;
use super::super::path::PathTableEntryHeader;
use super::super::read::PathSeparator;
use crate::file::EntryType;
#[cfg(test)]
use crate::file::{convert_joliet3, convert_l1, convert_l2, convert_l3};

use super::super::directory::DirectoryRef;
use super::{InputEntryKind, InputMetadata};

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
/// Represents DirectoryId.
pub struct DirectoryId {
    indices: Vec<usize>,
}

impl DirectoryId {
    /// Performs the `push` operation.
    pub fn push(&mut self, index: usize) {
        self.indices.push(index);
    }

    /// Performs the `pop` operation.
    pub fn pop(&mut self) -> usize {
        self.indices.pop().expect("directory underflow")
    }
}

#[derive(Debug)]
/// Represents WrittenFiles.
pub struct WrittenFiles {
    root: WrittenDirectory,
}

impl Default for WrittenFiles {
    fn default() -> Self {
        Self::new()
    }
}

impl WrittenFiles {
    /// Performs the `new` operation.
    pub fn new() -> Self {
        Self {
            root: WrittenDirectory::new(Arc::new(String::new())),
        }
    }

    /// Performs the `find_file` operation.
    pub fn find_file(&self, name: &str, _sep: PathSeparator) -> Option<DirectoryRef> {
        let mut current_dir = DirectoryId {
            indices: Vec::new(),
        };
        let mut parts = VPath::with_separators(name, Separators::SlashOrBackslash)
            .components()
            .filter_map(|component| match component {
                Component::Root | Component::Current => None,
                Component::Parent => Some(None),
                Component::Normal(component) => Some(Some(component)),
            })
            .peekable();
        'parts: while let Some(part) = parts.next() {
            let part = part?;
            if parts.peek().is_none() {
                let dir = self.get(&current_dir);
                return dir
                    .files
                    .iter()
                    .find(|file| file.name.as_str() == part)
                    .map(|file| file.entry);
            }
            let dir = self.get(&current_dir);
            for (idx, dir) in dir.dirs.iter().enumerate() {
                if dir.name.as_str() == part {
                    current_dir.push(idx);
                    continue 'parts;
                }
            }
            return None;
        }
        None
    }

    /// Performs the `root_dir` operation.
    pub fn root_dir(&self) -> DirectoryId {
        DirectoryId {
            indices: Vec::new(),
        }
    }

    /// Performs the `root_refs` operation.
    pub fn root_refs(&self) -> &BTreeMap<EntryType, DirectoryRef> {
        &self.root.entries
    }

    /// Performs the `get` operation.
    pub fn get(&self, id: &DirectoryId) -> &WrittenDirectory {
        let mut dir = &self.root;
        for index in &id.indices {
            dir = &dir.dirs[*index];
        }
        dir
    }

    /// Performs the `get_mut` operation.
    pub fn get_mut(&mut self, id: &DirectoryId) -> &mut WrittenDirectory {
        let mut dir = &mut self.root;
        for index in &id.indices {
            dir = &mut dir.dirs[*index];
        }
        dir
    }
}

#[derive(Debug)]
/// Represents WrittenDirectory.
pub struct WrittenDirectory {
    pub(crate) id: usize,
    /// The `name` field.
    pub name: Arc<String>,
    pub(crate) rrip_name: Arc<String>,
    /// The `entries` field.
    pub entries: BTreeMap<EntryType, DirectoryRef>,
    /// The `dirs` field.
    pub dirs: Vec<WrittenDirectory>,
    /// The `files` field.
    pub files: Vec<WrittenFile>,
    /// The `metadata` field.
    pub metadata: InputMetadata,
    pub(crate) relocation: DirectoryRelocation,
}

#[derive(Debug, Clone, Copy, Default)]
pub(crate) enum DirectoryRelocation {
    #[default]
    None,
    Placeholder {
        target: usize,
    },
    Moved {
        id: usize,
        logical_parent: usize,
    },
}

impl WrittenDirectory {
    /// Performs the `new` operation.
    pub fn new(name: Arc<String>) -> Self {
        Self {
            id: 0,
            rrip_name: name.clone(),
            name,
            entries: BTreeMap::new(),
            dirs: Vec::new(),
            files: Vec::new(),
            metadata: InputMetadata::default(),
            relocation: DirectoryRelocation::None,
        }
    }

    /// Performs the `push_dir` operation.
    pub fn push_dir(&mut self, name: Arc<String>, metadata: InputMetadata) -> usize {
        let mut directory = Self::new(name);
        directory.metadata = metadata;
        self.dirs.push(directory);
        self.dirs.len() - 1
    }
}

#[derive(Debug)]
/// Represents WrittenFile.
pub struct WrittenFile {
    /// The `name` field.
    pub name: Arc<String>,
    /// The `entry` field.
    pub entry: DirectoryRef,
    /// The `kind` field.
    pub kind: InputEntryKind,
    /// The `metadata` field.
    pub metadata: InputMetadata,
}

pub(crate) struct PathTableWriter<'a> {
    pub written_files: &'a WrittenFiles,
    pub ty: EntryType,
    pub endian: EndianType,
}

io_transform! {

/// Write a single path table record.
async fn write_pt_record<DATA: Write>(
    data: &mut DATA,
    endian: &EndianType,
    parent_number: u16,
    extent: LogicalSector,
    name: &[u8],
) -> io::Result<()> {
    let header = PathTableEntryHeader {
        len: name.len() as u8,
        extended_attr_record: 0,
        parent_directory_number: endian.u16_bytes(parent_number),
        parent_lba: endian.u32_bytes(extent.0 as u32),
    };
    data.write_all(bytemuck::bytes_of(&header)).await?;
    data.write_all(name).await?;
    if !name.len().is_multiple_of(2) {
        data.write_all(&[0x00]).await?; // padding to even
    }
    Ok(())
}

impl PathTableWriter<'_> {
    pub async fn write<DATA: Write>(&mut self, data: &mut DATA) -> io::Result<()> {
        // BFS queue: (directory_ref, parent_number)
        // ISO 9660 requires path table entries in breadth-first order.
        let mut queue: VecDeque<(&WrittenDirectory, u16)> = VecDeque::new();
        let mut current_number: u16 = 1;

        // Root entry (parent = 1, i.e. itself)
        let root = &self.written_files.root;
        let root_extent = *root.entries.get(&self.ty).unwrap();
        write_pt_record(data, &self.endian, 1, root_extent.extent, &[0x00]).await?;
        queue.push_back((root, 1));

        while let Some((dir, parent_num)) = queue.pop_front() {
            let my_number = parent_num;
            for child_dir in &dir.dirs {
                if matches!(
                    child_dir.relocation,
                    DirectoryRelocation::Placeholder { .. }
                ) {
                    continue;
                }
                current_number += 1;
                let name = self.ty.convert_name(&child_dir.name);
                let name_bytes = name.as_bytes();
                let extent = child_dir.entries.get(&self.ty).unwrap().extent;
                write_pt_record(data, &self.endian, my_number, extent, name_bytes).await?;
                queue.push_back((child_dir, current_number));
            }
        }
        Ok(())
    }
}

} // io_transform!

#[cfg(all(test, feature = "std"))]
mod tests {
    use super::*;
    use alloc::format;

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
        assert!(
            converted.len() <= 32,
            "L2 name too long: {}",
            converted.len()
        );
        assert!(converted.as_str().ends_with(";1"));
    }

    #[test]
    fn test_convert_l2_no_extension() {
        let orig = "this-is-a-very-long-directory-name-without-extension";
        let converted = convert_l2(orig, false);
        assert!(
            converted.len() <= 32,
            "L2 name too long: {}",
            converted.len()
        );
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
        assert!(
            converted.len() <= 207,
            "L3 name too long: {}",
            converted.len()
        );
        assert_eq!(converted.len(), 207);
    }

    #[test]
    fn test_convert_l3_with_extension() {
        // Create a name that exceeds 207 bytes with extension
        let basename = "a".repeat(200);
        let orig = format!("{basename}.txt");
        let converted = convert_l3(&orig, false);
        assert!(
            converted.len() <= 207,
            "L3 name too long: {}",
            converted.len()
        );
    }

    // Edge-case tests for convert_l1

    #[test]
    fn test_convert_l1_empty_extension() {
        let converted = convert_l1("file.", false);
        assert_eq!(converted.as_str(), "FILE.;1");
    }

    #[test]
    fn test_convert_l1_dot_only() {
        let converted = convert_l1(".", false);
        assert_eq!(converted.as_str(), ".;1");
    }

    #[test]
    fn test_convert_l1_dot_dot() {
        // ".." → basename empty, dot, ext "." substituted to "_"
        let converted = convert_l1("..", false);
        assert_eq!(converted.as_str(), "._;1");
    }

    #[test]
    fn test_convert_l1_no_dot() {
        let converted = convert_l1("README", false);
        assert_eq!(converted.as_str(), "README;1");
    }

    #[test]
    fn test_convert_l1_no_dot_long() {
        let converted = convert_l1("LONGFILENAME", false);
        assert_eq!(converted.as_str(), "LONGFILE;1");
    }

    #[test]
    fn test_convert_l1_exact_8_3() {
        let converted = convert_l1("12345678.abc", false);
        assert_eq!(converted.as_str(), "12345678.ABC;1");
    }

    #[test]
    fn test_convert_l1_oversized() {
        let converted = convert_l1("longname1.longext", false);
        assert_eq!(converted.as_str(), "LONGNAME.LON;1");
    }

    #[test]
    fn test_convert_l1_single_char() {
        let converted = convert_l1("a.b", false);
        assert_eq!(converted.as_str(), "A.B;1");
    }

    #[test]
    fn test_convert_l1_multibyte_utf8() {
        // "café.txt" — 'é' is 2 bytes in UTF-8, basename "café" = 5 bytes
        let converted = convert_l1("café.txt", false);
        // Should not panic; multi-byte chars get substituted by CharsetD
        assert!(converted.len() <= 14, "L1 overflow: {}", converted.len());
        assert!(converted.as_str().ends_with(";1"));
    }

    // Edge-case tests for convert_l2

    #[test]
    fn test_convert_l2_empty_extension() {
        let converted = convert_l2("file.", false);
        assert_eq!(converted.as_str(), "FILE.;1");
    }

    #[test]
    fn test_convert_l2_no_dot() {
        let converted = convert_l2("README", false);
        assert_eq!(converted.as_str(), "README;1");
    }

    #[test]
    fn test_convert_l2_single_char() {
        let converted = convert_l2("a.b", false);
        assert_eq!(converted.as_str(), "A.B;1");
    }

    // Edge-case tests for convert_l3

    #[test]
    fn test_convert_l3_empty_extension() {
        let converted = convert_l3("file.", false);
        assert_eq!(converted.as_str(), "FILE.");
    }

    #[test]
    fn test_convert_l3_no_dot() {
        let converted = convert_l3("README", false);
        assert_eq!(converted.as_str(), "README");
    }

    #[test]
    fn test_convert_l3_single_char() {
        let converted = convert_l3("a.b", false);
        assert_eq!(converted.as_str(), "A.B");
    }

    // Edge-case tests for convert_joliet3

    #[test]
    fn test_convert_joliet3_short_name() {
        let converted = convert_joliet3("readme.txt");
        // UTF-16 BE: each char is 2 bytes, "readme.txt" = 10 chars = 20 bytes
        assert_eq!(converted.len(), 20);
    }

    #[test]
    fn test_convert_joliet3_long_name_truncation() {
        // 207 bytes / 2 = 103 UTF-16 code units max
        let long_name = "a".repeat(150);
        let converted = convert_joliet3(&long_name);
        // 103 code units * 2 bytes = 206 bytes
        assert!(
            converted.len() <= 207,
            "Joliet overflow: {}",
            converted.len()
        );
        assert_eq!(converted.len(), 206);
    }

    #[test]
    fn test_convert_joliet3_multibyte_utf8() {
        // "café.txt" — 'é' is one UTF-16 code unit, 8 code units total
        let converted = convert_joliet3("café.txt");
        // 8 code units * 2 bytes = 16 bytes
        assert_eq!(converted.len(), 16);
    }
}
