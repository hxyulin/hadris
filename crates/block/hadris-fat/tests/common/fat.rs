use std::path::Path;
use std::process::Command;

/// Locate the standard read-only FAT validator. Some systems install it under
/// the `fsck.vfat` compatibility name.
fn fsck_fat_program() -> Option<&'static str> {
    if program_runs("fsck.fat") {
        Some("fsck.fat")
    } else if program_runs("fsck.vfat") {
        Some("fsck.vfat")
    } else {
        None
    }
}

fn program_runs(program: &str) -> bool {
    Command::new(program)
        .arg("-V")
        .output()
        .map(|output| output.status.success() || output.status.code().is_some())
        .unwrap_or(false)
}

pub fn fsck_fat_available() -> bool {
    fsck_fat_program().is_some()
}

/// Run `fsck.fat -n`. Only exit code zero represents a clean image; with
/// read-only validation, code one means errors were found but not repaired.
pub fn fsck_check(image_path: &Path) -> Result<(), String> {
    let program = fsck_fat_program().ok_or_else(|| "fsck.fat not available".to_string())?;
    let output = Command::new(program)
        .args(["-n", image_path.to_str().unwrap()])
        .output()
        .map_err(|error| format!("failed to spawn {program}: {error}"))?;

    if output.status.success() {
        Ok(())
    } else {
        Err(format!(
            "{program} reported errors (exit {:?}):\nstdout:\n{}\nstderr:\n{}",
            output.status.code(),
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr),
        ))
    }
}
