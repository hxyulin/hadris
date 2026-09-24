//! Read commands against an image built by `create`.

use std::path::Path;
use std::process::{Command, Output};

fn run(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_hadris-iso"))
        .args(args)
        .output()
        .unwrap()
}

fn stdout_of(args: &[&str]) -> String {
    let output = run(args);
    assert!(output.status.success(), "{args:?}: {output:?}");
    String::from_utf8(output.stdout).unwrap()
}

fn text(path: &Path) -> &str {
    path.to_str().unwrap()
}

fn host_names(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
        .collect();
    names.sort();
    names
}

fn image(temp: &Path) -> String {
    let source = temp.join("source");
    std::fs::create_dir_all(source.join("docs/deep")).unwrap();
    std::fs::write(source.join("readme.txt"), b"readme").unwrap();
    std::fs::write(source.join("docs/guide.txt"), b"guide").unwrap();
    std::fs::write(source.join("docs/deep/data.bin"), vec![3u8; 70_000]).unwrap();
    let image = temp.join("disc.iso");
    stdout_of(&[
        "create",
        "--joliet",
        "--rock-ridge",
        "-o",
        text(&image),
        text(&source),
    ]);
    text(&image).to_string()
}

#[test]
fn extract_merges_the_root_and_names_other_paths() {
    let temp = tempfile::tempdir().unwrap();
    let image = image(temp.path());

    let all = temp.path().join("all");
    stdout_of(&["extract", &image, "-o", text(&all)]);
    assert_eq!(host_names(&all), ["docs", "readme.txt"]);
    assert_eq!(
        std::fs::read(all.join("docs/deep/data.bin")).unwrap(),
        vec![3u8; 70_000]
    );

    let dir = temp.path().join("dir");
    stdout_of(&["extract", &image, "-o", text(&dir), "-p", "/docs"]);
    assert_eq!(host_names(&dir), ["docs"]);
    assert_eq!(host_names(&dir.join("docs")), ["deep", "guide.txt"]);

    let file = temp.path().join("file");
    stdout_of(&[
        "extract",
        &image,
        "-o",
        text(&file),
        "-p",
        "/docs/guide.txt",
    ]);
    assert_eq!(std::fs::read(file.join("guide.txt")).unwrap(), b"guide");

    let primary = temp.path().join("primary");
    stdout_of(&[
        "extract",
        &image,
        "-o",
        text(&primary),
        "-p",
        "/DOCS/GUIDE.TXT",
    ]);
    assert_eq!(host_names(&primary), ["GUIDE.TXT"]);

    assert!(
        !run(&["extract", &image, "-o", text(&file), "-p", "/missing"])
            .status
            .success()
    );
}

#[test]
fn cat_list_alias_and_strict_check() {
    let temp = tempfile::tempdir().unwrap();
    let image = image(temp.path());

    assert_eq!(stdout_of(&["cat", &image, "/docs/guide.txt"]), "guide");
    assert_eq!(stdout_of(&["list", &image]), "docs/\nreadme.txt\n");
    let check = stdout_of(&["check", "--strict", &image]);
    assert!(check.contains("Verification passed"), "{check}");
}
