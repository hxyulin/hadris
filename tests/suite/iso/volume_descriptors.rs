//! Volume descriptor sets of images produced by xorriso.

use std::fs;

use hadris_fs::tree::{Content, Tree};
use hadris_iso::IsoOptions;
use hadris_iso::raw::VolumeDescriptor;
use hadris_tests::harness::command::{program_available, run_command};
use hadris_tests::iso::hadris::write_tree;
use hadris_tests::iso::xorriso;
use tempfile::TempDir;

use super::{descriptors, open, open_file, volume_id, xorriso_sample_image};

/// Images end in 150 zero blocks inside the volume, as xorriso and
/// `mkisofs -pad` write, so readers that read ahead (isoinfo) accept small
/// images (integrate-5).
#[test]
fn small_images_are_padded_like_xorriso() {
    let mut tree = Tree::new();
    tree.add_file("a.txt", Content::bytes("hi\n")).unwrap();
    let bytes = write_tree(&tree, &IsoOptions::default()).unwrap();
    let image = open(bytes.clone());
    let volume = image.volume_blocks() as usize;
    assert_eq!(volume * 2048, bytes.len());
    assert!(volume >= 150 + 18, "{volume}");
    assert!(bytes[(volume - 150) * 2048..].iter().all(|&byte| byte == 0));

    let temp = TempDir::new().unwrap();
    let path = temp.path().join("small.iso");
    fs::write(&path, &bytes).unwrap();
    if program_available("isoinfo", "-version") {
        run_command("isoinfo", vec!["-d".into(), "-i".into(), path.into()]).unwrap();
    }
    if xorriso::require() {
        let source = temp.path().join("source");
        fs::create_dir(&source).unwrap();
        fs::write(source.join("a.txt"), "hi\n").unwrap();
        let peer = temp.path().join("peer.iso");
        xorriso::mkisofs(&source, &peer, "PEER", &[]).unwrap();
        let peer = open_file(&peer);
        let data_blocks = |image: &hadris_tests::iso::hadris::Image| image.volume_blocks() - 150;
        assert!(data_blocks(&peer) < 150);
        assert!(data_blocks(&image) < 150);
    }
}

#[test]
fn test_read_xorriso_minimal_iso() {
    let Some((_temp, iso_path)) = xorriso_sample_image(xorriso::create_minimal) else {
        return;
    };
    let mut image = open_file(&iso_path);
    assert_eq!(volume_id(&mut image), "MINIMAL");
}

#[test]
fn test_read_xorriso_joliet_iso() {
    let Some((_temp, iso_path)) = xorriso_sample_image(xorriso::create_joliet) else {
        return;
    };
    let mut image = open_file(&iso_path);
    assert_eq!(volume_id(&mut image), "JOLIET_TEST");

    let has_joliet = descriptors(&mut image)
        .iter()
        .any(|vd| matches!(vd, VolumeDescriptor::Supplementary(_)));
    assert!(
        has_joliet,
        "Should have supplementary volume descriptor for Joliet"
    );
}

#[test]
fn test_read_xorriso_rockridge_iso() {
    let Some((_temp, iso_path)) = xorriso_sample_image(xorriso::create_joliet_rock_ridge) else {
        return;
    };
    let mut image = open_file(&iso_path);
    assert_eq!(volume_id(&mut image), "TEST_VOLUME");
}

#[test]
fn test_unicode_filenames_joliet() {
    if !xorriso::require() {
        return;
    }
    let temp_dir = TempDir::new().unwrap();
    let content_dir = temp_dir.path().join("content");
    let iso_path = temp_dir.path().join("unicode.iso");
    fs::create_dir(&content_dir).unwrap();
    fs::write(content_dir.join("日本語.txt"), "Japanese filename\n").unwrap();
    fs::write(content_dir.join("中文.txt"), "Chinese filename\n").unwrap();
    fs::write(content_dir.join("한국어.txt"), "Korean filename\n").unwrap();
    xorriso::create_joliet(&content_dir, &iso_path).unwrap();

    let mut image = open_file(&iso_path);
    let has_joliet = descriptors(&mut image)
        .iter()
        .any(|vd| matches!(vd, VolumeDescriptor::Supplementary(_)));
    assert!(has_joliet, "Should have Joliet supplementary volume");
}

#[test]
fn test_volume_descriptor_chain() {
    if !xorriso::require() {
        return;
    }
    let temp_dir = TempDir::new().unwrap();
    let content_dir = temp_dir.path().join("content");
    let iso_path = temp_dir.path().join("vd_chain.iso");
    fs::create_dir(&content_dir).unwrap();
    fs::write(content_dir.join("test.txt"), "test").unwrap();
    xorriso::create_joliet_rock_ridge(&content_dir, &iso_path).unwrap();

    let mut image = open_file(&iso_path);
    let mut primary_count = 0;
    let mut supplementary_count = 0;
    let mut terminator_count = 0;
    for vd in descriptors(&mut image) {
        match vd {
            VolumeDescriptor::Primary(_) => primary_count += 1,
            VolumeDescriptor::Supplementary(_) => supplementary_count += 1,
            VolumeDescriptor::Terminator(_) => terminator_count += 1,
            _ => {}
        }
    }
    assert!(
        primary_count >= 1,
        "ECMA-119 6.7.1.1: the primary volume descriptor is recorded at least once"
    );
    assert!(
        supplementary_count >= 1,
        "Should have at least 1 supplementary (Joliet) descriptor"
    );
    assert!(
        terminator_count >= 1,
        "ECMA-119 6.7.1.6: the descriptor set ends with one or more terminators"
    );
}

#[test]
fn test_xorriso_report() {
    let Some((_temp, iso_path)) = xorriso_sample_image(xorriso::create_joliet_rock_ridge) else {
        return;
    };
    let output = xorriso::inspect(&iso_path, &["-report_el_torito", "as_mkisofs"]);
    assert!(
        output.status.success() || output.status.code() == Some(1),
        "xorriso report should not fail catastrophically"
    );
}
