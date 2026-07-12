#![allow(dead_code)]

use std::path::Path;
use std::process::Command;

/// fsck.fat is the standard external validator for FAT12/16/32 images on
/// Linux (and is available on macOS via Homebrew's `dosfstools`). The
/// binary is sometimes installed as `fsck.vfat`, which is a symlink to
/// `fsck.fat`.
fn fsck_fat_program() -> Option<&'static str> {
    if program_runs("fsck.fat") {
        Some("fsck.fat")
    } else if program_runs("fsck.vfat") {
        Some("fsck.vfat")
    } else {
        None
    }
}

fn program_runs(prog: &str) -> bool {
    Command::new(prog)
        .arg("-V")
        .output()
        .map(|o| o.status.success() || o.status.code().is_some())
        .unwrap_or(false)
}

pub fn fsck_fat_available() -> bool {
    fsck_fat_program().is_some()
}

/// Run `fsck.fat -n` (read-only). Returns Ok(()) on a clean image.
///
/// fsck.fat exit codes (per dosfstools(8)):
///   0  - filesystem is clean
///   1  - recoverable errors found (with `-n`, errors are reported but
///        not corrected, so exit 0 is the only "clean" outcome)
///   2+ - usage / unrecoverable error
pub fn fsck_check(image_path: &Path) -> Result<(), String> {
    let prog = fsck_fat_program().ok_or_else(|| "fsck.fat not available".to_string())?;
    let output = Command::new(prog)
        .args(["-n", image_path.to_str().unwrap()])
        .output()
        .map_err(|e| format!("failed to spawn {prog}: {e}"))?;

    if output.status.success() {
        Ok(())
    } else {
        Err(format!(
            "{prog} reported errors (exit {:?}):\nstdout:\n{}\nstderr:\n{}",
            output.status.code(),
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr),
        ))
    }
}
