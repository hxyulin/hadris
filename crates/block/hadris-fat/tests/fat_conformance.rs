#![cfg(feature = "write")]

use std::collections::{BTreeMap, VecDeque};
use std::ffi::OsString;
use std::fs::{File, OpenOptions};
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use hadris_fat::format::{FatFormatOptions, FatTypeSelection, FatVolumeFormatter};
use hadris_fat::raw::DirEntryAttrFlags;
use hadris_fat::write::FileWriter;
use hadris_fat::{FatDir, FatType, FatVolume, FatVolumeReadExt, FatVolumeWriteExt};
use tempfile::{Builder as TempBuilder, TempDir};

#[path = "common/fat_peer.rs"]
mod fat_peer;
#[path = "common/fat_spec.rs"]
mod fat_spec;

const LABEL: &str = "HADRISCONF";
const READ_ONLY: u8 = 0x01;
const HIDDEN: u8 = 0x02;
const SYSTEM: u8 = 0x04;
const ARCHIVE: u8 = 0x20;
const MUTABLE_ATTRS: u8 = READ_ONLY | HIDDEN | SYSTEM | ARCHIVE;
const TRACE_LEN: usize = 64;

#[derive(Clone, Copy, Debug)]
struct FatCase {
    name: &'static str,
    size: u64,
    selection: FatTypeSelection,
    expected: FatType,
    mkfs_type: &'static str,
}

const FAT_CASES: [FatCase; 3] = [
    FatCase {
        name: "fat12",
        size: 2 * 1024 * 1024,
        selection: FatTypeSelection::Fat12,
        expected: FatType::Fat12,
        mkfs_type: "12",
    },
    FatCase {
        name: "fat16",
        size: 16 * 1024 * 1024,
        selection: FatTypeSelection::Fat16,
        expected: FatType::Fat16,
        mkfs_type: "16",
    },
    FatCase {
        name: "fat32",
        size: 64 * 1024 * 1024,
        selection: FatTypeSelection::Fat32,
        expected: FatType::Fat32,
        mkfs_type: "32",
    },
];

