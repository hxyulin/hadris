#![allow(dead_code)]

use std::fs;
use std::path::Path;
use std::process::Command;

/// Check if xorriso is available on the system
pub fn xorriso_available() -> bool {
    Command::new("xorriso")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Create a test directory structure with some files
pub fn create_test_content(dir: &Path) {
    // Create directories
    fs::create_dir_all(dir.join("subdir")).unwrap();
    fs::create_dir_all(dir.join("deep/nested/path")).unwrap();

    // Create files with various content
    fs::write(dir.join("readme.txt"), "This is a test file.\n").unwrap();
    fs::write(dir.join("hello.txt"), "Hello, World!\n").unwrap();
    fs::write(dir.join("subdir/data.bin"), vec![0u8; 1024]).unwrap();
    fs::write(
        dir.join("deep/nested/path/deep_file.txt"),
        "Deep nested content\n",
    )
    .unwrap();

    // Create a larger file (64KB)
    let large_content: Vec<u8> = (0..65536).map(|i| (i % 256) as u8).collect();
    fs::write(dir.join("large_file.bin"), &large_content).unwrap();
}

/// Create an ISO using xorriso
pub fn create_iso_with_xorriso(content_dir: &Path, iso_path: &Path) -> bool {
    let output = Command::new("xorriso")
        .args([
            "-as",
            "mkisofs",
            "-o",
            iso_path.to_str().unwrap(),
            "-V",
            "TEST_VOLUME",
            "-J", // Joliet
            "-R", // Rock Ridge
            content_dir.to_str().unwrap(),
        ])
        .output()
        .expect("Failed to run xorriso");

    output.status.success()
}

/// Create a minimal ISO using xorriso (no extensions)
pub fn create_minimal_iso_with_xorriso(content_dir: &Path, iso_path: &Path) -> bool {
    let output = Command::new("xorriso")
        .args([
            "-as",
            "mkisofs",
            "-o",
            iso_path.to_str().unwrap(),
            "-V",
            "MINIMAL",
            content_dir.to_str().unwrap(),
        ])
        .output()
        .expect("Failed to run xorriso");

    output.status.success()
}

/// Create a Joliet-only ISO using xorriso
pub fn create_joliet_iso_with_xorriso(content_dir: &Path, iso_path: &Path) -> bool {
    let output = Command::new("xorriso")
        .args([
            "-as",
            "mkisofs",
            "-o",
            iso_path.to_str().unwrap(),
            "-V",
            "JOLIET_TEST",
            "-J", // Joliet only
            content_dir.to_str().unwrap(),
        ])
        .output()
        .expect("Failed to run xorriso");

    output.status.success()
}

/// Create a bootable ISO using xorriso with El-Torito
pub fn create_bootable_iso_with_xorriso(
    content_dir: &Path,
    iso_path: &Path,
    boot_image: &str,
) -> bool {
    let output = Command::new("xorriso")
        .args([
            "-as",
            "mkisofs",
            "-o",
            iso_path.to_str().unwrap(),
            "-V",
            "BOOT_TEST",
            "-b",
            boot_image,
            "-no-emul-boot",
            "-boot-load-size",
            "4",
            content_dir.to_str().unwrap(),
        ])
        .output()
        .expect("Failed to run xorriso");

    if !output.status.success() {
        eprintln!(
            "xorriso stderr: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    output.status.success()
}

/// Check if qemu is available
pub fn qemu_available() -> bool {
    Command::new("qemu-system-x86_64")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Run QEMU with a timeout
pub fn run_qemu_with_timeout(iso_path: &Path, timeout_secs: u64) -> Option<String> {
    use std::io::Read as StdRead;
    use std::process::Stdio;
    use std::thread;
    use std::time::Duration;

    let mut child = Command::new("qemu-system-x86_64")
        .args([
            "-cdrom",
            iso_path.to_str().unwrap(),
            "-boot",
            "d",
            "-nographic",
            "-serial",
            "stdio",
            "-no-reboot",
            "-m",
            "16",
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .ok()?;

    // Wait with timeout
    let timeout = Duration::from_secs(timeout_secs);
    let start = std::time::Instant::now();

    loop {
        match child.try_wait() {
            Ok(Some(_status)) => break,
            Ok(None) => {
                if start.elapsed() > timeout {
                    let _ = child.kill();
                    break;
                }
                thread::sleep(Duration::from_millis(100));
            }
            Err(_) => break,
        }
    }

    let mut stdout = String::new();
    if let Some(mut out) = child.stdout.take() {
        let _ = out.read_to_string(&mut stdout);
    }

    Some(stdout)
}
