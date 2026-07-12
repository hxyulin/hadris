#![allow(dead_code)]

use std::path::Path;
use std::process::Command;

pub fn mkfs_exfat_available() -> bool {
    Command::new("mkfs.exfat")
        .arg("-V")
        .output()
        .map(|o| o.status.success() || o.status.code().is_some())
        .unwrap_or(false)
}

pub fn fsck_exfat_available() -> bool {
    Command::new("fsck.exfat")
        .arg("-V")
        .output()
        .map(|o| o.status.success() || o.status.code().is_some())
        .unwrap_or(false)
}

/// Run `fsck.exfat -n` (read-only). Returns Ok(()) on a clean image.
pub fn fsck_check(image_path: &Path) -> Result<(), String> {
    let output = Command::new("fsck.exfat")
        .args(["-n", image_path.to_str().unwrap()])
        .output()
        .map_err(|e| format!("failed to spawn fsck.exfat: {e}"))?;

    if output.status.success() {
        Ok(())
    } else {
        Err(format!(
            "fsck.exfat reported errors (exit {:?}):\nstdout:\n{}\nstderr:\n{}",
            output.status.code(),
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr),
        ))
    }
}
