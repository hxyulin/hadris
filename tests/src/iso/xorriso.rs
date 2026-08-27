//! xorriso / libisofs as both an ISO producer and an ISO consumer.

use std::ffi::OsString;
use std::fs;
use std::path::Path;
use std::process::{Command, Output};

use super::adapter::{IsoConsumer, IsoProducer};
use super::model::IsoState;
use crate::harness::command::{program_available, require_or_skip, run_command};

pub const PROGRAM: &str = "xorriso";
pub const NAME: &str = "xorriso/libisofs";

pub fn available() -> bool {
    program_available(PROGRAM, "--version")
}

/// Skips the calling test when xorriso is missing (see
/// [`crate::harness::command::REQUIRE_TOOLS_ENV`]).
pub fn require() -> bool {
    require_or_skip(PROGRAM, "--version")
}

/// `xorriso -as mkisofs -o image -V volume_id <extra> source`.
pub fn mkisofs(source: &Path, image: &Path, volume_id: &str, extra: &[&str]) -> Result<(), String> {
    let mut args: Vec<OsString> = vec![
        "-as".into(),
        "mkisofs".into(),
        "-o".into(),
        image.as_os_str().into(),
        "-V".into(),
        volume_id.into(),
    ];
    args.extend(extra.iter().map(OsString::from));
    args.push(source.as_os_str().into());
    run_command(PROGRAM, args).map(|_| ())
}

pub fn extract(image: &Path, destination: &Path) -> Result<(), String> {
    fs::create_dir_all(destination).map_err(|error| error.to_string())?;
    run_command(
        PROGRAM,
        vec![
            "-osirrox".into(),
            "on".into(),
            "-indev".into(),
            image.as_os_str().into(),
            "-extract".into(),
            "/".into(),
            destination.as_os_str().into(),
        ],
    )
    .map(|_| ())
}

/// Runs `xorriso -indev image <args>` and returns the raw output. xorriso
/// exits with status 1 for warnings, so callers inspect the status themselves.
pub fn inspect(image: &Path, args: &[&str]) -> Output {
    Command::new(PROGRAM)
        .arg("-indev")
        .arg(image)
        .args(args)
        .output()
        .expect("failed to run xorriso")
}

/// A small mixed tree used by the interoperability read tests.
pub fn write_sample_tree(dir: &Path) {
    fs::create_dir_all(dir.join("subdir")).unwrap();
    fs::create_dir_all(dir.join("deep/nested/path")).unwrap();
    fs::write(dir.join("readme.txt"), "This is a test file.\n").unwrap();
    fs::write(dir.join("hello.txt"), "Hello, World!\n").unwrap();
    fs::write(dir.join("subdir/data.bin"), vec![0u8; 1024]).unwrap();
    fs::write(
        dir.join("deep/nested/path/deep_file.txt"),
        "Deep nested content\n",
    )
    .unwrap();
    let large_content: Vec<u8> = (0..65536).map(|i| (i % 256) as u8).collect();
    fs::write(dir.join("large_file.bin"), &large_content).unwrap();
}

/// Plain ISO 9660 with no extensions.
pub fn create_minimal(source: &Path, image: &Path) -> Result<(), String> {
    mkisofs(source, image, "MINIMAL", &[])
}

pub fn create_joliet(source: &Path, image: &Path) -> Result<(), String> {
    mkisofs(source, image, "JOLIET_TEST", &["-J"])
}

pub fn create_joliet_rock_ridge(source: &Path, image: &Path) -> Result<(), String> {
    mkisofs(source, image, "TEST_VOLUME", &["-J", "-R"])
}

/// El Torito no-emulation boot with a four-sector load size.
pub fn create_bootable(source: &Path, image: &Path, boot_image: &str) -> Result<(), String> {
    mkisofs(
        source,
        image,
        "BOOT_TEST",
        &["-b", boot_image, "-no-emul-boot", "-boot-load-size", "4"],
    )
}

pub struct Xorriso;

impl IsoProducer for Xorriso {
    fn name(&self) -> String {
        NAME.to_string()
    }

    fn produce(&self, state: &IsoState, workspace: &Path, image: &Path) -> Result<(), String> {
        let source = workspace.join("xorriso-source");
        state.write_host(&source)?;
        mkisofs(&source, image, &state.volume_id, &["-iso-level", "1"])
    }
}

impl IsoConsumer for Xorriso {
    fn name(&self) -> String {
        NAME.to_string()
    }

    fn snapshot(&self, image: &Path, workspace: &Path) -> Result<IsoState, String> {
        let destination = workspace.join("xorriso-extracted");
        extract(image, &destination)?;
        IsoState::from_host(&destination)
    }
}
