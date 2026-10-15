//! Tree walkers fail on an image whose subdirectory leads back to the root
//! instead of looping forever.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// Runs `hadris args`, failing the test when it does not exit in time, and
/// returns whether it succeeded and its stdout and stderr.
fn run(args: &[&str]) -> (bool, String) {
    let mut child = Command::new(env!("CARGO_BIN_EXE_hadris"))
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let drain = |mut pipe: Box<dyn Read + Send>| {
        std::thread::spawn(move || {
            let mut out = Vec::new();
            pipe.read_to_end(&mut out).unwrap();
            out
        })
    };
    let stdout = drain(Box::new(child.stdout.take().unwrap()));
    let stderr = drain(Box::new(child.stderr.take().unwrap()));
    let deadline = Instant::now() + Duration::from_secs(30);
    let status = loop {
        if let Some(status) = child.try_wait().unwrap() {
            break status;
        }
        if Instant::now() > deadline {
            child.kill().unwrap();
            panic!("{args:?} did not terminate");
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    let mut output = stdout.join().unwrap();
    output.extend(stderr.join().unwrap());
    (
        status.success(),
        String::from_utf8_lossy(&output).into_owned(),
    )
}

fn assert_loop_fails(args: &[&str]) {
    let (success, output) = run(args);
    assert!(!success, "{args:?}: {output}");
    assert!(output.contains("Directory loop at /"), "{args:?}: {output}");
    assert!(
        output.contains("corrupt filesystem data"),
        "{args:?}: {output}"
    );
}

fn text(path: &Path) -> &str {
    path.to_str().unwrap()
}

fn source(temp: &Path) -> PathBuf {
    let source = temp.join("source");
    std::fs::create_dir_all(source.join("sub")).unwrap();
    std::fs::write(source.join("readme.txt"), b"readme").unwrap();
    std::fs::write(source.join("sub/inner.txt"), b"inner").unwrap();
    source
}

fn build(args: &[&str]) {
    let output = Command::new(env!("CARGO_BIN_EXE_hadris"))
        .args(args)
        .output()
        .unwrap();
    assert!(output.status.success(), "{args:?}: {output:?}");
}

/// Points every ISO 9660 directory record of `sub` at the root directory
/// of its tree: the primary and Rock Ridge tree, and the Joliet tree.
fn loop_iso(image: &mut [u8]) {
    let mut patched = 0;
    for sector in 16.. {
        let at = sector * 2048;
        let kind = image[at];
        if kind == 255 {
            break;
        }
        if kind != 1 && kind != 2 {
            continue;
        }
        let root = &image[at + 156..at + 156 + 34];
        let (extent, size) = (root[2..10].to_vec(), root[10..18].to_vec());
        let name: &[u8] = if kind == 1 { b"SUB" } else { b"\0s\0u\0b" };
        let found = find_records(image, name);
        assert!(!found.is_empty(), "no {name:?} record");
        for record in found {
            image[record + 2..record + 10].copy_from_slice(&extent);
            image[record + 10..record + 18].copy_from_slice(&size);
            patched += 1;
        }
    }
    assert!(patched >= 1);
}

fn find_records(image: &[u8], name: &[u8]) -> Vec<usize> {
    (0..image.len() - 33 - name.len())
        .filter(|&at| {
            let len = usize::from(image[at]);
            let le: Vec<u8> = image[at + 2..at + 6].iter().rev().copied().collect();
            len >= 33 + name.len()
                && image[at + 25] & 0x02 != 0
                && usize::from(image[at + 32]) == name.len()
                && &image[at + 33..at + 33 + name.len()] == name
                && le == image[at + 6..at + 10]
        })
        .collect()
}

/// Points the UDF file identifier of `sub` at the root directory, whose
/// location every parent entry holds in an image with no deeper
/// directories.
fn loop_udf(image: &mut [u8]) {
    let fids: Vec<usize> = (0..image.len() - 40)
        .step_by(4)
        .filter(|&at| u16::from_le_bytes([image[at], image[at + 1]]) == 257 && tag_valid(image, at))
        .collect();
    let parent = fids
        .iter()
        .copied()
        .find(|&at| image[at + 18] & 0x08 != 0)
        .expect("no parent entry");
    let root = image[parent + 24..parent + 30].to_vec();
    let sub = fids
        .iter()
        .copied()
        .find(|&at| {
            let len = usize::from(image[at + 19]);
            let iu = usize::from(u16::from_le_bytes([image[at + 36], image[at + 37]]));
            let id = &image[at + 38 + iu..at + 38 + iu + len];
            image[at + 18] & 0x02 != 0 && (id == b"\x08sub" || id == b"\x10\0s\0u\0b")
        })
        .expect("no sub entry");
    image[sub + 24..sub + 30].copy_from_slice(&root);
    let len = usize::from(u16::from_le_bytes([image[sub + 10], image[sub + 11]]));
    let crc = crc16(&image[sub + 16..sub + 16 + len]);
    image[sub + 8..sub + 10].copy_from_slice(&crc.to_le_bytes());
    image[sub + 4] = checksum(&image[sub..sub + 16]);
}

fn checksum(tag: &[u8]) -> u8 {
    tag.iter()
        .enumerate()
        .filter(|&(i, _)| i != 4)
        .fold(0u8, |sum, (_, &byte)| sum.wrapping_add(byte))
}

fn tag_valid(image: &[u8], at: usize) -> bool {
    image[at + 4] == checksum(&image[at..at + 16]) && image[at + 5] == 0
}

fn crc16(bytes: &[u8]) -> u16 {
    let mut crc = 0u16;
    for &byte in bytes {
        crc ^= u16::from(byte) << 8;
        for _ in 0..8 {
            crc = if crc & 0x8000 != 0 {
                (crc << 1) ^ 0x1021
            } else {
                crc << 1
            };
        }
    }
    crc
}

fn patched(path: &Path, patch: fn(&mut [u8])) -> PathBuf {
    let mut image = std::fs::read(path).unwrap();
    patch(&mut image);
    let looped = path.with_extension("looped");
    std::fs::write(&looped, image).unwrap();
    looped
}

#[test]
fn iso_walkers_fail_on_a_directory_loop() {
    let temp = tempfile::tempdir().unwrap();
    let source = source(temp.path());
    let image = temp.path().join("disc.iso");
    build(&[
        "iso",
        "create",
        "--joliet",
        "--rock-ridge",
        "-o",
        text(&image),
        text(&source),
    ]);
    let looped = patched(&image, loop_iso);

    assert_loop_fails(&["iso", "tree", text(&looped)]);
    assert_loop_fails(&["iso", "verify", "--strict", text(&looped)]);
    let (extracted, _) = run(&[
        "iso",
        "extract",
        text(&looped),
        "-o",
        text(&temp.path().join("out")),
    ]);
    assert!(!extracted);
}

#[test]
fn udf_walkers_fail_on_a_directory_loop() {
    let temp = tempfile::tempdir().unwrap();
    let source = source(temp.path());
    let image = temp.path().join("disc.udf");
    build(&["udf", "create", "-o", text(&image), text(&source)]);
    let looped = patched(&image, loop_udf);

    assert_loop_fails(&["udf", "tree", text(&looped)]);
    assert_loop_fails(&["udf", "verify", text(&looped)]);
}

#[test]
fn bridge_compare_fails_on_a_directory_loop() {
    let temp = tempfile::tempdir().unwrap();
    let source = source(temp.path());
    let image = temp.path().join("bridge.iso");
    build(&["udf", "bridge", text(&source), "--output", text(&image)]);

    assert_loop_fails(&["udf", "compare", text(&patched(&image, loop_iso))]);
    assert_loop_fails(&["udf", "compare", text(&patched(&image, loop_udf))]);
}
