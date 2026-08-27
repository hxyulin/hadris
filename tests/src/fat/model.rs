use std::collections::BTreeMap;

use super::{ARCHIVE, LABEL, MUTABLE_ATTRS, READ_ONLY, fat_path_eq};
use crate::harness::split_parent;
use crate::harness::tree::{self, EntryData, fnv1a};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EntryState {
    pub data: EntryData,
    pub attrs: u8,
}

impl EntryState {
    pub fn summary(&self) -> String {
        match &self.data {
            EntryData::Directory => format!("directory attrs={:#04x}", self.attrs),
            EntryData::File(data) => format!(
                "file len={} hash={:#018x} attrs={:#04x}",
                data.len(),
                fnv1a(data),
                self.attrs
            ),
        }
    }
}

/// The deterministic reference model every adapter is compared against.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FsState {
    pub label: String,
    pub entries: BTreeMap<String, EntryState>,
}

impl FsState {
    pub fn empty() -> Self {
        Self {
            label: LABEL.to_string(),
            entries: BTreeMap::new(),
        }
    }

    pub fn apply(&mut self, operation: &Operation) -> Result<(), String> {
        match operation {
            Operation::CreateDir { path } => {
                self.ensure_parent(path)?;
                self.ensure_absent(path)?;
                self.entries.insert(
                    path.clone(),
                    EntryState {
                        data: EntryData::Directory,
                        attrs: 0,
                    },
                );
            }
            Operation::CreateFile { path, data } => {
                self.ensure_parent(path)?;
                self.ensure_absent(path)?;
                self.entries.insert(
                    path.clone(),
                    EntryState {
                        data: EntryData::File(data.clone()),
                        attrs: ARCHIVE,
                    },
                );
            }
            Operation::ReplaceFile { path, data } => {
                let entry = self.file_mut(path)?;
                entry.data = EntryData::File(data.clone());
            }
            Operation::AppendFile { path, data } => {
                let entry = self.file_mut(path)?;
                let EntryData::File(contents) = &mut entry.data else {
                    unreachable!();
                };
                contents.extend_from_slice(data);
            }
            Operation::TruncateFile { path, len } => {
                let entry = self.file_mut(path)?;
                let EntryData::File(contents) = &mut entry.data else {
                    unreachable!();
                };
                if *len > contents.len() {
                    return Err(format!("cannot grow {path} with truncate"));
                }
                contents.truncate(*len);
            }
            Operation::Rename { from, to } => {
                self.ensure_parent(to)?;
                self.ensure_absent(to)?;
                let prefix = format!("{from}/");
                let mut moved = Vec::new();
                for (path, entry) in &self.entries {
                    if path == from || path.starts_with(&prefix) {
                        moved.push((path.clone(), entry.clone()));
                    }
                }
                if moved.is_empty() {
                    return Err(format!("missing rename source {from}"));
                }
                for (path, _) in &moved {
                    self.entries.remove(path);
                }
                for (path, entry) in moved {
                    let suffix = &path[from.len()..];
                    self.entries.insert(format!("{to}{suffix}"), entry);
                }
            }
            Operation::SetAttrs { path, attrs } => {
                let entry = self
                    .entries
                    .get_mut(path)
                    .ok_or_else(|| format!("missing attribute target {path}"))?;
                entry.attrs = attrs & MUTABLE_ATTRS;
            }
            Operation::Delete { path } => {
                let entry = self
                    .entries
                    .get(path)
                    .ok_or_else(|| format!("missing delete target {path}"))?;
                if entry.data.is_directory() {
                    let prefix = format!("{path}/");
                    if self.entries.keys().any(|other| other.starts_with(&prefix)) {
                        return Err(format!("directory is not empty: {path}"));
                    }
                }
                self.entries.remove(path);
            }
        }
        Ok(())
    }

    fn ensure_parent(&self, path: &str) -> Result<(), String> {
        let (parent, _) = split_parent(path)?;
        if parent == "/" {
            return Ok(());
        }
        match self.entries.get(parent) {
            Some(EntryState {
                data: EntryData::Directory,
                ..
            }) => Ok(()),
            Some(_) => Err(format!("parent is not a directory: {parent}")),
            None => Err(format!("missing parent directory: {parent}")),
        }
    }

