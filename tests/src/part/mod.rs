//! Partition table peers: GPT fdisk `sgdisk`.

use std::ffi::OsString;
use std::path::Path;

use crate::harness::run_command;

/// Report directory name under the harness report root.
pub const FORMAT: &str = "part";

/// Runs `sgdisk` with `args` on `image` and returns its standard output.
pub fn sgdisk(args: &[&str], image: &Path) -> Result<String, String> {
    let mut all: Vec<OsString> = args.iter().map(OsString::from).collect();
    all.push(image.as_os_str().into());
    let output = run_command("sgdisk", all)?;
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

/// Runs `sgdisk -v` on `image` and fails unless it finds no problems.
pub fn verify(image: &Path) -> Result<(), String> {
    let report = sgdisk(&["-v"], image)?;
    if report.contains("No problems found") {
        Ok(())
    } else {
        Err(format!("sgdisk -v reported problems:\n{report}"))
    }
}