#[derive(Clone, Debug, PartialEq, Eq)]
enum EntryData {
    Directory,
    File(Vec<u8>),
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct EntryState {
    data: EntryData,
    attrs: u8,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct FsState {
    label: String,
    entries: BTreeMap<String, EntryState>,
}

impl FsState {
    fn empty() -> Self {
        Self {
            label: LABEL.to_string(),
            entries: BTreeMap::new(),
        }
    }

    fn apply(&mut self, operation: &Operation) -> Result<(), String> {
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
                if matches!(entry.data, EntryData::Directory) {
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
        if matches!(entry.data, EntryData::File(_)) {
            Ok(entry)
        } else {
            Err(format!("not a file: {path}"))
        }
    }

    fn directories(&self) -> Vec<String> {
        let mut dirs = vec!["/".to_string()];
        dirs.extend(self.entries.iter().filter_map(|(path, entry)| {
            matches!(entry.data, EntryData::Directory).then(|| path.clone())
        }));
        dirs
    }

    fn files(&self, writable_only: bool) -> Vec<String> {
        self.entries
            .iter()
            .filter_map(|(path, entry)| {
                let is_file = matches!(entry.data, EntryData::File(_));
                let is_writable = entry.attrs & READ_ONLY == 0;
                (is_file && (!writable_only || is_writable)).then(|| path.clone())
            })
            .collect()
    }

    fn empty_directories(&self) -> Vec<String> {
        self.entries
            .iter()
            .filter_map(|(path, entry)| {
                if !matches!(entry.data, EntryData::Directory) {
                    return None;
                }
                let prefix = format!("{path}/");
                (!self.entries.keys().any(|other| other.starts_with(&prefix))).then(|| path.clone())
            })
            .collect()
    }
}

#[derive(Clone, Debug)]
enum Operation {
    CreateDir { path: String },
    CreateFile { path: String, data: Vec<u8> },
    ReplaceFile { path: String, data: Vec<u8> },
    AppendFile { path: String, data: Vec<u8> },
    TruncateFile { path: String, len: usize },
    Rename { from: String, to: String },
    SetAttrs { path: String, attrs: u8 },
    Delete { path: String },
}

trait FatAdapter {
    fn apply(&mut self, operation: &Operation) -> Result<(), String>;
    fn snapshot(&mut self) -> Result<FsState, String>;
}

struct HadrisFatAdapter {
    image: PathBuf,
}

impl HadrisFatAdapter {
    fn new(image: PathBuf) -> Self {
        Self { image }
    }

    fn open(&self) -> Result<FatVolume<File>, String> {
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .open(&self.image)
            .map_err(|error| error.to_string())?;
        FatVolume::open(file).map_err(|error| error.to_string())
    }

    fn put_file(&self, path: &str, contents: &[u8], replace: bool) -> Result<(), String> {
        let volume = self.open()?;
        let (parent_path, name) = split_parent(path)?;
        let parent = open_dir_path(&volume, parent_path)?;
        let entry = if replace {
            let old = parent
                .find(name)
                .map_err(|error| error.to_string())?
                .ok_or_else(|| format!("missing file: {path}"))?;
            volume
                .truncate(&old, 0)
                .map_err(|error| error.to_string())?;
            parent
                .find(name)
                .map_err(|error| error.to_string())?
                .ok_or_else(|| format!("missing truncated file: {path}"))?
        } else {
            volume
                .create_file(&parent, name)
                .map_err(|error| error.to_string())?
        };
        let mut writer = volume
            .write_file(&entry)
            .map_err(|error| error.to_string())?;
        write_all_hadris(&mut writer, contents)?;
        writer.finish().map_err(|error| error.to_string())?;
        volume.sync().map_err(|error| error.to_string())
    }
}

impl FatAdapter for HadrisFatAdapter {
    fn apply(&mut self, operation: &Operation) -> Result<(), String> {
        match operation {
            Operation::CreateDir { path } => {
                let volume = self.open()?;
                let (parent_path, name) = split_parent(path)?;
                let parent = open_dir_path(&volume, parent_path)?;
                volume
                    .create_dir(&parent, name)
                    .map_err(|error| error.to_string())?;
                volume.sync().map_err(|error| error.to_string())
            }
            Operation::CreateFile { path, data } => self.put_file(path, data, false),
            Operation::ReplaceFile { path, data } => self.put_file(path, data, true),
            Operation::AppendFile { path, data } => {
                let volume = self.open()?;
                let (parent_path, name) = split_parent(path)?;
                let parent = open_dir_path(&volume, parent_path)?;
                let entry = parent
                    .find(name)
                    .map_err(|error| error.to_string())?
                    .ok_or_else(|| format!("missing file: {path}"))?;
                let mut writer =
                    FileWriter::new_append(&volume, &entry).map_err(|error| error.to_string())?;
                write_all_hadris(&mut writer, data)?;
                writer.finish().map_err(|error| error.to_string())?;
                volume.sync().map_err(|error| error.to_string())
            }
            Operation::TruncateFile { path, len } => {
                let volume = self.open()?;
                let (parent_path, name) = split_parent(path)?;
                let parent = open_dir_path(&volume, parent_path)?;
                let entry = parent
                    .find(name)
                    .map_err(|error| error.to_string())?
                    .ok_or_else(|| format!("missing file: {path}"))?;
                volume
                    .truncate(&entry, *len)
                    .map_err(|error| error.to_string())?;
                volume.sync().map_err(|error| error.to_string())
            }
            Operation::Rename { from, to } => {
                let volume = self.open()?;
                let (source_parent, source_name) = split_parent(from)?;
                let (dest_parent, dest_name) = split_parent(to)?;
                let source_dir = open_dir_path(&volume, source_parent)?;
                let dest_dir = open_dir_path(&volume, dest_parent)?;
                let entry = source_dir
                    .find(source_name)
                    .map_err(|error| error.to_string())?
                    .ok_or_else(|| format!("missing rename source: {from}"))?;
                volume
                    .rename(&entry, &dest_dir, dest_name)
                    .map_err(|error| error.to_string())?;
                volume.sync().map_err(|error| error.to_string())
            }
            Operation::SetAttrs { path, attrs } => {
                let volume = self.open()?;
                let (parent_path, name) = split_parent(path)?;
                let parent = open_dir_path(&volume, parent_path)?;
                let entry = parent
                    .find(name)
                    .map_err(|error| error.to_string())?
                    .ok_or_else(|| format!("missing attribute target: {path}"))?;
                let immutable = entry.attributes().bits() & !MUTABLE_ATTRS;
                let attributes = DirEntryAttrFlags::from_bits_retain(immutable | attrs);
                volume
                    .set_attributes(&entry, attributes)
                    .map_err(|error| error.to_string())?;
                volume.sync().map_err(|error| error.to_string())
            }
            Operation::Delete { path } => {
                let volume = self.open()?;
                let (parent_path, name) = split_parent(path)?;
                let parent = open_dir_path(&volume, parent_path)?;
                let entry = parent
                    .find(name)
                    .map_err(|error| error.to_string())?
                    .ok_or_else(|| format!("missing delete target: {path}"))?;
                volume.delete(&entry).map_err(|error| error.to_string())?;
                volume.sync().map_err(|error| error.to_string())
            }
        }
    }

    fn snapshot(&mut self) -> Result<FsState, String> {
        let volume = self.open()?;
        let mut state = FsState {
            label: volume.volume_info().volume_label().trim().to_string(),
            entries: BTreeMap::new(),
        };
        snapshot_hadris_dir(&volume, volume.root_dir(), "/", &mut state.entries)?;
        Ok(state)
    }
}

struct MtoolsFatAdapter {
    image: PathBuf,
    config: PathBuf,
    scratch: PathBuf,
    next_file: usize,
}

impl MtoolsFatAdapter {
    fn new(image: PathBuf, workspace: &Path) -> Result<Self, String> {
        let config = workspace.join("mtoolsrc");
        std::fs::write(&config, []).map_err(|error| error.to_string())?;
        let scratch = workspace.join("mtools-inputs");
        std::fs::create_dir_all(&scratch).map_err(|error| error.to_string())?;
        Ok(Self {
            image,
            config,
            scratch,
            next_file: 0,
        })
    }

    fn run(&self, program: &str, args: Vec<OsString>) -> Result<Output, String> {
        run_command(program, args, Some(&self.config))
    }

    fn write_host_file(&mut self, contents: &[u8]) -> Result<PathBuf, String> {
        let path = self
            .scratch
            .join(format!("input-{:05}.bin", self.next_file));
        self.next_file += 1;
        std::fs::write(&path, contents).map_err(|error| error.to_string())?;
        Ok(path)
    }

    fn put_file(
        &mut self,
        path: &str,
        contents: &[u8],
        preserve_attrs: bool,
    ) -> Result<(), String> {
        let attrs = if preserve_attrs {
            self.read_attrs(&[path.to_string()])?.get(path).copied()
        } else {
            None
        };
        let source = self.write_host_file(contents)?;
        self.run(
            "mcopy",
            vec![
                "-i".into(),
                self.image.as_os_str().into(),
                "-D".into(),
                "o".into(),
                source.as_os_str().into(),
                mtools_path(path).into(),
            ],
        )?;
        if let Some(attrs) = attrs {
            self.set_attrs(path, attrs)?;
        }
        Ok(())
    }

    fn set_attrs(&self, path: &str, attrs: u8) -> Result<(), String> {
        let mut args = vec!["-i".into(), self.image.as_os_str().into()];
        for (flag, bit) in [
            ("r", READ_ONLY),
            ("h", HIDDEN),
            ("s", SYSTEM),
            ("a", ARCHIVE),
        ] {
            args.push(format!("{}{flag}", if attrs & bit == 0 { "-" } else { "+" }).into());
        }
        args.push(mtools_path(path).into());
        self.run("mattrib", args)?;
        Ok(())
    }

    fn read_file(&self, path: &str) -> Result<Vec<u8>, String> {
        Ok(self
            .run(
                "mtype",
                vec![
                    "-i".into(),
                    self.image.as_os_str().into(),
                    mtools_path(path).into(),
                ],
            )?
            .stdout)
    }

    fn list_dir(&self, path: &str) -> Result<Vec<(String, bool)>, String> {
        let pattern = if path == "/" {
            "::*".to_string()
        } else {
            format!("::{}/*", path.trim_end_matches('/'))
        };
        let args = vec![
            "-i".into(),
            self.image.as_os_str().into(),
            "-a".into(),
            "-b".into(),
            pattern.into(),
        ];
        let output = match self.run("mdir", args) {
            Ok(output) => output,
            Err(error) if error.to_ascii_lowercase().contains("not found") => return Ok(Vec::new()),
            Err(error) => return Err(error),
        };
        String::from_utf8(output.stdout)
            .map_err(|error| error.to_string())?
            .lines()
            .map(|line| {
                let value = line
                    .strip_prefix("::")
                    .ok_or_else(|| format!("unexpected mdir output: {line:?}"))?;
                let directory = value.ends_with('/');
                let path = value.trim_end_matches('/');
                Ok((normalize_path(path), directory))
            })
            .collect()
    }

    fn read_attrs(&self, paths: &[String]) -> Result<BTreeMap<String, u8>, String> {
        if paths.is_empty() {
            return Ok(BTreeMap::new());
        }
        let mut args = vec!["-i".into(), self.image.as_os_str().into()];
        args.extend(paths.iter().map(|path| OsString::from(mtools_path(path))));
        let output = self.run("mattrib", args)?;
        let text = String::from_utf8(output.stdout).map_err(|error| error.to_string())?;
        let mut attrs = BTreeMap::new();
        for line in text.lines() {
            let marker = line
                .find("::")
                .ok_or_else(|| format!("unexpected mattrib output: {line:?}"))?;
            let prefix = &line[..marker];
            let path = normalize_path(line[marker + 2..].trim());
            let mut bits = 0;
            for byte in prefix.bytes() {
                bits |= match byte.to_ascii_uppercase() {
                    b'A' => ARCHIVE,
                    b'H' => HIDDEN,
                    b'R' => READ_ONLY,
                    b'S' => SYSTEM,
                    _ => 0,
                };
            }
            attrs.insert(path, bits);
        }
        Ok(attrs)
    }

    fn read_label(&self) -> Result<String, String> {
        let output = self.run(
            "mlabel",
            vec![
                "-i".into(),
                self.image.as_os_str().into(),
                "-s".into(),
                "::".into(),
            ],
        )?;
        let text = String::from_utf8(output.stdout).map_err(|error| error.to_string())?;
        let label = text
            .lines()
            .find_map(|line| {
                line.split_once("Volume label is")
                    .map(|(_, label)| label.trim())
            })
            .ok_or_else(|| format!("unexpected mlabel output: {text:?}"))?;
        Ok(label.to_string())
    }
}

impl FatAdapter for MtoolsFatAdapter {
    fn apply(&mut self, operation: &Operation) -> Result<(), String> {
        match operation {
            Operation::CreateDir { path } => {
                self.run(
                    "mmd",
                    vec![
                        "-i".into(),
                        self.image.as_os_str().into(),
                        mtools_path(path).into(),
                    ],
                )?;
                Ok(())
            }
            Operation::CreateFile { path, data } => self.put_file(path, data, false),
            Operation::ReplaceFile { path, data } => self.put_file(path, data, true),
            Operation::AppendFile { path, data } => {
                let mut contents = self.read_file(path)?;
                contents.extend_from_slice(data);
                self.put_file(path, &contents, true)
            }
            Operation::TruncateFile { path, len } => {
                let mut contents = self.read_file(path)?;
                contents.truncate(*len);
                self.put_file(path, &contents, true)
            }
            Operation::Rename { from, to } => {
                self.run(
                    "mren",
                    vec![
                        "-i".into(),
                        self.image.as_os_str().into(),
                        mtools_path(from).into(),
                        mtools_path(to).into(),
                    ],
                )?;
                Ok(())
            }
            Operation::SetAttrs { path, attrs } => self.set_attrs(path, *attrs),
            Operation::Delete { path } => {
                let listing = self.snapshot()?;
                let entry = listing
                    .entries
                    .get(path)
                    .ok_or_else(|| format!("missing delete target: {path}"))?;
                let program = if matches!(entry.data, EntryData::Directory) {
                    "mrd"
                } else {
                    "mdel"
                };
                self.run(
                    program,
                    vec![
                        "-i".into(),
                        self.image.as_os_str().into(),
                        mtools_path(path).into(),
                    ],
                )?;
                Ok(())
            }
        }
    }

    fn snapshot(&mut self) -> Result<FsState, String> {
        let mut entries = BTreeMap::new();
        let mut dirs = VecDeque::from(["/".to_string()]);
        while let Some(dir) = dirs.pop_front() {
            for (path, is_directory) in self.list_dir(&dir)? {
                if entries.contains_key(&path) {
                    continue;
                }
                let data = if is_directory {
                    dirs.push_back(path.clone());
                    EntryData::Directory
                } else {
                    EntryData::File(self.read_file(&path)?)
                };
                entries.insert(path, EntryState { data, attrs: 0 });
            }
        }
        let paths: Vec<String> = entries.keys().cloned().collect();
        let attrs = self.read_attrs(&paths)?;
        for (path, entry) in &mut entries {
            entry.attrs = attrs.get(path).copied().unwrap_or(0) & MUTABLE_ATTRS;
        }
        Ok(FsState {
            label: self.read_label()?,
            entries,
        })
    }
}

fn open_dir_path<'a>(volume: &'a FatVolume<File>, path: &str) -> Result<FatDir<'a, File>, String> {
    let mut dir = volume.root_dir();
    for component in path.split('/').filter(|part| !part.is_empty()) {
        dir = dir.open_dir(component).map_err(|error| error.to_string())?;
    }
    Ok(dir)
}

fn write_all_hadris(writer: &mut FileWriter<'_, File>, mut contents: &[u8]) -> Result<(), String> {
    while !contents.is_empty() {
        let written = writer.write(contents).map_err(|error| error.to_string())?;
        if written == 0 {
            return Err("file writer made no progress".to_string());
        }
        contents = &contents[written..];
    }
    Ok(())
}

fn snapshot_hadris_dir(
    volume: &FatVolume<File>,
    dir: FatDir<'_, File>,
    path: &str,
    entries: &mut BTreeMap<String, EntryState>,
) -> Result<(), String> {
    let children = dir
        .entries()
        .map(|entry| {
            entry
                .map_err(|error| error.to_string())?
                .as_entry()
                .cloned()
                .ok_or_else(|| "unexpected directory entry variant".to_string())
        })
        .collect::<Result<Vec<_>, _>>()?;
    for entry in children {
        let name = entry.name().into_owned();
        if name == "." || name == ".." {
            continue;
        }
        let child_path = join_path(path, &name);
        let attrs = entry.attributes().bits() & MUTABLE_ATTRS;
        let data = if entry.is_directory() {
            EntryData::Directory
        } else {
            let mut reader = volume
                .read_file(&entry)
                .map_err(|error| error.to_string())?;
            let mut contents = Vec::with_capacity(entry.len() as usize);
            let mut buffer = [0_u8; 4096];
            loop {
                let count = reader
                    .read(&mut buffer)
                    .map_err(|error| error.to_string())?;
                if count == 0 {
                    break;
                }
                contents.extend_from_slice(&buffer[..count]);
            }
            EntryData::File(contents)
        };
        entries.insert(child_path.clone(), EntryState { data, attrs });
        if entry.is_directory() {
            let child = dir.open_entry(&entry).map_err(|error| error.to_string())?;
            snapshot_hadris_dir(volume, child, &child_path, entries)?;
        }
    }
    Ok(())
}

fn split_parent(path: &str) -> Result<(&str, &str), String> {
    let index = path
        .rfind('/')
        .ok_or_else(|| format!("path is not absolute: {path}"))?;
    let parent = if index == 0 { "/" } else { &path[..index] };
    let name = &path[index + 1..];
    if name.is_empty() {
        Err(format!("path has no final component: {path}"))
    } else {
        Ok((parent, name))
    }
}

fn join_path(parent: &str, name: &str) -> String {
    if parent == "/" {
        format!("/{name}")
    } else {
        format!("{parent}/{name}")
    }
}

fn normalize_path(path: &str) -> String {
    let trimmed = path.trim();
    if trimmed.starts_with('/') {
        trimmed.to_string()
    } else {
        format!("/{trimmed}")
    }
}

fn mtools_path(path: &str) -> String {
    format!("::{path}")
}

fn fat_path_eq(left: &str, right: &str) -> bool {
    left.eq_ignore_ascii_case(right)
}

fn run_command(
    program: &str,
    args: Vec<OsString>,
    mtools_config: Option<&Path>,
) -> Result<Output, String> {
    let printable = args
        .iter()
        .map(|arg| arg.to_string_lossy())
        .collect::<Vec<_>>()
        .join(" ");
    let mut command = Command::new(program);
    command.args(&args).env("LC_ALL", "C.UTF-8");
    if let Some(config) = mtools_config {
        command.env("MTOOLSRC", config);
    }
    let output = command
        .output()
        .map_err(|error| format!("failed to run {program} {printable}: {error}"))?;
    if output.status.success() {
        Ok(output)
    } else {
        Err(format!(
            "{program} {printable} failed with {:?}\nstdout:\n{}\nstderr:\n{}",
            output.status.code(),
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        ))
    }
}

fn format_hadris(path: &Path, case: FatCase) -> Result<(), String> {
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(true)
        .open(path)
        .map_err(|error| error.to_string())?;
    file.set_len(case.size).map_err(|error| error.to_string())?;
    let options = FatFormatOptions::new(case.size)
        .fat_type(case.selection)
        .volume_label(LABEL)
        .volume_id(0x4841_4452);
    let volume = FatVolumeFormatter::format(file, options).map_err(|error| error.to_string())?;
    if volume.fat_type() != case.expected {
        return Err(format!(
            "{} formatted as {:?}, expected {:?}",
            case.name,
            volume.fat_type(),
            case.expected
        ));
    }
    drop(volume);
    Ok(())
}

fn format_mkfs(path: &Path, case: FatCase) -> Result<(), String> {
    let file = File::create(path).map_err(|error| error.to_string())?;
    file.set_len(case.size).map_err(|error| error.to_string())?;
    drop(file);
    run_command(
        "mkfs.fat",
        vec![
            "-F".into(),
            case.mkfs_type.into(),
            "-n".into(),
            LABEL.into(),
            path.as_os_str().into(),
        ],
        None,
    )?;
    Ok(())
}

fn fsck(path: &Path) -> Result<(), String> {
    run_command("fsck.fat", vec!["-n".into(), path.as_os_str().into()], None)?;
    Ok(())
}

fn curated_operations() -> Vec<Operation> {
    vec![
        Operation::CreateDir {
            path: "/Empty".into(),
        },
        Operation::CreateDir {
            path: "/Nested".into(),
        },
        Operation::CreateDir {
            path: "/Nested/Deep".into(),
        },
        Operation::CreateFile {
            path: "/README.TXT".into(),
            data: Vec::new(),
        },
        Operation::CreateFile {
            path: "/lower.txt".into(),
            data: b"lowercase short name".to_vec(),
        },
        Operation::CreateFile {
            path: "/.hidden".into(),
            data: b"leading dot filename".to_vec(),
        },
        Operation::CreateFile {
            path: "/Mixed Case Name.bin".into(),
            data: payload(513, 0x11),
        },
        Operation::CreateFile {
            path: "/Nested/boundary.bin".into(),
            data: payload(4097, 0x22),
        },
        Operation::CreateFile {
            path: "/Nested/Deep/日本語.txt".into(),
            data: "Unicode filename contents\n".as_bytes().to_vec(),
        },
        Operation::AppendFile {
            path: "/lower.txt".into(),
            data: b" appended".to_vec(),
        },
        Operation::ReplaceFile {
            path: "/Mixed Case Name.bin".into(),
            data: payload(8193, 0x33),
        },
        Operation::TruncateFile {
            path: "/Nested/boundary.bin".into(),
            len: 512,
        },
        Operation::Rename {
            from: "/Nested/Deep/日本語.txt".into(),
            to: "/Nested/資料 renamed.txt".into(),
        },
        Operation::CreateFile {
            path: "/Delete Me.txt".into(),
            data: payload(31, 0x44),
        },
        Operation::Delete {
            path: "/Delete Me.txt".into(),
        },
        Operation::CreateFile {
            path: "/Slot Reuse Long Name.txt".into(),
            data: vec![0, 1, 0, 2, 0, 3],
        },
        Operation::SetAttrs {
            path: "/lower.txt".into(),
            attrs: ARCHIVE | READ_ONLY | HIDDEN,
        },
        Operation::SetAttrs {
            path: "/Slot Reuse Long Name.txt".into(),
            attrs: ARCHIVE | SYSTEM,
        },
        Operation::CreateDir {
            path: "/Move Source".into(),
        },
        Operation::Rename {
            from: "/Move Source".into(),
            to: "/Nested/Moved Empty Directory".into(),
        },
    ]
}

fn edge_case_scenarios() -> Vec<(String, Vec<Operation>)> {
    let deleted_slot_lfn_expansion = vec![
        Operation::CreateDir {
            path: "/D000".into(),
        },
        Operation::CreateDir {
            path: "/TEMP".into(),
        },
        Operation::Delete {
            path: "/TEMP".into(),
        },
        Operation::Rename {
            from: "/D000".into(),
            to: "/Renamed Directory 0009".into(),
        },
    ];

    let mut subdirectory_entry_boundary = vec![Operation::CreateDir {
        path: "/Entries".into(),
    }];
    subdirectory_entry_boundary.extend((0..15).map(|index| Operation::CreateFile {
        path: format!("/Entries/F{index:02}.TXT"),
        data: vec![index as u8],
    }));
    subdirectory_entry_boundary.extend([
        Operation::Delete {
            path: "/Entries/F04.TXT".into(),
        },
        Operation::Delete {
            path: "/Entries/F05.TXT".into(),
        },
        Operation::CreateFile {
            path: "/Entries/Long Name Reusing Adjacent Slots.txt".into(),
            data: payload(513, 0x41),
        },
    ]);

    let short_alias_collisions = ["x+.txt", "x,.txt", "x=.txt", "x;.txt", "x'.txt", "x].txt"]
        .into_iter()
        .enumerate()
        .map(|(index, name)| Operation::CreateFile {
            path: format!("/{name}"),
            data: vec![index as u8],
        })
        .collect();

    let directory_moves = vec![
        Operation::CreateDir {
            path: "/Parent A".into(),
        },
        Operation::CreateDir {
            path: "/Parent B".into(),
        },
        Operation::CreateDir {
            path: "/Parent A/Child".into(),
        },
        Operation::CreateDir {
            path: "/Parent A/Child/Deep".into(),
        },
        Operation::Rename {
            from: "/Parent A/Child".into(),
            to: "/Parent B/Moved Child".into(),
        },
        Operation::Rename {
            from: "/Parent B/Moved Child".into(),
            to: "/Moved Again".into(),
        },
    ];

    let truncate_reallocate = vec![
        Operation::CreateFile {
            path: "/CHAIN.BIN".into(),
            data: payload(16_385, 0x52),
        },
        Operation::TruncateFile {
            path: "/CHAIN.BIN".into(),
            len: 8_192,
        },
        Operation::TruncateFile {
            path: "/CHAIN.BIN".into(),
            len: 4_097,
        },
        Operation::TruncateFile {
            path: "/CHAIN.BIN".into(),
            len: 4_096,
        },
        Operation::TruncateFile {
            path: "/CHAIN.BIN".into(),
            len: 0,
        },
        Operation::AppendFile {
            path: "/CHAIN.BIN".into(),
            data: payload(513, 0x53),
        },
    ];

    let lfn_boundaries = [
        "123456789.txt",
        "1234567890.txt",
        "1234567890123456789012.txt",
        "12345678901234567890123.txt",
        "日本語.txt",
    ]
    .into_iter()
    .enumerate()
    .map(|(index, name)| Operation::CreateFile {
        path: format!("/{name}"),
        data: vec![index as u8],
    })
    .collect();

    vec![
        (
            "deleted-slot-lfn-expansion".into(),
            deleted_slot_lfn_expansion,
        ),
        (
            "subdirectory-entry-boundary".into(),
            subdirectory_entry_boundary,
        ),
        ("short-alias-collisions".into(), short_alias_collisions),
        ("directory-moves".into(), directory_moves),
        ("truncate-reallocate".into(), truncate_reallocate),
        ("lfn-boundaries".into(), lfn_boundaries),
    ]
}

fn payload(len: usize, salt: u8) -> Vec<u8> {
    (0..len)
        .map(|index| (index as u8).wrapping_mul(31).wrapping_add(salt))
        .collect()
}

struct Rng(u64);

impl Rng {
    fn new(seed: u64) -> Self {
        Self(seed.max(1))
    }

