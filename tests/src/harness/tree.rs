use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;

/// The format-agnostic content of one filesystem entry.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EntryData {
    Directory,
    File(Vec<u8>),
}

impl EntryData {
    pub fn is_directory(&self) -> bool {
        matches!(self, EntryData::Directory)
    }

    pub fn summary(&self) -> String {
        match self {
            EntryData::Directory => "directory".to_string(),
            EntryData::File(contents) => {
                format!("file len={} hash={:#018x}", contents.len(), fnv1a(contents))
            }
        }
    }
}

pub fn fnv1a(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |hash, byte| {
        (hash ^ u64::from(*byte)).wrapping_mul(0x0000_0100_0000_01b3)
    })
}

pub fn summarize<T>(entry: Option<&T>, describe: impl Fn(&T) -> String) -> String {
    match entry {
        None => "<missing>".to_string(),
        Some(entry) => describe(entry),
    }
}

/// Lists every path whose expected and actual entries differ.
pub fn differences<T: PartialEq>(
    expected: &BTreeMap<String, T>,
    actual: &BTreeMap<String, T>,
    describe: impl Fn(&T) -> String,
) -> Vec<String> {
    let mut differences = Vec::new();
    for path in expected
        .keys()
        .chain(actual.keys())
        .collect::<BTreeSet<_>>()
    {
        let expected_entry = expected.get(path.as_str());
        let actual_entry = actual.get(path.as_str());
        if expected_entry != actual_entry {
            differences.push(format!(
                "{path}: expected {}, actual {}",
                summarize(expected_entry, &describe),
                summarize(actual_entry, &describe)
            ));
        }
    }
    differences
}

/// Materializes a tree on the host filesystem, for tools that build images
/// from a source directory.
pub fn write_host_tree(root: &Path, entries: &BTreeMap<String, EntryData>) -> Result<(), String> {
    fs::create_dir_all(root).map_err(|error| error.to_string())?;
    for (path, data) in entries {
        let destination = root.join(path.trim_start_matches('/'));
        match data {
            EntryData::Directory => fs::create_dir_all(&destination),
            EntryData::File(contents) => {
                if let Some(parent) = destination.parent() {
                    fs::create_dir_all(parent).map_err(|error| error.to_string())?;
                }
                fs::write(destination, contents)
            }
        }
        .map_err(|error| error.to_string())?;
    }
    Ok(())
}

/// Reads a host directory back into the semantic tree model, for tools that
/// extract or mount images onto the host filesystem.
pub fn snapshot_host(root: &Path) -> Result<BTreeMap<String, EntryData>, String> {
    fn visit(
        root: &Path,
        current: &Path,
        entries: &mut BTreeMap<String, EntryData>,
    ) -> Result<(), String> {
        let mut children = fs::read_dir(current)
            .map_err(|error| error.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| error.to_string())?;
        children.sort_by_key(|entry| entry.file_name());
        for child in children {
            let path = child.path();
            let relative = path.strip_prefix(root).map_err(|error| error.to_string())?;
            let display = format!("/{}", relative.to_string_lossy().replace('\\', "/"));
            if path.is_dir() {
                entries.insert(display, EntryData::Directory);
                visit(root, &path, entries)?;
            } else if path.is_file() {
                entries.insert(
                    display,
                    EntryData::File(fs::read(path).map_err(|error| error.to_string())?),
                );
            }
        }
        Ok(())
    }

    let mut entries = BTreeMap::new();
    visit(root, root, &mut entries)?;
    Ok(entries)
}
