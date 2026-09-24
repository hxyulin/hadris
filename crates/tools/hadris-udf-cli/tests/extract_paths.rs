//! Extraction must never write outside the output directory.

use std::io::Cursor;

use hadris_udf::write::{SimpleDir, SimpleFile, UdfWriteOptions, UdfWriter};

fn extract_image(root: &SimpleDir) -> (tempfile::TempDir, std::process::Output) {
    let image = UdfWriter::create(Cursor::new(Vec::new()), root, UdfWriteOptions::default())
        .unwrap()
        .target
        .into_inner();

    let temp = tempfile::tempdir().unwrap();
    let image_path = temp.path().join("evil.udf");
    std::fs::write(&image_path, image).unwrap();
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_hadris-udf"))
        .args(["extract", image_path.to_str().unwrap(), "--output"])
        .arg(temp.path().join("a/out"))
        .output()
        .unwrap();
    (temp, output)
}

#[test]
fn rejects_parent_directory_name() {
    let mut parent = SimpleDir::new("..");
    parent.add_file(SimpleFile::new("escaped.txt", b"evil".to_vec()));
    let mut root = SimpleDir::root();
    root.add_dir(parent);
    let (temp, output) = extract_image(&root);
    assert!(!output.status.success());
    assert!(!temp.path().join("a/escaped.txt").exists());
}

#[test]
fn rejects_name_with_separators() {
    let mut root = SimpleDir::root();
    root.add_file(SimpleFile::new("../escaped.txt", b"evil".to_vec()));
    let (temp, output) = extract_image(&root);
    assert!(!output.status.success());
    assert!(!temp.path().join("a/escaped.txt").exists());
}

#[test]
fn extracts_plain_names() {
    let mut sub = SimpleDir::new("sub");
    sub.add_file(SimpleFile::new("inner.txt", b"inner".to_vec()));
    let mut root = SimpleDir::root();
    root.add_file(SimpleFile::new("top.txt", b"top".to_vec()));
    root.add_dir(sub);
    let (temp, output) = extract_image(&root);
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let out = temp.path().join("a/out");
    assert_eq!(std::fs::read(out.join("top.txt")).unwrap(), b"top");
    assert_eq!(std::fs::read(out.join("sub/inner.txt")).unwrap(), b"inner");
}