    fn next(&mut self) -> u64 {
        let mut value = self.0;
        value ^= value << 13;
        value ^= value >> 7;
        value ^= value << 17;
        self.0 = value;
        value
    }

    fn index(&mut self, len: usize) -> usize {
        (self.next() as usize) % len
    }
}

fn generate_trace(seed: u64) -> Vec<Operation> {
    let mut rng = Rng::new(seed);
    let mut model = FsState::empty();
    let mut operations = Vec::with_capacity(TRACE_LEN);
    for index in 0..TRACE_LEN {
        let dirs = model.directories();
        let writable_files = model.files(true);
        let empty_dirs = model.empty_directories();
        let can_create = model.entries.len() < 48;
        let roll = rng.index(100);
        let operation = if can_create && (model.entries.len() < 4 || roll < 22) {
            let parents: Vec<_> = dirs
                .iter()
                .filter(|path| path_depth(path) < 4)
                .cloned()
                .collect();
            let parent = &parents[rng.index(parents.len())];
            Operation::CreateDir {
                path: join_path(parent, &generated_name(&mut rng, index, true)),
            }
        } else if can_create && (writable_files.is_empty() || roll < 45) {
            let parents: Vec<_> = dirs
                .iter()
                .filter(|path| path_depth(path) < 4)
                .cloned()
                .collect();
            let parent = &parents[rng.index(parents.len())];
            Operation::CreateFile {
                path: join_path(parent, &generated_name(&mut rng, index, false)),
                data: generated_payload(&mut rng),
            }
        } else if !writable_files.is_empty() && roll < 58 {
            Operation::ReplaceFile {
                path: writable_files[rng.index(writable_files.len())].clone(),
                data: generated_payload(&mut rng),
            }
        } else if !writable_files.is_empty() && roll < 68 {
            Operation::AppendFile {
                path: writable_files[rng.index(writable_files.len())].clone(),
                data: payload([1, 31, 512, 513][rng.index(4)], rng.next() as u8),
            }
        } else if !writable_files.is_empty() && roll < 76 {
            let path = writable_files[rng.index(writable_files.len())].clone();
            let len = match &model.entries[&path].data {
                EntryData::File(data) => rng.index(data.len() + 1),
                EntryData::Directory => unreachable!(),
            };
            Operation::TruncateFile { path, len }
        } else if !model.entries.is_empty() && roll < 87 {
            generate_rename(&mut rng, &model, index).unwrap_or_else(|| {
                let parent = &dirs[rng.index(dirs.len())];
                Operation::CreateFile {
                    path: join_path(parent, &generated_name(&mut rng, index, false)),
                    data: generated_payload(&mut rng),
                }
            })
        } else if !writable_files.is_empty() && roll < 94 {
            Operation::SetAttrs {
                path: writable_files[rng.index(writable_files.len())].clone(),
                attrs: ARCHIVE
                    | if rng.next() & 1 == 0 { HIDDEN } else { 0 }
                    | if rng.next() & 1 == 0 { SYSTEM } else { 0 }
                    | if rng.next() % 5 == 0 { READ_ONLY } else { 0 },
            }
        } else if !writable_files.is_empty() {
            Operation::Delete {
                path: writable_files[rng.index(writable_files.len())].clone(),
            }
        } else if !empty_dirs.is_empty() {
            Operation::Delete {
                path: empty_dirs[rng.index(empty_dirs.len())].clone(),
            }
        } else {
            let parent = &dirs[rng.index(dirs.len())];
            Operation::CreateFile {
                path: join_path(parent, &generated_name(&mut rng, index, false)),
                data: generated_payload(&mut rng),
            }
        };
        model
            .apply(&operation)
            .expect("generated operation is valid");
        operations.push(operation);
    }
    operations
}

fn generate_rename(rng: &mut Rng, model: &FsState, index: usize) -> Option<Operation> {
    let sources: Vec<_> = model
        .entries
        .iter()
        .filter(|(_, entry)| entry.attrs & READ_ONLY == 0)
        .map(|(path, _)| path.clone())
        .collect();
    if sources.is_empty() {
        return None;
    }
    let from = sources[rng.index(sources.len())].clone();
    let source_is_dir = matches!(model.entries[&from].data, EntryData::Directory);
    let prefix = format!("{from}/");
    let destinations: Vec<_> = model
        .directories()
        .into_iter()
        .filter(|path| path != &from && !path.starts_with(&prefix) && path_depth(path) < 4)
        .collect();
    if destinations.is_empty() {
        return None;
    }
    let parent = &destinations[rng.index(destinations.len())];
    Some(Operation::Rename {
        from,
        to: join_path(
            parent,
            &if source_is_dir {
                format!("Renamed Directory {index:04}")
            } else {
                format!("Renamed File {index:04}.bin")
            },
        ),
    })
}

fn generated_name(rng: &mut Rng, index: usize, directory: bool) -> String {
    let suffix = if directory { "" } else { ".bin" };
    match rng.index(4) {
        0 => format!("{}{:03}{suffix}", if directory { "D" } else { "F" }, index),
        1 => format!(
            "{}{:03}{suffix}",
            if directory { "dir" } else { "file" },
            index
        ),
        2 => format!(
            "{} {index:03} Name{suffix}",
            if directory { "Directory" } else { "Mixed" }
        ),
        _ => format!("資料{index:03}{suffix}"),
    }
}

fn generated_payload(rng: &mut Rng) -> Vec<u8> {
    let lengths = [0, 1, 31, 511, 512, 513, 4095, 4096, 4097, 8193, 32769];
    payload(lengths[rng.index(lengths.len())], rng.next() as u8)
}

fn path_depth(path: &str) -> usize {
    path.split('/').filter(|part| !part.is_empty()).count()
}

struct Workspace {
    _temp: Option<TempDir>,
    path: PathBuf,
}

impl Workspace {
    fn new(prefix: &str) -> Result<Self, String> {
        let keep = std::env::var_os("HADRIS_FAT_CONFORMANCE_KEEP").is_some();
        if keep {
            let root =
                PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../target/fat-conformance");
            std::fs::create_dir_all(&root).map_err(|error| error.to_string())?;
            let temp = TempBuilder::new()
                .prefix(prefix)
                .tempdir_in(root)
                .map_err(|error| error.to_string())?;
            let path = temp.keep();
            eprintln!("retaining FAT conformance artifacts at {}", path.display());
            Ok(Self { _temp: None, path })
        } else {
            let temp = TempBuilder::new()
                .prefix(prefix)
                .tempdir()
                .map_err(|error| error.to_string())?;
            let path = temp.path().to_path_buf();
            Ok(Self {
                _temp: Some(temp),
                path,
            })
        }
    }
}

fn apply_operations(
    adapter: &mut dyn FatAdapter,
    operations: &[Operation],
) -> Result<FsState, String> {
    let mut expected = FsState::empty();
    for (index, operation) in operations.iter().enumerate() {
        adapter.apply(operation).map_err(|error| {
            format!(
                "operation {index} failed: {}\n{error}\ntrace:\n{}",
                summarize_operation(operation),
                format_trace(&operations[..=index])
            )
        })?;
        expected.apply(operation).map_err(|error| {
            format!(
                "model rejected operation {index}: {}: {error}",
                summarize_operation(operation)
            )
        })?;
    }
    Ok(expected)
}

fn summarize_operation(operation: &Operation) -> String {
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

fn format_trace(operations: &[Operation]) -> String {
    operations
        .iter()
        .enumerate()
        .map(|(index, operation)| format!("{index:02}: {}", summarize_operation(operation)))
        .collect::<Vec<_>>()
        .join("\n")
}

fn compare_snapshot(context: &str, expected: &FsState, actual: &FsState) -> Result<(), String> {
    if expected == actual {
        Ok(())
    } else {
        let mut differences = Vec::new();
        if expected.label != actual.label {
            differences.push(format!(
                "label: expected {:?}, actual {:?}",
                expected.label, actual.label
            ));
        }
        for path in expected
            .entries
            .keys()
            .chain(actual.entries.keys())
            .collect::<std::collections::BTreeSet<_>>()
        {
            let expected_entry = expected.entries.get(path.as_str());
            let actual_entry = actual.entries.get(path.as_str());
            if expected_entry != actual_entry {
                differences.push(format!(
                    "{path}: expected {}, actual {}",
                    summarize_entry(expected_entry),
                    summarize_entry(actual_entry)
                ));
            }
        }
        Err(format!(
            "{context} snapshot mismatch:\n{}",
            differences.join("\n")
        ))
    }
}

fn summarize_entry(entry: Option<&EntryState>) -> String {
    match entry {
        None => "<missing>".to_string(),
        Some(EntryState {
            data: EntryData::Directory,
            attrs,
        }) => format!("directory attrs={attrs:#04x}"),
        Some(EntryState {
            data: EntryData::File(data),
            attrs,
        }) => format!(
            "file len={} hash={:#018x} attrs={attrs:#04x}",
            data.len(),
            fnv1a(data)
        ),
    }
}

fn fnv1a(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |hash, byte| {
        (hash ^ u64::from(*byte)).wrapping_mul(0x0000_0100_0000_01b3)
    })
}

fn require_tools() -> Result<(), String> {
    for program in [
        "mkfs.fat", "fsck.fat", "mdir", "mcopy", "mtype", "mmd", "mrd", "mdel", "mren", "mattrib",
        "mlabel",
    ] {
        Command::new(program)
            .arg("--help")
            .env("LC_ALL", "C.UTF-8")
            .output()
            .map_err(|error| format!("required external tool {program} is unavailable: {error}"))?;
    }
    Ok(())
}

fn run_spec_matrix(operations: &[Operation]) -> Result<(), String> {
    for case in FAT_CASES {
        let workspace = Workspace::new(&format!("{}-spec-", case.name))?;
        let image = workspace.path.join(format!("{}-hadris.img", case.name));
        format_hadris(&image, case)?;
        let expected = apply_operations(&mut HadrisFatAdapter::new(image.clone()), operations)?;
        let oracle = fat_spec::snapshot(&image, case.mkfs_type.parse().unwrap())?;
        compare_snapshot(
            &format!("{} FAT specification oracle", case.name),
            &expected,
            &oracle,
        )?;
        let hadris = HadrisFatAdapter::new(image).snapshot()?;
        compare_snapshot(&format!("{} Hadris reader", case.name), &expected, &hadris)?;
    }
    Ok(())
}

#[derive(Default)]
struct FatfsAccuracy {
    reads_passed: usize,
    reads_attempted: usize,
    writes_passed: usize,
    writes_attempted: usize,
    details: Vec<String>,
}

impl FatfsAccuracy {
    fn report(&self) -> String {
        let passed = self.reads_passed + self.writes_passed;
        let attempted = self.reads_attempted + self.writes_attempted;
        let percent = if attempted == 0 {
            0.0
        } else {
            passed as f64 * 100.0 / attempted as f64
        };
        format!(
            "rust-fatfs semantic accuracy: {passed}/{attempted} ({percent:.2}%)\n\
             rust-fatfs reading Hadris: {}/{}\n\
             rust-fatfs writing spec-valid images: {}/{}\n{}",
            self.reads_passed,
            self.reads_attempted,
            self.writes_passed,
            self.writes_attempted,
            self.details.join("\n")
        )
    }
}

fn measure_fatfs(
    case: FatCase,
    scenario: &str,
    operations: &[Operation],
    accuracy: &mut FatfsAccuracy,
) -> Result<(), String> {
    let workspace = Workspace::new(&format!("{}-{scenario}-fatfs-", case.name))?;
    let hadris_image = workspace.path.join("hadris.img");
    format_hadris(&hadris_image, case)?;
    let expected = apply_operations(&mut HadrisFatAdapter::new(hadris_image.clone()), operations)?;
    let oracle = fat_spec::snapshot(&hadris_image, case.mkfs_type.parse().unwrap())?;
    compare_snapshot(
        "Hadris image before rust-fatfs measurement",
        &expected,
        &oracle,
    )?;

    accuracy.reads_attempted += 1;
    match catch_peer_panic(|| {
        fat_peer::snapshot(&hadris_image).and_then(|snapshot| {
            compare_snapshot(
                &format!("{} rust-fatfs reading Hadris", case.name),
                &expected,
                &snapshot,
            )
        })
    }) {
        Ok(()) => accuracy.reads_passed += 1,
        Err(error) => accuracy.details.push(format!(
            "{} {scenario} rust-fatfs read mismatch: {error}",
            case.name
        )),
    }

    accuracy.writes_attempted += 1;
    let fatfs_image = workspace.path.join("fatfs.img");
    let writer_result = catch_peer_panic(|| {
        fat_peer::format(&fatfs_image, case)?;
        let mut expected = apply_operations_without_attrs(
            &mut fat_peer::FatfsAdapter::new(fatfs_image.clone()),
            operations,
        )?;
        let mut oracle = fat_spec::snapshot(&fatfs_image, case.mkfs_type.parse().unwrap())?;
        fat_peer::clear_mutable_attrs(&mut expected);
        fat_peer::clear_mutable_attrs(&mut oracle);
        compare_snapshot(
            &format!("{} FAT specification oracle reading rust-fatfs", case.name),
            &expected,
            &oracle,
        )?;
        let mut hadris = HadrisFatAdapter::new(fatfs_image).snapshot()?;
        fat_peer::clear_mutable_attrs(&mut hadris);
        compare_snapshot(
            &format!("{} Hadris reading rust-fatfs", case.name),
            &expected,
            &hadris,
        )
    });
    match writer_result {
        Ok(()) => accuracy.writes_passed += 1,
        Err(error) => accuracy.details.push(format!(
            "{} {scenario} rust-fatfs write mismatch: {error}",
            case.name
        )),
    }
    Ok(())
}

fn catch_peer_panic<T>(operation: impl FnOnce() -> Result<T, String>) -> Result<T, String> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(operation)).map_err(|payload| {
        let message = payload
            .downcast_ref::<&str>()
            .copied()
            .or_else(|| payload.downcast_ref::<String>().map(String::as_str))
            .unwrap_or("unknown panic payload");
        format!("rust-fatfs panicked: {message}")
    })?
}

