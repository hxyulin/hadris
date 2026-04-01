//! Test helpers for hadris-ntfs integration tests.
//!
//! Provides [`NtfsTestImage`] which creates temporary NTFS filesystem images
//! using system tools (`mkntfs`, `ntfscp`, `ntfs-3g`) and cleans up on drop.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use tempfile::TempDir;

const DEFAULT_IMAGE_SIZE: u64 = 10 * 1024 * 1024; // 10 MB

/// A temporary NTFS image that is removed when dropped.
pub struct NtfsTestImage {
    _temp_dir: TempDir,
    image_path: PathBuf,
    staging_path: PathBuf,
}

impl NtfsTestImage {
    /// Create a 10 MB NTFS image with the given volume label.
    ///
    /// Returns `None` if `mkntfs` is not available or formatting fails.
    pub fn new(label: &str) -> Option<Self> {
        Self::with_size(label, DEFAULT_IMAGE_SIZE)
    }

    /// Create an NTFS image of the specified size (bytes).
    pub fn with_size(label: &str, size: u64) -> Option<Self> {
        if !tool_available("mkntfs") {
            eprintln!("SKIP: mkntfs not found");
            return None;
        }

        let temp_dir = TempDir::new().ok()?;
        let image_path = temp_dir.path().join("test.ntfs");
        let staging_path = temp_dir.path().join("staging");
        std::fs::create_dir_all(&staging_path).ok()?;

        // Create a sparse file of the given size
        {
            let f = std::fs::File::create(&image_path).ok()?;
            f.set_len(size).ok()?;
        }

        let output = Command::new("mkntfs")
            .args(["-F", "-Q", "-L", label])
            .arg(&image_path)
            .stderr(Stdio::piped())
            .stdout(Stdio::null())
            .output()
            .ok()?;

        if !output.status.success() {
            eprintln!(
                "mkntfs failed: {}",
                String::from_utf8_lossy(&output.stderr)
            );
            return None;
        }

        Some(Self {
            _temp_dir: temp_dir,
            image_path,
            staging_path,
        })
    }

    /// Path to the raw NTFS image file.
    pub fn path(&self) -> &Path {
        &self.image_path
    }

    /// Copy a file into the NTFS image's root directory using `ntfscp`.
    ///
    /// Returns `false` if `ntfscp` is unavailable or the copy fails.
    pub fn add_file(&self, name: &str, content: &[u8]) -> bool {
        let src = self.staging_path.join(name);
        if let Some(parent) = src.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if std::fs::write(&src, content).is_err() {
            return false;
        }

        let ok = run_quiet("ntfscp", &[
            self.image_path.to_str().unwrap(),
            src.to_str().unwrap(),
            name,
        ]);

        // ntfscp may dirty the journal; clear it so subsequent calls succeed
        if ok {
            self.fix_journal();
        }
        ok
    }

    /// Mount the image with `ntfs-3g` (FUSE), run a closure with the mount
    /// path, then unmount.  Returns `None` when FUSE mounting is unavailable.
    pub fn with_mounted<F, R>(&self, f: F) -> Option<R>
    where
        F: FnOnce(&Path) -> R,
    {
        let mount_dir = self.staging_path.join("mnt");
        std::fs::create_dir_all(&mount_dir).ok()?;

        self.fix_journal();

        let output = Command::new("ntfs-3g")
            .arg(&self.image_path)
            .arg(&mount_dir)
            .stderr(Stdio::piped())
            .stdout(Stdio::null())
            .output()
            .ok()?;

        if !output.status.success() {
            eprintln!(
                "SKIP: ntfs-3g mount failed: {}",
                String::from_utf8_lossy(&output.stderr)
            );
            return None;
        }

        let result = f(&mount_dir);

        // Unmount (try fusermount3 first, fall back to fusermount)
        let _ = Command::new("fusermount3")
            .args(["-u"])
            .arg(&mount_dir)
            .status()
            .or_else(|_| {
                Command::new("fusermount")
                    .args(["-u"])
                    .arg(&mount_dir)
                    .status()
            });

        Some(result)
    }

    /// Clear the dirty journal flag so ntfscp / ntfs-3g won't refuse to work.
    fn fix_journal(&self) {
        let _ = Command::new("ntfsfix")
            .arg("-d")
            .arg(&self.image_path)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
}

fn tool_available(name: &str) -> bool {
    Command::new("sh")
        .args(["-c", &format!("command -v {name}")])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

fn run_quiet(program: &str, args: &[&str]) -> bool {
    Command::new(program)
        .args(args)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}