    fn ensure_absent(&self, path: &str) -> Result<(), String> {
        if self
            .entries
            .keys()
            .any(|existing| fat_path_eq(existing, path))
        {
            Err(format!("path already exists: {path}"))
        } else {
            Ok(())
        }
    }

    fn file_mut(&mut self, path: &str) -> Result<&mut EntryState, String> {
        let entry = self
            .entries
            .get_mut(path)
            .ok_or_else(|| format!("missing file: {path}"))?;
        if entry.data.is_directory() {
            Err(format!("not a file: {path}"))
        } else {
            Ok(entry)
        }
    }

    pub fn directories(&self) -> Vec<String> {
        let mut dirs = vec!["/".to_string()];
        dirs.extend(
            self.entries
                .iter()
                .filter(|(_, entry)| entry.data.is_directory())
                .map(|(path, _)| path.clone()),
        );
        dirs
    }

    pub fn files(&self, writable_only: bool) -> Vec<String> {
        self.entries
            .iter()
            .filter(|(_, entry)| {
                !entry.data.is_directory() && (!writable_only || entry.attrs & READ_ONLY == 0)
            })
            .map(|(path, _)| path.clone())
            .collect()
    }

    pub fn empty_directories(&self) -> Vec<String> {
        self.entries
            .iter()
            .filter(|(path, entry)| {
                let prefix = format!("{path}/");
                entry.data.is_directory()
                    && !self.entries.keys().any(|other| other.starts_with(&prefix))
            })
            .map(|(path, _)| path.clone())
            .collect()
    }
}

/// One mutation in an operation trace. Every adapter must support all of them.
#[derive(Clone, Debug)]
pub enum Operation {
    CreateDir { path: String },
    CreateFile { path: String, data: Vec<u8> },
    ReplaceFile { path: String, data: Vec<u8> },
    AppendFile { path: String, data: Vec<u8> },
    TruncateFile { path: String, len: usize },
    Rename { from: String, to: String },
    SetAttrs { path: String, attrs: u8 },
    Delete { path: String },
}

pub fn summarize_operation(operation: &Operation) -> String {
    match operation {
        Operation::CreateDir { path } => format!("create_dir {path}"),
        Operation::CreateFile { path, data } => {
            format!(
                "create_file {path} len={} hash={:#018x}",
                data.len(),
                fnv1a(data)
            )
        }
        Operation::ReplaceFile { path, data } => {
            format!(
                "replace_file {path} len={} hash={:#018x}",
                data.len(),
                fnv1a(data)
            )
        }
        Operation::AppendFile { path, data } => {
            format!(
                "append_file {path} len={} hash={:#018x}",
                data.len(),
                fnv1a(data)
            )
        }
        Operation::TruncateFile { path, len } => format!("truncate_file {path} len={len}"),
        Operation::Rename { from, to } => format!("rename {from} -> {to}"),
        Operation::SetAttrs { path, attrs } => format!("set_attrs {path} {attrs:#04x}"),
        Operation::Delete { path } => format!("delete {path}"),
    }
}

pub fn format_trace(operations: &[Operation]) -> String {
    operations
        .iter()
        .enumerate()
        .map(|(index, operation)| format!("{index:02}: {}", summarize_operation(operation)))
        .collect::<Vec<_>>()
        .join("\n")
}

pub fn compare_snapshot(context: &str, expected: &FsState, actual: &FsState) -> Result<(), String> {
    if expected == actual {
        return Ok(());
    }
    let mut differences = Vec::new();
    if expected.label != actual.label {
        differences.push(format!(
            "label: expected {:?}, actual {:?}",
            expected.label, actual.label
        ));
    }
    differences.extend(tree::differences(
        &expected.entries,
        &actual.entries,
        EntryState::summary,
    ));
    Err(format!(
        "{context} snapshot mismatch:\n{}",
        differences.join("\n")
    ))
}