fn apply_operations_without_attrs(
    adapter: &mut dyn FatAdapter,
    operations: &[Operation],
) -> Result<FsState, String> {
    let mut expected = FsState::empty();
    for (index, operation) in operations.iter().enumerate() {
        adapter.apply(operation).map_err(|error| {
            format!(
                "operation {index} failed: {}\n{error}\ntrace:\n{}",
                summarize_operation(operation),
                format_trace(&operations[..=index])
            )
        })?;
        if !matches!(operation, Operation::SetAttrs { .. }) {
            expected.apply(operation)?;
        }
    }
    Ok(expected)
}

fn run_native_tools_matrix(operations: &[Operation]) -> Result<(), String> {
    fat_peer::native_tools()?;
    for case in FAT_CASES {
        let workspace = Workspace::new(&format!("{}-native-", case.name))?;

        let hadris_image = workspace.path.join("hadris.img");
        format_hadris(&hadris_image, case)?;
        let expected =
            apply_operations(&mut HadrisFatAdapter::new(hadris_image.clone()), operations)?;
        fat_peer::native_fsck(&hadris_image)?;
        compare_snapshot(
            &format!("{} native checker input", case.name),
            &expected,
            &fat_peer::snapshot(&hadris_image)?,
        )?;

        let native_image = workspace.path.join("native.img");
        fat_peer::format_native(&native_image, case)?;
        let expected =
            apply_operations(&mut HadrisFatAdapter::new(native_image.clone()), operations)?;
        fat_peer::native_fsck(&native_image)?;
        let oracle = fat_spec::snapshot(&native_image, case.mkfs_type.parse().unwrap())?;
        compare_snapshot(
            &format!("{} specification oracle reading native format", case.name),
            &expected,
            &oracle,
        )?;
        compare_snapshot(
            &format!("{} rust-fatfs reading native format", case.name),
            &expected,
            &fat_peer::snapshot(&native_image)?,
        )?;
    }
    Ok(())
}

