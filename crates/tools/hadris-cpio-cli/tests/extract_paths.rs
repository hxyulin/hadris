//! Extraction must never write outside the output directory.

use std::path::Path;

fn newc_entry(out: &mut Vec<u8>, name: &str, mode: u32, data: &[u8]) {
    let namesize = name.len() as u32 + 1;
    let fields = [
        1,
        mode,
        0,
        0,
        1,
        0,
        data.len() as u32,
        0,
        0,
        0,
        0,
        namesize,
        0,
    ];
    out.extend_from_slice(b"070701");
    for field in fields {
        out.extend_from_slice(format!("{field:08X}").as_bytes());
    }
    out.extend_from_slice(name.as_bytes());
    out.push(0);
    while out.len() % 4 != 0 {
        out.push(0);
    }
    out.extend_from_slice(data);
    while out.len() % 4 != 0 {
        out.push(0);
    }
}

fn extract(archive: &[u8], root: &Path) -> std::process::Output {
    let archive_path = root.join("evil.cpio");
    std::fs::write(&archive_path, archive).unwrap();
    std::process::Command::new(env!("CARGO_BIN_EXE_hadris-cpio"))
        .args(["extract", archive_path.to_str().unwrap(), "--output"])
        .arg(root.join("out"))
        .output()
        .unwrap()
}

#[test]
fn rejects_parent_and_absolute_names() {
    let temp = tempfile::tempdir().unwrap();
    let absolute = temp.path().join("absolute.txt");
    let mut archive = Vec::new();
    newc_entry(&mut archive, "../parent.txt", 0o100644, b"evil");
    newc_entry(&mut archive, "a/../../nested.txt", 0o100644, b"evil");
    newc_entry(&mut archive, absolute.to_str().unwrap(), 0o100644, b"evil");
    newc_entry(&mut archive, "./ok.txt", 0o100644, b"fine");
    newc_entry(&mut archive, "TRAILER!!!", 0, b"");

    let output = extract(&archive, temp.path());
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(!temp.path().join("parent.txt").exists());
    assert!(!temp.path().join("nested.txt").exists());
    assert!(!absolute.exists());
    assert_eq!(
        std::fs::read(temp.path().join("out/ok.txt")).unwrap(),
        b"fine"
    );
    assert!(String::from_utf8_lossy(&output.stderr).contains("unsafe path"));
}

#[cfg(unix)]
#[test]
fn rejects_writes_through_extracted_symlinks() {
    let temp = tempfile::tempdir().unwrap();
    let outside = temp.path().join("outside");
    std::fs::create_dir(&outside).unwrap();
    let mut archive = Vec::new();
    newc_entry(
        &mut archive,
        "link",
        0o120777,
        outside.to_str().unwrap().as_bytes(),
    );
    newc_entry(&mut archive, "link/escaped.txt", 0o100644, b"evil");
    newc_entry(&mut archive, "TRAILER!!!", 0, b"");

    let output = extract(&archive, temp.path());
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(!outside.join("escaped.txt").exists());
}
