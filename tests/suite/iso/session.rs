//! Sessions written onto images other producers made, checked by xorriso.

use std::fs;
use std::path::Path;

use hadris_fs::tree::Content;
use hadris_iso::SessionMode;
use hadris_iso::sync::Session;
use hadris_tests::iso::xorriso;

/// The `GPT start and size` lines xorriso reports for `image`.
fn gpt_partitions(image: &Path) -> Vec<String> {
    let output = xorriso::inspect(image, &["-report_system_area", "plain"]);
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter(|line| line.starts_with("GPT start and size"))
        .map(str::to_owned)
        .collect()
}

/// A partition xorriso appends after the ISO data survives both session
/// modes, and the new file is readable (osdev-6).
#[test]
fn sessions_keep_partitions_appended_after_the_image() {
    if !xorriso::require() {
        return;
    }
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source");
    fs::create_dir(&source).unwrap();
    fs::write(source.join("readme.txt"), "hello\n").unwrap();
    let appended = temp.path().join("appended.img");
    let payload: Vec<u8> = (0..1024 * 1024).map(|i| (i % 253) as u8).collect();
    fs::write(&appended, &payload).unwrap();
    let base = temp.path().join("base.iso");
    xorriso::mkisofs(
        &source,
        &base,
        "APPENDED",
        &[
            "-R",
            "-append_partition",
            "2",
            "0xef",
            appended.to_str().unwrap(),
            "-appended_part_as_gpt",
        ],
    )
    .unwrap();
    let before = gpt_partitions(&base);
    assert!(before.len() >= 2, "{before:?}");

    for mode in [SessionMode::Append, SessionMode::Rewrite] {
        let image = temp.path().join(format!("{mode:?}.iso"));
        fs::copy(&base, &image).unwrap();
        let file = fs::File::options()
            .read(true)
            .write(true)
            .open(&image)
            .and_then(hadris_storage::host::FileDevice::new)
            .unwrap();
        let mut session = Session::open(file).map_err(|err| err.into_error()).unwrap();
        session
            .tree_mut()
            .add_file("new.txt", Content::bytes("added\n"))
            .unwrap();
        let options = session.options();
        session.write(&options, mode).unwrap();
        drop(session);

        let after = gpt_partitions(&image);
        let appended_line = &before[1];
        assert!(
            after.contains(appended_line),
            "{mode:?}: {before:?} -> {after:?}"
        );
        let start: u64 = appended_line
            .split_whitespace()
            .rev()
            .nth(1)
            .unwrap()
            .parse()
            .unwrap();
        let bytes = fs::read(&image).unwrap();
        let at = start as usize * 512;
        assert_eq!(&bytes[at..at + payload.len()], &payload[..], "{mode:?}");

        let extracted = temp.path().join(format!("{mode:?}-out"));
        xorriso::extract(&image, &extracted).unwrap();
        assert_eq!(fs::read(extracted.join("new.txt")).unwrap(), b"added\n");
        assert_eq!(fs::read(extracted.join("readme.txt")).unwrap(), b"hello\n");
    }
}
