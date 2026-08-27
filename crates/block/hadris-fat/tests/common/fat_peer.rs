use std::collections::BTreeMap;
use std::ffi::OsString;
use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use fatfs::{
    DefaultTimeProvider, Dir, FileSystem, FormatVolumeOptions, FsOptions, LossyOemCpConverter,
    StdIoWrapper,
};

use super::{
    EntryData, EntryState, FatAdapter, FatCase, FsState, LABEL, MUTABLE_ATTRS, Operation,
    join_path, run_command,
};

pub(super) struct FatfsAdapter {
    image: PathBuf,
}

impl FatfsAdapter {
    pub(super) fn new(image: PathBuf) -> Self {
        Self { image }
    }

    fn open(&self) -> Result<FileSystem<StdIoWrapper<File>>, String> {
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .open(&self.image)
            .map_err(|error| error.to_string())?;
        FileSystem::new(file, FsOptions::new()).map_err(|error| error.to_string())
    }
}

impl FatAdapter for FatfsAdapter {
    fn apply(&mut self, operation: &Operation) -> Result<(), String> {
        let fs = self.open()?;
        let root = fs.root_dir();
        match operation {
            Operation::CreateDir { path } => {
                root.create_dir(relative(path))
                    .map_err(|error| error.to_string())?;
            }
            Operation::CreateFile { path, data } => {
                let mut file = root
                    .create_file(relative(path))
                    .map_err(|error| error.to_string())?;
                file.write_all(data).map_err(|error| error.to_string())?;
            }
            Operation::ReplaceFile { path, data } => {
                let mut file = root
                    .open_file(relative(path))
                    .map_err(|error| error.to_string())?;
                file.seek(SeekFrom::Start(0))
                    .map_err(|error| error.to_string())?;
                file.truncate().map_err(|error| error.to_string())?;
                file.write_all(data).map_err(|error| error.to_string())?;
            }
            Operation::AppendFile { path, data } => {
                let mut file = root
                    .open_file(relative(path))
                    .map_err(|error| error.to_string())?;
                file.seek(SeekFrom::End(0))
                    .map_err(|error| error.to_string())?;
                file.write_all(data).map_err(|error| error.to_string())?;
            }
            Operation::TruncateFile { path, len } => {
                let mut file = root
                    .open_file(relative(path))
                    .map_err(|error| error.to_string())?;
                file.seek(SeekFrom::Start(*len as u64))
                    .map_err(|error| error.to_string())?;
                file.truncate().map_err(|error| error.to_string())?;
            }
            Operation::Rename { from, to } => root
                .rename(relative(from), &root, relative(to))
                .map_err(|error| error.to_string())?,
            Operation::SetAttrs { .. } => {}
            Operation::Delete { path } => root
                .remove(relative(path))
                .map_err(|error| error.to_string())?,
        }
        drop(root);
        fs.unmount().map_err(|error| error.to_string())
    }

    fn snapshot(&mut self) -> Result<FsState, String> {
        snapshot(&self.image)
    }
}

pub(super) fn snapshot(path: &Path) -> Result<FsState, String> {
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .open(path)
        .map_err(|error| error.to_string())?;
    let fs = FileSystem::new(file, FsOptions::new()).map_err(|error| error.to_string())?;
    let mut state = FsState {
        label: fs.volume_label().trim().to_string(),
        entries: BTreeMap::new(),
    };
    snapshot_dir(&fs.root_dir(), "/", &mut state.entries)?;
    Ok(state)
}

fn snapshot_dir(
    dir: &Dir<'_, StdIoWrapper<File>, DefaultTimeProvider, LossyOemCpConverter>,
    path: &str,
    entries: &mut BTreeMap<String, EntryState>,
) -> Result<(), String> {
    let children = dir
        .iter()
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;
    for entry in children {
        let name = entry.file_name();
        if name == "." || name == ".." {
            continue;
        }
        let child_path = join_path(path, &name);
        let attrs = entry.attributes().bits() & MUTABLE_ATTRS;
        let data = if entry.is_dir() {
            EntryData::Directory
        } else {
            let mut contents = Vec::with_capacity(entry.len() as usize);
            entry
                .to_file()
                .read_to_end(&mut contents)
                .map_err(|error| error.to_string())?;
            EntryData::File(contents)
        };
        entries.insert(child_path.clone(), EntryState { data, attrs });
        if entry.is_dir() {
            snapshot_dir(&entry.to_dir(), &child_path, entries)?;
        }
    }
    Ok(())
}

pub(super) fn format(path: &Path, case: FatCase) -> Result<(), String> {
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(true)
        .open(path)
        .map_err(|error| error.to_string())?;
    file.set_len(case.size).map_err(|error| error.to_string())?;
    let fat_type = match case.mkfs_type {
        "12" => fatfs::FatType::Fat12,
        "16" => fatfs::FatType::Fat16,
        "32" => fatfs::FatType::Fat32,
        other => return Err(format!("unsupported FAT width {other}")),
    };
    let mut storage = StdIoWrapper::new(file);
    fatfs::format_volume(
        &mut storage,
        FormatVolumeOptions::new()
            .fat_type(fat_type)
            .volume_label(*b"HADRISCONF "),
    )
    .map_err(|error| error.to_string())
}

pub(super) fn clear_mutable_attrs(state: &mut FsState) {
    for entry in state.entries.values_mut() {
        entry.attrs = 0;
    }
}

pub(super) fn remove_native_metadata(state: &mut FsState) {
    if cfg!(target_os = "macos") {
        state.entries.retain(|path, _| {
            !path
                .rsplit('/')
                .next()
                .is_some_and(|name| name.starts_with("._"))
        });
    }
}

