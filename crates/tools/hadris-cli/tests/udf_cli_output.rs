//! `create` writes its output only once the image is complete.

use std::path::Path;

fn names(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
        .collect();
    names.sort();
    names
}

#[test]
fn create_replaces_an_existing_output_only_with_force() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source");
    std::fs::create_dir(&source).unwrap();
    std::fs::write(source.join("a.txt"), b"a").unwrap();
    let out = temp.path().join("out");
    std::fs::create_dir(&out).unwrap();
    let image = out.join("disc.udf");
    std::fs::write(&image, b"stale").unwrap();

    let refused = hadris("udf")
        .args(["create", "-o"])
        .arg(&image)
        .arg(&source)
        .output()
        .unwrap();
    assert!(!refused.status.success(), "{refused:?}");
    assert_eq!(std::fs::read(&image).unwrap(), b"stale");

    let output = hadris("udf")
        .args(["create", "--force", "-o"])
        .arg(&image)
        .arg(&source)
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    assert_eq!(names(&out), ["disc.udf"]);
    assert!(std::fs::metadata(&image).unwrap().len() > 5);
}

/// The `hadris` binary with the format subcommand `format`.
fn hadris(format: &str) -> std::process::Command {
    let mut command = std::process::Command::new(env!("CARGO_BIN_EXE_hadris"));
    command.arg(format);
    command
}