fn run_native_mount_matrix() -> Result<(), String> {
    if std::env::var_os("HADRIS_FAT_NATIVE_MOUNT").is_none() {
        eprintln!("set HADRIS_FAT_NATIVE_MOUNT=1 to enable native kernel mount tests");
        return Ok(());
    }
    for case in FAT_CASES {
        let workspace = Workspace::new(&format!("{}-mount-", case.name))?;
        let image = workspace.path.join("native-mount.img");
        format_hadris(&image, case)?;
        let base = vec![
            Operation::CreateDir {
                path: "/Hadris Source".into(),
            },
            Operation::CreateFile {
                path: "/Hadris Source/Original.txt".into(),
                data: b"written by Hadris".to_vec(),
            },
        ];
        let mut expected = apply_operations(&mut HadrisFatAdapter::new(image.clone()), &base)?;
        let mountpoint = workspace.path.join("mount");
        let mount = fat_peer::NativeMount::mount(&image, mountpoint)?;
        let original = mount.path().join("Hadris Source/Original.txt");
        if std::fs::read(&original).map_err(|error| error.to_string())? != b"written by Hadris" {
            return Err(format!(
                "{} native mount read incorrect contents",
                case.name
            ));
        }
        let native_dir = mount.path().join("Kernel Directory");
        std::fs::create_dir(&native_dir).map_err(|error| error.to_string())?;
        let temporary = mount.path().join("Temporary.txt");
        std::fs::write(&temporary, b"delete me").map_err(|error| error.to_string())?;
        let renamed = native_dir.join("Renamed by Kernel.txt");
        std::fs::write(&renamed, format!("native-{}", case.name))
            .map_err(|error| error.to_string())?;
        std::fs::remove_file(temporary).map_err(|error| error.to_string())?;
        mount.unmount()?;

        expected.apply(&Operation::CreateDir {
            path: "/Kernel Directory".into(),
        })?;
        expected.apply(&Operation::CreateFile {
            path: "/Kernel Directory/Renamed by Kernel.txt".into(),
            data: format!("native-{}", case.name).into_bytes(),
        })?;
        fat_peer::native_fsck(&image)?;
        let mut oracle = fat_spec::snapshot(&image, case.mkfs_type.parse().unwrap())?;
        fat_peer::clear_mutable_attrs(&mut expected);
        fat_peer::clear_mutable_attrs(&mut oracle);
        fat_peer::remove_native_metadata(&mut oracle);
        compare_snapshot(
            &format!("{} specification oracle reading native mount", case.name),
            &expected,
            &oracle,
        )?;
        let mut hadris = HadrisFatAdapter::new(image).snapshot()?;
        fat_peer::clear_mutable_attrs(&mut hadris);
        fat_peer::remove_native_metadata(&mut hadris);
        compare_snapshot(
            &format!("{} Hadris reading native mount", case.name),
            &expected,
            &hadris,
        )?;
    }
    Ok(())
}

