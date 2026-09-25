//! Every read command against one image built by `create`.

use std::path::{Path, PathBuf};
use std::process::Output;

fn run(args: &[&str]) -> Output {
    hadris("fat").args(args).output().unwrap()
}

fn stdout_of(args: &[&str]) -> String {
    let output = run(args);
    assert!(output.status.success(), "{args:?}: {output:?}");
    String::from_utf8(output.stdout).unwrap()
}

fn text(path: &Path) -> &str {
    path.to_str().unwrap()
}

/// Host names in `dir`, as the host stores them.
fn host_names(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
        .collect();
    names.sort();
    names
}

struct Fixture {
    temp: tempfile::TempDir,
    image: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        Self::with_args(&[])
    }

    fn exfat() -> Self {
        Self::with_args(&["--fat-type", "exfat"])
    }

    fn with_args(extra: &[&str]) -> Self {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("source");
        std::fs::create_dir_all(source.join("Sub/deep")).unwrap();
        std::fs::write(source.join("top.txt"), b"top").unwrap();
        std::fs::write(source.join("b"), b"b").unwrap();
        std::fs::write(source.join("a"), b"a").unwrap();
        std::fs::write(source.join("Sub/Inner.TXT"), b"inner").unwrap();
        std::fs::write(source.join("Sub/deep/data.bin"), vec![7u8; 10_000]).unwrap();
        let image = temp.path().join("disk.img");
        let mut args = vec!["create", text(&source), "--output", text(&image)];
        args.extend_from_slice(extra);
        let output = run(&args);
        assert!(output.status.success(), "{output:?}");
        Self { temp, image }
    }

    fn image(&self) -> &str {
        text(&self.image)
    }

    fn dir(&self, name: &str) -> PathBuf {
        self.temp.path().join(name)
    }
}

#[test]
fn listing_commands() {
    listing(&Fixture::new());
    listing(&Fixture::exfat());
}

fn listing(fx: &Fixture) {
    assert_eq!(
        stdout_of(&["ls", fx.image()]),
        "Sub/\na\nb\ntop.txt\n",
        "create imports in name order"
    );
    let long = stdout_of(&["ls", "--long", fx.image(), "/sub"]);
    assert!(long.contains("<DIR>  deep"), "{long}");
    assert!(long.contains("5  Inner.TXT"), "{long}");

    let tree = stdout_of(&["tree", fx.image()]);
    assert!(tree.starts_with("/\n"), "{tree}");
    assert!(tree.contains("├── Sub/\n"), "{tree}");
    assert!(tree.contains("│   └── deep/\n"), "{tree}");
    assert!(tree.contains("│       └── data.bin\n"), "{tree}");
    assert!(tree.contains("└── top.txt\n"), "{tree}");
    let shallow = stdout_of(&["tree", "--depth", "1", fx.image()]);
    assert!(
        shallow.contains("Sub/") && !shallow.contains("Inner.TXT"),
        "{shallow}"
    );

    let output = run(&["ls", fx.image(), "/missing"]);
    assert!(!output.status.success());
}

#[test]
fn report_commands() {
    let fx = Fixture::new();
    reports(&fx);
    let info = stdout_of(&["info", fx.image()]);
    assert!(info.contains("FAT Type:        Fat12"), "{info}");

    let fx = Fixture::exfat();
    reports(&fx);
    let info = stdout_of(&["info", fx.image()]);
    assert!(info.contains("FS Revision:     1.00"), "{info}");
}

fn reports(fx: &Fixture) {
    let info = stdout_of(&["info", fx.image()]);
    assert!(info.contains("Volume Label:    HADRIS"), "{info}");
    let stat = stdout_of(&["stat", fx.image()]);
    assert!(stat.contains("Files:             5"), "{stat}");
    let verify = stdout_of(&["verify", "--verbose", fx.image()]);
    assert!(verify.contains("Result: PASS"), "{verify}");
    let check = stdout_of(&["check", fx.image()]);
    assert!(check.contains("Result: PASS"), "{check}");

    let chain = stdout_of(&["chain", fx.image(), "/Sub/deep/data.bin"]);
    assert!(chain.contains("File size: 10000 bytes"), "{chain}");
    assert!(chain.contains("Fragments: 1"), "{chain}");
    let small = stdout_of(&["chain", fx.image(), "/top.txt"]);
    assert!(small.contains("Chain length: 1 clusters"), "{small}");

    let fragmentation = stdout_of(&["fragmentation", fx.image()]);
    assert!(
        fragmentation.contains("Total Files:             5"),
        "{fragmentation}"
    );
}

#[test]
fn extract_uses_the_stored_name() {
    extract_stored_name(&Fixture::new());
    extract_stored_name(&Fixture::exfat());
}

fn extract_stored_name(fx: &Fixture) {
    let out = fx.dir("file");
    stdout_of(&[
        "extract",
        fx.image(),
        "-o",
        text(&out),
        "-p",
        "/SUB/inner.txt",
    ]);
    assert_eq!(host_names(&out), ["Inner.TXT"]);
    assert_eq!(std::fs::read(out.join("Inner.TXT")).unwrap(), b"inner");

    let out = fx.dir("dir");
    stdout_of(&["extract", fx.image(), "-o", text(&out), "-p", "/sub/"]);
    assert_eq!(host_names(&out), ["Sub"]);
    assert_eq!(host_names(&out.join("Sub")), ["Inner.TXT", "deep"]);
    assert_eq!(
        std::fs::read(out.join("Sub/deep/data.bin")).unwrap(),
        vec![7u8; 10_000]
    );
}

#[test]
fn extract_stays_inside_the_output_directory() {
    let fx = Fixture::new();
    let nested = fx.dir("nested");
    let out = nested.join("out");

    stdout_of(&["extract", fx.image(), "-o", text(&out), "-p", "/Sub/.."]);
    assert_eq!(host_names(&nested), ["out"]);
    assert_eq!(host_names(&out), ["Sub", "a", "b", "top.txt"]);

    let out = fx.dir("dotted");
    stdout_of(&[
        "extract",
        fx.image(),
        "-o",
        text(&out),
        "-p",
        "/Sub/deep/../.",
    ]);
    assert_eq!(host_names(&out), ["Sub"]);

    let output = run(&["extract", fx.image(), "-o", text(&out), "-p", "/missing"]);
    assert!(!output.status.success());
}

#[test]
fn cat_and_list_alias_on_exfat() {
    let fx = Fixture::exfat();
    assert_eq!(stdout_of(&["cat", fx.image(), "/sub/inner.txt"]), "inner");
    assert_eq!(stdout_of(&["list", fx.image()]), "Sub/\na\nb\ntop.txt\n");
}

#[test]
fn verify_fails_on_a_damaged_image() {
    let fx = Fixture::new();
    let mut bytes = std::fs::read(&fx.image).unwrap();
    // Mark every cluster of the first FAT used past the reserved entries,
    // which leaves lost clusters and a FAT copy mismatch.
    let fat = 512 * usize::from(u16::from_le_bytes([bytes[14], bytes[15]]));
    for byte in &mut bytes[fat + 3..fat + 512] {
        *byte = 0xFF;
    }
    std::fs::write(&fx.image, bytes).unwrap();
    let output = run(&["verify", fx.image()]);
    assert!(!output.status.success(), "{output:?}");
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("Result: FAIL"), "{stdout}");
}

/// The `hadris` binary with the format subcommand `format`.
fn hadris(format: &str) -> std::process::Command {
    let mut command = std::process::Command::new(env!("CARGO_BIN_EXE_hadris"));
    command.arg(format);
    command
}
