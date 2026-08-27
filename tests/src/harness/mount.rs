use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};

use super::command::{command_value, run_command};

/// A kernel mount of a disk image that is detached when dropped.
///
/// Every constructor is compiled on every platform and fails at runtime when
/// the host cannot provide that mount, so callers can pick by `target_os`
/// without extra `cfg` plumbing.
pub struct NativeMount {
    mountpoint: PathBuf,
    unmount: Option<(&'static str, Vec<OsString>)>,
}

impl NativeMount {
    /// `mount [-t fs_type] -o options image mountpoint`, through `sudo -n`
    /// when the caller is not root.
    pub fn linux(
        image: &Path,
        mountpoint: &Path,
        fs_type: Option<&str>,
        options: &str,
    ) -> Result<Self, String> {
        if !cfg!(target_os = "linux") {
            return Err("kernel mounts through mount(8) require Linux".to_string());
        }
        fs::create_dir_all(mountpoint).map_err(|error| error.to_string())?;
        let root = command_value("id", "-u")? == "0";
        let (program, mut args) = if root {
            ("mount", Vec::new())
        } else {
            ("sudo", vec![OsString::from("-n"), OsString::from("mount")])
        };
        if let Some(fs_type) = fs_type {
            args.extend(["-t".into(), fs_type.into()]);
        }
        args.extend([
            "-o".into(),
            options.into(),
            image.as_os_str().into(),
            mountpoint.as_os_str().into(),
        ]);
        run_command(program, args)?;
        let unmount = if root {
            ("umount", vec![mountpoint.as_os_str().into()])
        } else {
            (
                "sudo",
                vec!["-n".into(), "umount".into(), mountpoint.as_os_str().into()],
            )
        };
        Ok(Self {
            mountpoint: mountpoint.to_path_buf(),
            unmount: Some(unmount),
        })
    }

    /// `hdiutil attach <attach_args> -mountpoint mountpoint image`.
    pub fn macos(image: &Path, mountpoint: &Path, attach_args: &[&str]) -> Result<Self, String> {
        if !cfg!(target_os = "macos") {
            return Err("hdiutil mounts require macOS".to_string());
        }
        fs::create_dir_all(mountpoint).map_err(|error| error.to_string())?;
        let mut args = vec![OsString::from("attach")];
        args.extend(attach_args.iter().map(OsString::from));
        args.extend([
            "-mountpoint".into(),
            mountpoint.as_os_str().into(),
            image.as_os_str().into(),
        ]);
        run_command("hdiutil", args)?;
        Ok(Self {
            mountpoint: mountpoint.to_path_buf(),
            unmount: Some((
                "hdiutil",
                vec!["detach".into(), mountpoint.as_os_str().into()],
            )),
        })
    }

    /// `Mount-DiskImage` through PowerShell; the mountpoint is the assigned
    /// drive letter.
    pub fn windows(image: &Path) -> Result<Self, String> {
        if !cfg!(target_os = "windows") {
            return Err("Mount-DiskImage requires Windows".to_string());
        }
        let escaped = image.to_string_lossy().replace('\'', "''");
        let attach = format!(
            "$p=(Resolve-Path '{escaped}').Path; (Mount-DiskImage -ImagePath $p -PassThru | Get-Volume).DriveLetter"
        );
        let output = run_command(
            "powershell.exe",
            vec!["-NoProfile".into(), "-Command".into(), attach.into()],
        )?;
        let drive = String::from_utf8(output.stdout).map_err(|error| error.to_string())?;
        let detach = format!("Dismount-DiskImage -ImagePath (Resolve-Path '{escaped}').Path");
        Ok(Self {
            mountpoint: PathBuf::from(format!("{}:\\", drive.trim())),
            unmount: Some((
                "powershell.exe",
                vec!["-NoProfile".into(), "-Command".into(), detach.into()],
            )),
        })
    }

    pub fn path(&self) -> &Path {
        &self.mountpoint
    }

    pub fn unmount(mut self) -> Result<(), String> {
        match self.unmount.take() {
            Some((program, args)) => run_command(program, args).map(|_| ()),
            None => Ok(()),
        }
    }
}

impl Drop for NativeMount {
    fn drop(&mut self) {
        if let Some((program, args)) = self.unmount.take() {
            let _ = run_command(program, args);
        }
    }
}
