//! `create` writes its output only once the image is complete.

use std::path::Path;
use std::process::{Command, Output};

fn create(source: &Path, image: &Path, extra: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_hadris-iso"))
        .arg("create")
        .args(extra)
        .arg("-o")
        .arg(image)
        .arg(source)
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

#[test]
fn failed_create_keeps_the_existing_output() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source");
    std::fs::create_dir(&source).unwrap();
    std::fs::write(source.join("a.txt"), b"a").unwrap();
    let out = temp.path().join("out");
    std::fs::create_dir(&out).unwrap();
    let image = out.join("keep.iso");
    std::fs::write(&image, b"previous contents").unwrap();

    let output = create(&source, &image, &["-V", &"A".repeat(40)]);
    assert!(!output.status.success(), "{output:?}");
    assert_eq!(std::fs::read(&image).unwrap(), b"previous contents");
    assert_eq!(names(&out), ["keep.iso"]);
}

#[test]
fn failed_create_leaves_no_output() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source");
    std::fs::create_dir_all(source.join("1/2/3/4/5/6/7/8/9")).unwrap();
    let out = temp.path().join("out");
    std::fs::create_dir(&out).unwrap();

    let output = create(&source, &out.join("deep.iso"), &[]);
    assert!(!output.status.success(), "{output:?}");
    assert!(names(&out).is_empty(), "{:?}", names(&out));
}

#[test]
fn create_replaces_an_existing_output() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source");
    std::fs::create_dir(&source).unwrap();
    std::fs::write(source.join("a.txt"), b"a").unwrap();
    let out = temp.path().join("out");
    std::fs::create_dir(&out).unwrap();
    let image = out.join("disc.iso");
    std::fs::write(&image, b"stale").unwrap();

    let output = create(&source, &image, &[]);
    assert!(output.status.success(), "{output:?}");
    assert_eq!(names(&out), ["disc.iso"]);
    assert_eq!(std::fs::metadata(&image).unwrap().len(), 32 * 2048);
}