#[derive(Default)]
struct Accuracy {
    mtools_reads_passed: usize,
    mtools_reads_attempted: usize,
    mtools_writes_passed: usize,
    mtools_writes_attempted: usize,
    fsck_hadris_passed: usize,
    fsck_hadris_attempted: usize,
    fsck_mtools_passed: usize,
    fsck_mtools_attempted: usize,
    command_failures: usize,
    details: Vec<String>,
}

impl Accuracy {
    fn report(&self) -> String {
        let total_passed = self.mtools_reads_passed + self.mtools_writes_passed;
        let total_attempted = self.mtools_reads_attempted + self.mtools_writes_attempted;
        let percent = if total_attempted == 0 {
            0.0
        } else {
            total_passed as f64 * 100.0 / total_attempted as f64
        };
        format!(
            "mtools semantic accuracy: {total_passed}/{total_attempted} ({percent:.2}%)\n\
             mtools reading Hadris: {}/{}\n\
             mtools writing spec-valid images: {}/{}\n\
             fsck.fat accepting Hadris images: {}/{}\n\
             fsck.fat accepting mtools images: {}/{}\n\
             external-tool command failures: {}\n{}",
            self.mtools_reads_passed,
            self.mtools_reads_attempted,
            self.mtools_writes_passed,
            self.mtools_writes_attempted,
            self.fsck_hadris_passed,
            self.fsck_hadris_attempted,
            self.fsck_mtools_passed,
            self.fsck_mtools_attempted,
            self.command_failures,
            self.details.join("\n")
        )
    }
}

