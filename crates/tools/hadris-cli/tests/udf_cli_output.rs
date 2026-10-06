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

#[test]
fn unsupported_revisions_fail_during_argument_parsing_without_touching_output() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source");
    std::fs::create_dir(&source).unwrap();
    let image = temp.path().join("existing.udf");
    std::fs::write(&image, b"original").unwrap();
    for command in ["create", "bridge"] {
        for revision in ["2.50", "2.60"] {
            let output = hadris("udf")
                .args([command, "--force", "--revision", revision, "--output"])
                .arg(&image)
                .arg(&source)
                .output()
                .unwrap();
            assert_eq!(
                output.status.code(),
                Some(2),
                "{command} {revision}: {output:?}"
            );
            assert!(
                String::from_utf8_lossy(&output.stderr).contains("expected a writable revision")
            );
            assert_eq!(std::fs::read(&image).unwrap(), b"original");
            assert_eq!(names(temp.path()), ["existing.udf", "source"]);
        }
        let help = hadris("udf").args([command, "--help"]).output().unwrap();
        assert!(help.status.success());
        let help = String::from_utf8(help.stdout).unwrap();
        assert!(help.contains("1.02, 1.50, 2.00, or 2.01"));
        assert!(!help.contains("2.50") && !help.contains("2.60"));
    }
}

/// The `hadris` binary with the format subcommand `format`.
fn hadris(format: &str) -> std::process::Command {
    let mut command = std::process::Command::new(env!("CARGO_BIN_EXE_hadris"));
    command.arg(format);
    command
}
