use std::path::Path;
use std::process::Command;

fn program_runs(program: &str) -> bool {
    Command::new(program)
        .arg("-V")
        .output()
        .map(|output| output.status.success() || output.status.code().is_some())
        .unwrap_or(false)
}

fn checker() -> &'static str {
    if cfg!(target_os = "macos") {
        "/sbin/fsck_exfat"
    } else {
        "fsck.exfat"
    }
}

pub fn fsck_exfat_available() -> bool {
    program_runs(checker())
}

/// Run the platform exFAT checker in read-only mode.
pub fn fsck_check(image_path: &Path) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    let device = AttachedImage::new(image_path)?;
    #[cfg(target_os = "macos")]
    let check_path = Path::new(&device.0);
    #[cfg(not(target_os = "macos"))]
    let check_path = image_path;
    let output = Command::new(checker())
        .arg("-n")
        .arg(check_path)
        .output()
        .map_err(|error| format!("failed to spawn exFAT checker: {error}"))?;

    if output.status.success() {
        Ok(())
    } else {
        Err(format!(
            "exFAT checker reported errors (exit {:?}):\nstdout:\n{}\nstderr:\n{}",
            output.status.code(),
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr),
        ))
    }
}

#[cfg(target_os = "macos")]
struct AttachedImage(String);

#[cfg(target_os = "macos")]
impl AttachedImage {
    fn new(path: &Path) -> Result<Self, String> {
        let output = Command::new("hdiutil")
            .args([
                "attach",
                "-nomount",
                "-readonly",
                "-noverify",
                "-imagekey",
                "diskimage-class=CRawDiskImage",
            ])
            .arg(path)
            .output()
            .map_err(|error| error.to_string())?;
        if !output.status.success() {
            return Err(format!(
                "hdiutil attach failed: {}",
                String::from_utf8_lossy(&output.stderr)
            ));
        }
        let text = String::from_utf8_lossy(&output.stdout);
        let device = text
            .split_whitespace()
            .find(|word| word.starts_with("/dev/disk"))
            .ok_or_else(|| format!("hdiutil did not return a device: {text}"))?;
        Ok(Self(device.replacen("/dev/disk", "/dev/rdisk", 1)))
    }
}

#[cfg(target_os = "macos")]
impl Drop for AttachedImage {
    fn drop(&mut self) {
        let result = Command::new("hdiutil")
            .args([
                "detach",
                "-quiet",
                &self.0.replace("/dev/rdisk", "/dev/disk"),
            ])
            .status();
        if !matches!(&result, Ok(status) if status.success()) {
            let message = format!("hdiutil failed to detach {}: {result:?}", self.0);
            if std::thread::panicking() {
                eprintln!("{message}");
            } else {
                panic!("{message}");
            }
        }
    }
}