fn selected_seeds() -> Vec<u64> {
    match std::env::var("HADRIS_FAT_CONFORMANCE_SEED") {
        Ok(seed) => vec![seed.parse::<u64>().expect("seed must be a u64")],
        Err(_) => vec![
            0x0000_0000_0000_0001,
            0x243f_6a88_85a3_08d3,
            0x1319_8a2e_0370_7344,
            0xa409_3822_299f_31d0,
            0x082e_fa98_ec4e_6c89,
            0x4528_21e6_38d0_1377,
            0xbe54_66cf_34e9_0c6c,
            0xc0ac_29b7_c97c_50dd,
            0x3f84_d5b5_b547_0917,
            0x9216_d5d9_8979_fb1b,
            0xd131_0ba6_98df_b5ac,
            0x2ffd_72db_d01a_dfb7,
            0xb8e1_afed_6a26_7e96,
            0xba7c_9045_f12c_7f99,
            0x24a1_9947_b391_6cf7,
            0x0801_f2e2_858e_fc16,
        ],
    }
}

fn interoperability_scenarios() -> Vec<(String, Vec<Operation>)> {
    let mut scenarios = vec![("curated".to_string(), curated_operations())];
    scenarios.extend(edge_case_scenarios());
    scenarios
}

fn specification_scenarios() -> Vec<(String, Vec<Operation>)> {
    let mut scenarios = interoperability_scenarios();
    scenarios.extend(
        selected_seeds()
            .into_iter()
            .map(|seed| (format!("seed-{seed:016x}"), generate_trace(seed))),
    );
    scenarios
}

