//! Native exFAT tools: exfatprogs `mkfs.exfat` and `fsck.exfat` (from the
//! `PATH`, or the `hadris-exfatprogs` Docker image), macOS `newfs_exfat` and
//! `fsck_exfat`, and the macOS kernel driver.

use std::ffi::OsString;
use std::path::Path;
use std::process::Command;

use super::ExFatCase;
use crate::fat::LABEL;
use crate::harness::mount::NativeMount;
use crate::harness::run_command;

/// The Docker image with exfatprogs that stands in for a local install.
pub const DOCKER_IMAGE: &str = "hadris-exfatprogs";

/// Where the exfatprogs tools run.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Exfatprogs {
    Local,
    Docker,
}

impl Exfatprogs {
    pub fn find() -> Option<Self> {
        if Command::new("fsck.exfat").arg("-V").output().is_ok() {
            return Some(Self::Local);
        }
        let docker = Command::new("docker")
            .args(["image", "inspect", DOCKER_IMAGE])
            .output();
        docker
            .is_ok_and(|output| output.status.success())
            .then_some(Self::Docker)
    }

    fn run(self, tool: &str, args: &[&str], image: &Path) -> Result<(), String> {
        match self {
            Self::Local => {
                let mut all: Vec<OsString> = args.iter().map(OsString::from).collect();
                all.push(image.as_os_str().into());
                run_command(tool, all).map(|_| ())
            }
            Self::Docker => {
                let dir = image.parent().ok_or("image has no directory")?;
                let name = image.file_name().ok_or("image has no name")?;
                let mut all: Vec<OsString> = vec![
                    "run".into(),
                    "--rm".into(),
                    "-v".into(),
                    format!("{}:/w", dir.display()).into(),
                    DOCKER_IMAGE.into(),
                    tool.into(),
                ];
                all.extend(args.iter().map(OsString::from));
                all.push(Path::new("/w").join(name).into_os_string());
                run_command("docker", all).map(|_| ())
            }
        }
    }

    /// Formats `image` with `mkfs.exfat` for `case`.
    pub fn format(self, image: &Path, case: ExFatCase) -> Result<(), String> {
        create_image(image, case.size)?;
        let cluster = case.cluster.to_string();
        self.run("mkfs.exfat", &["-L", LABEL, "-c", &cluster], image)
    }

    /// Runs `fsck.exfat -n` on `image`.
    pub fn fsck(self, image: &Path) -> Result<(), String> {
        self.run("fsck.exfat", &["-n"], image)
    }
}

fn create_image(image: &Path, size: u64) -> Result<(), String> {
    let file = std::fs::File::create(image).map_err(|error| error.to_string())?;
    file.set_len(size).map_err(|error| error.to_string())
}

/// Whether macOS `newfs_exfat`, `fsck_exfat` and `hdiutil` are present.
pub fn macos_available() -> bool {
    cfg!(target_os = "macos")
        && Path::new("/sbin/newfs_exfat").exists()
        && Path::new("/sbin/fsck_exfat").exists()
}

struct Attached(String);

impl Attached {
    fn new(image: &Path) -> Result<Self, String> {
        let output = run_command(
            "hdiutil",
            vec![
                "attach".into(),
                "-nomount".into(),
                "-imagekey".into(),
                "diskimage-class=CRawDiskImage".into(),
                image.as_os_str().into(),
            ],
        )?;
        let text = String::from_utf8_lossy(&output.stdout).into_owned();
        let device = text
            .split_whitespace()
            .find(|field| field.starts_with("/dev/"))
            .ok_or_else(|| format!("hdiutil did not report a device: {text:?}"))?;
        Ok(Self(device.to_string()))
    }

    fn raw(&self) -> String {
        self.0.replace("/dev/disk", "/dev/rdisk")
    }
}

impl Drop for Attached {
    fn drop(&mut self) {
        let _ = run_command("hdiutil", vec!["detach".into(), self.0.clone().into()]);
    }
}

/// Formats `image` with macOS `newfs_exfat`, which picks its own cluster
/// size.
pub fn newfs(image: &Path, size: u64) -> Result<(), String> {
    create_image(image, size)?;
    let attached = Attached::new(image)?;
    run_command(
        "/sbin/newfs_exfat",
        vec!["-v".into(), LABEL.into(), attached.raw().into()],
    )
    .map(|_| ())
}

/// Runs macOS `fsck_exfat -n` on `image`. `hdiutil` attaches with 512-byte
/// blocks, so only volumes with 512-byte sectors can be checked.
pub fn fsck_macos(image: &Path) -> Result<(), String> {
    let attached = Attached::new(image)?;
    run_command("/sbin/fsck_exfat", vec!["-n".into(), attached.raw().into()]).map(|_| ())
}

/// Mounts `image` read-write through the macOS kernel driver.
pub fn mount(image: &Path, mountpoint: &Path) -> Result<NativeMount, String> {
    if !cfg!(target_os = "macos") {
        return Err("native exFAT mounts are supported on macOS".to_string());
    }
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
}
