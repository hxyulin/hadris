//! Volume descriptor sets of images produced by xorriso.

use std::fs;

use hadris_iso::volume::VolumeDescriptor;
use hadris_tests::iso::xorriso;
use tempfile::TempDir;

use super::{open_file, xorriso_sample_image};

#[test]
fn test_read_xorriso_minimal_iso() {
    let Some((_temp, iso_path)) = xorriso_sample_image(xorriso::create_minimal) else {
        return;
    };
    let image = open_file(&iso_path);
    let pvd = image.read_pvd().unwrap();
    assert_eq!(pvd.volume_identifier.to_str().trim(), "MINIMAL");
}

#[test]
fn test_read_xorriso_joliet_iso() {
    let Some((_temp, iso_path)) = xorriso_sample_image(xorriso::create_joliet) else {
        return;
    };
    let image = open_file(&iso_path);
    let pvd = image.read_pvd().unwrap();
    assert_eq!(pvd.volume_identifier.to_str().trim(), "JOLIET_TEST");

    let has_joliet = image
        .read_volume_descriptors()
        .any(|vd| matches!(vd, Ok(VolumeDescriptor::Supplementary(_))));
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
    let image = open_file(&iso_path);
    let pvd = image.read_pvd().unwrap();
    assert_eq!(pvd.volume_identifier.to_str().trim(), "TEST_VOLUME");
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

    let image = open_file(&iso_path);
    let has_joliet = image
        .read_volume_descriptors()
        .any(|vd| matches!(vd, Ok(VolumeDescriptor::Supplementary(_))));
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

    let image = open_file(&iso_path);
    let mut primary_count = 0;
    let mut supplementary_count = 0;
    let mut terminator_count = 0;
    for vd_result in image.read_volume_descriptors() {
        match vd_result {
            Ok(VolumeDescriptor::Primary(_)) => primary_count += 1,
            Ok(VolumeDescriptor::Supplementary(_)) => supplementary_count += 1,
            Ok(VolumeDescriptor::End(_)) => terminator_count += 1,
            _ => {}
        }
    }
    assert_eq!(
        primary_count, 1,
        "Should have exactly 1 primary volume descriptor"
    );
    assert!(
        supplementary_count >= 1,
        "Should have at least 1 supplementary (Joliet) descriptor"
    );
    assert!(terminator_count <= 1, "Should have at most 1 terminator");
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
