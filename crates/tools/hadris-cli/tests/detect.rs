//! `hadris detect` and the options every format shares.

use std::process::Command;

fn hadris() -> Command {
    Command::new(env!("CARGO_BIN_EXE_hadris"))
}

#[test]
fn version_and_help_cover_every_format() {
    let output = hadris().arg("--version").output().unwrap();
    assert!(output.status.success());
    assert!(String::from_utf8_lossy(&output.stdout).starts_with("hadris "));
    let output = hadris().arg("--help").output().unwrap();
    let help = String::from_utf8_lossy(&output.stdout);
    for format in ["fat", "iso", "udf", "cpio", "detect"] {
        assert!(help.contains(format), "{help}");
        assert!(
            hadris()
                .args([format, "--help"])
                .status()
                .unwrap()
                .success()
        );
    }
}

#[test]
fn detect_lists_a_bridge_and_refuses_unknown_bytes() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source");
    std::fs::create_dir(&source).unwrap();
    std::fs::write(source.join("a.txt"), b"a").unwrap();
    let image = temp.path().join("bridge.iso");
    let created = hadris()
        .args(["udf", "bridge", "-o"])
        .arg(&image)
        .arg(&source)
        .output()
        .unwrap();
    assert!(created.status.success(), "{created:?}");

    let output = hadris().arg("detect").arg(&image).output().unwrap();
    assert!(output.status.success(), "{output:?}");
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "ISO 9660 and UDF bridge\nISO 9660\nUDF\n"
    );

    let blank = temp.path().join("blank.img");
    std::fs::write(&blank, vec![0u8; 64 * 1024]).unwrap();
    let output = hadris().arg("detect").arg(&blank).output().unwrap();
    assert!(!output.status.success());
}
