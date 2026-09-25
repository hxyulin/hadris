//! `create` writes its output only once the archive is complete.

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
    let archive = out.join("root.cpio");
    std::fs::write(&archive, vec![0xFF; 100_000]).unwrap();

    let refused = hadris("cpio")
        .args(["create", "-o"])
        .arg(&archive)
        .arg(&source)
        .output()
        .unwrap();
    assert!(!refused.status.success(), "{refused:?}");
    assert_eq!(std::fs::read(&archive).unwrap(), vec![0xFF; 100_000]);

    let output = hadris("cpio")
        .args(["create", "--force", "-o"])
        .arg(&archive)
        .arg(&source)
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    assert_eq!(names(&out), ["root.cpio"]);
    let bytes = std::fs::read(&archive).unwrap();
    assert!(bytes.starts_with(b"070701"));
    assert!(bytes.len() < 100_000);
}

/// The `hadris` binary with the format subcommand `format`.
fn hadris(format: &str) -> std::process::Command {
    let mut command = std::process::Command::new(env!("CARGO_BIN_EXE_hadris"));
    command.arg(format);
    command
}