pub(super) fn native_tools() -> Result<(&'static str, &'static str), String> {
    let tools = if cfg!(target_os = "linux") {
        ("mkfs.fat", "fsck.fat")
    } else if cfg!(target_os = "macos") {
        ("/sbin/newfs_msdos", "/sbin/fsck_msdos")
    } else {
        return Err("native FAT tools are supported on Linux and macOS".to_string());
    };
    for tool in [tools.0, tools.1] {
        std::process::Command::new(tool)
            .output()
            .map_err(|error| format!("required native FAT tool {tool} is unavailable: {error}"))?;
    }
    if cfg!(target_os = "macos") {
        std::process::Command::new("hdiutil")
            .output()
            .map_err(|error| format!("required native tool hdiutil is unavailable: {error}"))?;
    }
    Ok(tools)
}

pub(super) fn format_native(path: &Path, case: FatCase) -> Result<(), String> {
    let (formatter, _) = native_tools()?;
    let file = File::create(path).map_err(|error| error.to_string())?;
    file.set_len(case.size).map_err(|error| error.to_string())?;
    drop(file);
    if cfg!(target_os = "linux") {
        run_command(
            formatter,
            vec![
                "-F".into(),
                case.mkfs_type.into(),
                "-n".into(),
                LABEL.into(),
                path.as_os_str().into(),
            ],
            None,
        )?;
        return Ok(());
    }
    let attach = run_command(
        "hdiutil",
        vec![
            "attach".into(),
            "-nomount".into(),
            "-imagekey".into(),
            "diskimage-class=CRawDiskImage".into(),
            path.as_os_str().into(),
        ],
        None,
    )?;
    let text = String::from_utf8(attach.stdout).map_err(|error| error.to_string())?;
    let device = text
        .lines()
        .find_map(|line| {
            line.split_whitespace()
                .find(|field| field.starts_with("/dev/"))
        })
        .ok_or_else(|| format!("hdiutil did not report an attached device: {text:?}"))?;
    let format_result = run_command(
        formatter,
        vec![
            "-F".into(),
            case.mkfs_type.into(),
            "-v".into(),
            LABEL.into(),
            device.into(),
        ],
        None,
    );
    let detach_result = run_command("hdiutil", vec!["detach".into(), device.into()], None);
    match (format_result, detach_result) {
        (Err(error), _) => Err(error),
        (Ok(_), Err(error)) => Err(error),
        (Ok(_), Ok(_)) => Ok(()),
    }
}

pub(super) fn native_fsck(path: &Path) -> Result<(), String> {
    let (_, checker) = native_tools()?;
    run_command(checker, vec!["-n".into(), path.as_os_str().into()], None)?;
    Ok(())
}

pub(super) struct NativeMount {
    mountpoint: PathBuf,
    command: &'static str,
    prefix: Vec<OsString>,
}

impl NativeMount {
    pub(super) fn mount(image: &Path, mountpoint: PathBuf) -> Result<Self, String> {
        std::fs::create_dir_all(&mountpoint).map_err(|error| error.to_string())?;
        if cfg!(target_os = "linux") {
            let uid = command_value("id", "-u")?;
            let gid = command_value("id", "-g")?;
            let root = uid == "0";
            let (command, mut prefix) = if root {
                ("mount", Vec::new())
            } else {
                ("sudo", vec![OsString::from("-n"), OsString::from("mount")])
            };
            prefix.extend([
                "-t".into(),
                "vfat".into(),
                "-o".into(),
                format!("loop,uid={uid},gid={gid}").into(),
                image.as_os_str().into(),
                mountpoint.as_os_str().into(),
            ]);
            run_command(command, prefix, None)?;
            Ok(Self {
                mountpoint,
                command: if root { "umount" } else { "sudo" },
                prefix: if root {
                    Vec::new()
                } else {
                    vec!["-n".into(), "umount".into()]
                },
            })
        } else if cfg!(target_os = "macos") {
            run_command(
                "hdiutil",
                vec![
                    "attach".into(),
                    "-nobrowse".into(),
                    "-imagekey".into(),
                    "diskimage-class=CRawDiskImage".into(),
                    "-owners".into(),
                    "off".into(),
                    "-mountpoint".into(),
                    mountpoint.as_os_str().into(),
                    image.as_os_str().into(),
                ],
                None,
            )?;
            Ok(Self {
                mountpoint,
                command: "hdiutil",
                prefix: vec!["detach".into()],
            })
        } else {
            Err("native FAT mounts are supported on Linux and macOS".to_string())
        }
    }

    pub(super) fn path(&self) -> &Path {
        &self.mountpoint
    }

    pub(super) fn unmount(mut self) -> Result<(), String> {
        self.run_unmount()?;
        self.command = "";
        Ok(())
    }

    fn run_unmount(&self) -> Result<(), String> {
        let mut args = self.prefix.clone();
        args.push(self.mountpoint.as_os_str().into());
        run_command(self.command, args, None)?;
        Ok(())
    }
}

impl Drop for NativeMount {
    fn drop(&mut self) {
        if !self.command.is_empty() {
            let _ = self.run_unmount();
        }
    }
}

fn command_value(program: &str, arg: &str) -> Result<String, String> {
    let output = run_command(program, vec![arg.into()], None)?;
    String::from_utf8(output.stdout)
        .map(|value| value.trim().to_string())
        .map_err(|error| error.to_string())
}

fn relative(path: &str) -> &str {
    path.trim_start_matches('/')
}
