//! The host platform's own FAT formatter, checker, and kernel driver.

use std::fs::File;
use std::path::Path;

use super::model::FsState;
use super::{FatCase, LABEL};
use crate::harness::mount::NativeMount;
use crate::harness::{command_value, run_command};

/// Returns the native `(formatter, checker)` pair after confirming both run.
pub fn tools() -> Result<(&'static str, &'static str), String> {
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

pub fn format(path: &Path, case: FatCase) -> Result<(), String> {
    let (formatter, _) = tools()?;
    let file = File::create(path).map_err(|error| error.to_string())?;
    file.set_len(case.size).map_err(|error| error.to_string())?;
    drop(file);
    if cfg!(target_os = "linux") {
        run_command(
            formatter,
            vec![
                "-F".into(),
                case.mkfs_type().into(),
                "-n".into(),
                LABEL.into(),
                path.as_os_str().into(),
            ],
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
            case.mkfs_type().into(),
            "-v".into(),
            LABEL.into(),
            device.into(),
        ],
    );
    let detach_result = run_command("hdiutil", vec!["detach".into(), device.into()]);
    match (format_result, detach_result) {
        (Err(error), _) => Err(error),
        (Ok(_), Err(error)) => Err(error),
        (Ok(_), Ok(_)) => Ok(()),
    }
}

pub fn fsck(path: &Path) -> Result<(), String> {
    let (_, checker) = tools()?;
    run_command(checker, vec!["-n".into(), path.as_os_str().into()])?;
    Ok(())
}

/// Mounts `image` read-write through the kernel FAT driver.
pub fn mount(image: &Path, mountpoint: &Path) -> Result<NativeMount, String> {
    if cfg!(target_os = "linux") {
        let uid = command_value("id", "-u")?;
        let gid = command_value("id", "-g")?;
        NativeMount::linux(
            image,
            mountpoint,
            Some("vfat"),
            &format!("loop,uid={uid},gid={gid}"),
        )
    } else if cfg!(target_os = "macos") {
        NativeMount::macos(
            image,
            mountpoint,
            &[
                "-nobrowse",
                "-imagekey",
                "diskimage-class=CRawDiskImage",
                "-owners",
                "off",
            ],
        )
    } else {
        Err("native FAT mounts are supported on Linux and macOS".to_string())
    }
}

/// Drops platform metadata files (macOS AppleDouble `._*`) that the kernel
/// driver creates and which are not part of the semantic tree.
pub fn remove_native_metadata(state: &mut FsState) {
    if cfg!(target_os = "macos") {
        state.entries.retain(|path, _| {
            !path
                .rsplit('/')
                .next()
                .is_some_and(|name| name.starts_with("._"))
        });
    }
}