fn measure_external_tools(
    case: FatCase,
    scenario: &str,
    operations: &[Operation],
    accuracy: &mut Accuracy,
) -> Result<(), String> {
    let hadris_workspace = Workspace::new(&format!("{}-{scenario}-hadris-", case.name))?;
    let hadris_image = hadris_workspace.path.join("hadris.img");
    format_hadris(&hadris_image, case)?;
    let expected = apply_operations(&mut HadrisFatAdapter::new(hadris_image.clone()), operations)?;
    let oracle = fat_spec::snapshot(&hadris_image, case.mkfs_type.parse().unwrap())?;
    compare_snapshot(
        "Hadris image before external measurement",
        &expected,
        &oracle,
    )?;

    accuracy.mtools_reads_attempted += 1;
    match MtoolsFatAdapter::new(hadris_image.clone(), &hadris_workspace.path)
        .and_then(|mut adapter| adapter.snapshot())
        .and_then(|snapshot| compare_snapshot("mtools reader", &expected, &snapshot))
    {
        Ok(()) => accuracy.mtools_reads_passed += 1,
        Err(error) => accuracy.details.push(format!(
            "{} {scenario} mtools read mismatch: {error}",
            case.name
        )),
    }
    accuracy.fsck_hadris_attempted += 1;
    if let Err(error) = fsck(&hadris_image) {
        accuracy.details.push(format!(
            "{} {scenario} fsck rejected Hadris image: {error}",
            case.name
        ));
    } else {
        accuracy.fsck_hadris_passed += 1;
    }

    accuracy.mtools_writes_attempted += 1;
    let mtools_workspace = Workspace::new(&format!("{}-{scenario}-mtools-", case.name))?;
    let mtools_image = mtools_workspace.path.join("mtools.img");
    if let Err(error) = format_mkfs(&mtools_image, case) {
        accuracy.command_failures += 1;
        accuracy
            .details
            .push(format!("{} {scenario} mkfs.fat failed: {error}", case.name));
        return Ok(());
    }
    let mut adapter = MtoolsFatAdapter::new(mtools_image.clone(), &mtools_workspace.path)?;
    let mut written_model = FsState::empty();
    for (index, operation) in operations.iter().enumerate() {
        if let Err(error) = adapter.apply(operation) {
            accuracy.command_failures += 1;
            accuracy.details.push(format!(
                "{} {scenario} mtools operation {index} failed ({}): {error}\ntrace:\n{}",
                case.name,
                summarize_operation(operation),
                format_trace(&operations[..=index])
            ));
            return Ok(());
        }
        written_model.apply(operation)?;
    }
    match fat_spec::snapshot(&mtools_image, case.mkfs_type.parse().unwrap())
        .and_then(|snapshot| compare_snapshot("mtools writer", &written_model, &snapshot))
    {
        Ok(()) => {
            accuracy.mtools_writes_passed += 1;
            let hadris = HadrisFatAdapter::new(mtools_image.clone()).snapshot()?;
            compare_snapshot(
                "Hadris reading spec-valid mtools image",
                &written_model,
                &hadris,
            )?;
        }
        Err(error) => accuracy.details.push(format!(
            "{} {scenario} mtools write mismatch: {error}",
            case.name
        )),
    }
    accuracy.fsck_mtools_attempted += 1;
    if fsck(&mtools_image).is_ok() {
        accuracy.fsck_mtools_passed += 1;
    }
    Ok(())
}

#[test]
#[ignore = "manual FAT specification conformance suite"]
fn fat_spec_conformance() {
    for (scenario, operations) in specification_scenarios() {
        run_spec_matrix(&operations).unwrap_or_else(|error| {
            panic!("{scenario}: {error}\ntrace:\n{}", format_trace(&operations))
        });
    }
}

#[test]
fn fat_edge_cases_match_spec() {
    for (scenario, operations) in edge_case_scenarios() {
        run_spec_matrix(&operations).unwrap_or_else(|error| {
            panic!("{scenario}: {error}\ntrace:\n{}", format_trace(&operations))
        });
    }
}

#[test]
#[ignore = "manual bidirectional rust-fatfs accuracy suite"]
fn fatfs_accuracy_report() {
    let mut accuracy = FatfsAccuracy::default();
    for (scenario, operations) in interoperability_scenarios() {
        for case in FAT_CASES {
            measure_fatfs(case, &scenario, &operations, &mut accuracy).unwrap_or_else(|error| {
                panic!(
                    "{} {scenario}: {error}\ntrace:\n{}",
                    case.name,
                    format_trace(&operations)
                )
            });
        }
    }
    let report = accuracy.report();
    eprintln!("{report}");
    let report_dir =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../target/fat-conformance");
    std::fs::create_dir_all(&report_dir).unwrap();
    std::fs::write(report_dir.join("fatfs-accuracy.txt"), format!("{report}\n")).unwrap();
}

#[test]
fn fatfs_dot_entries_match_spec() {
    let operations = vec![
        Operation::CreateDir {
            path: "/Empty".into(),
        },
        Operation::CreateDir {
            path: "/Nested".into(),
        },
        Operation::CreateDir {
            path: "/Nested/Deep".into(),
        },
    ];
    let mut accuracy = FatfsAccuracy::default();
    for case in FAT_CASES {
        measure_fatfs(case, "dot-entries", &operations, &mut accuracy).unwrap();
    }
    assert_eq!(
        accuracy.writes_passed,
        accuracy.writes_attempted,
        "{}",
        accuracy.report()
    );
}

#[test]
#[ignore = "requires native Linux or macOS FAT formatter and checker"]
fn native_platform_tools() {
    let operations = curated_operations();
    run_native_tools_matrix(&operations)
        .unwrap_or_else(|error| panic!("{error}\ntrace:\n{}", format_trace(&operations)));
}

#[test]
#[ignore = "requires HADRIS_FAT_NATIVE_MOUNT=1 and native mount privileges"]
fn native_mount_roundtrip() {
    run_native_mount_matrix().unwrap();
}

#[test]
#[ignore = "requires mtools and dosfstools; run through nix develop"]
fn mtools_accuracy_report() {
    require_tools().unwrap();
    let mut accuracy = Accuracy::default();
    for (scenario, operations) in interoperability_scenarios() {
        for case in FAT_CASES {
            measure_external_tools(case, &scenario, &operations, &mut accuracy).unwrap_or_else(
                |error| {
                    panic!(
                        "{} {scenario}: {error}\ntrace:\n{}",
                        case.name,
                        format_trace(&operations)
                    )
                },
            );
        }
    }
    let report = accuracy.report();
    eprintln!("{report}");
    let report_dir =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../target/fat-conformance");
    std::fs::create_dir_all(&report_dir).unwrap();
    std::fs::write(
        report_dir.join("mtools-accuracy.txt"),
        format!("{report}\n"),
    )
    .unwrap();
}
