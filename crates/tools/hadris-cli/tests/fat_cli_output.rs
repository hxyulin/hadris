//! `create` writes its output only once the image is complete.

use std::path::Path;
use std::process::Output;

fn create(source: &Path, image: &Path, extra: &[&str]) -> Output {
    hadris("fat")
        .arg("create")
        .arg(source)
        .arg("--output")
        .arg(image)
        .args(extra)
        .output()
        .unwrap()
}

fn names(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
        .collect();
    names.sort();
    names
}

fn fixture() -> (tempfile::TempDir, std::path::PathBuf, std::path::PathBuf) {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source");
    std::fs::create_dir(&source).unwrap();
    std::fs::write(source.join("big.bin"), vec![1u8; 2 * 1024 * 1024]).unwrap();
    let out = temp.path().join("out");
    std::fs::create_dir(&out).unwrap();
    (temp, source, out)
}

#[test]
fn failed_import_leaves_no_output() {
    let (_temp, source, out) = fixture();
    let output = create(&source, &out.join("disk.img"), &["--size", "1000000"]);
    assert!(!output.status.success(), "{output:?}");
    assert!(names(&out).is_empty(), "{:?}", names(&out));
}

#[test]
fn failed_format_leaves_no_output() {
    let (_temp, source, out) = fixture();
    let output = create(&source, &out.join("disk.img"), &["-V", "bad*lbl"]);
    assert!(!output.status.success(), "{output:?}");
    assert!(names(&out).is_empty(), "{:?}", names(&out));
}

#[test]
fn create_replaces_an_existing_output_only_with_force() {
    let (_temp, source, out) = fixture();
    let image = out.join("disk.img");
    std::fs::write(&image, b"keep").unwrap();
    let output = create(&source, &image, &[]);
    assert!(!output.status.success(), "{output:?}");
    assert_eq!(std::fs::read(&image).unwrap(), b"keep");
    assert_eq!(names(&out), ["disk.img"]);

    let output = create(&source, &image, &["--force"]);
    assert!(output.status.success(), "{output:?}");
    assert_eq!(names(&out), ["disk.img"]);
    assert!(std::fs::metadata(&image).unwrap().len() > 4);
}

/// The `hadris` binary with the format subcommand `format`.
fn hadris(format: &str) -> std::process::Command {
    let mut command = std::process::Command::new(env!("CARGO_BIN_EXE_hadris"));
    command.arg(format);
    command
}
