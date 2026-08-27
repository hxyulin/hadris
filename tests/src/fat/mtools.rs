use std::collections::{BTreeMap, VecDeque};
use std::ffi::OsString;
use std::fs::File;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use super::adapter::FatAdapter;
use super::model::{EntryState, FsState, Operation};
use super::{ARCHIVE, FatCase, HIDDEN, LABEL, MUTABLE_ATTRS, READ_ONLY, SYSTEM};
use crate::harness::command::run_command_with_env;
use crate::harness::normalize_path;
use crate::harness::run_command;
use crate::harness::tree::EntryData;

pub const NAME: &str = "mtools";

pub const PROGRAMS: [&str; 11] = [
    "mkfs.fat", "fsck.fat", "mdir", "mcopy", "mtype", "mmd", "mrd", "mdel", "mren", "mattrib",
    "mlabel",
];

/// GNU mtools driven through its command-line programs with an isolated
/// `MTOOLSRC` so host configuration cannot leak into the run.
pub struct MtoolsFatAdapter {
    image: PathBuf,
    config: PathBuf,
    scratch: PathBuf,
    next_file: usize,
}

impl MtoolsFatAdapter {
    pub fn new(image: PathBuf, workspace: &Path) -> Result<Self, String> {
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
        run_command_with_env(program, args, &[("MTOOLSRC", self.config.as_os_str())])
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
                let program = if entry.data.is_directory() {
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

/// Formats `path` with dosfstools' `mkfs.fat`.
pub fn format(path: &Path, case: FatCase) -> Result<(), String> {
    let file = File::create(path).map_err(|error| error.to_string())?;
    file.set_len(case.size).map_err(|error| error.to_string())?;
    drop(file);
    run_command(
        "mkfs.fat",
        vec![
            "-F".into(),
            case.mkfs_type().into(),
            "-n".into(),
            LABEL.into(),
            path.as_os_str().into(),
        ],
    )?;
    Ok(())
}

/// Runs dosfstools' `fsck.fat -n` and fails if it reports problems.
pub fn fsck(path: &Path) -> Result<(), String> {
    run_command("fsck.fat", vec!["-n".into(), path.as_os_str().into()])?;
    Ok(())
}

pub fn require_tools() -> Result<(), String> {
    for program in PROGRAMS {
        Command::new(program)
            .arg("--help")
            .env("LC_ALL", "C.UTF-8")
            .output()
            .map_err(|error| format!("required external tool {program} is unavailable: {error}"))?;
    }
    Ok(())
}

fn mtools_path(path: &str) -> String {
    format!("::{path}")
}
