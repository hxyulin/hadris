use std::path::Path;
use std::process::Command;

fn program_runs(program: &str) -> bool {
    Command::new(program)
        .arg("-V")
        .output()
        .map(|output| output.status.success() || output.status.code().is_some())
        .unwrap_or(false)
}

pub fn fsck_exfat_available() -> bool {
    program_runs("fsck.exfat")
}

/// Run `fsck.exfat -n` and require a clean exit.
pub fn fsck_check(image_path: &Path) -> Result<(), String> {
    let output = Command::new("fsck.exfat")
        .args(["-n", image_path.to_str().unwrap()])
        .output()
        .map_err(|error| format!("failed to spawn fsck.exfat: {error}"))?;

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
