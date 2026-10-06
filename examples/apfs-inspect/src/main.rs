//! Inspect and extract a selected APFS volume from an image.

use anyhow::{Result, ensure};
use hadris_apfs::{VolumeSelector, sync::ApfsFs};
use hadris_fs::sync::{FileSystem, Volume, read_tree};
use hadris_fs::{ErrorKind, MountOptions, OpenOptions, Resolve};
use hadris_io::SeekFrom;
use hadris_storage::{BlockSize, MemDevice};

#[path = "../../../crates/block/hadris-apfs/tests/common/mod.rs"]
mod image;

fn device(bytes: Vec<u8>) -> MemDevice<Vec<u8>> {
    MemDevice::new(bytes, BlockSize::new(image::BLOCK as u32).unwrap())
}

fn main() -> Result<()> {
    let fs = ApfsFs::mount(device(image::use_case_image()), MountOptions::new())?;
    let vol = Volume::with_resolve(fs, Resolve::Follow);
    let mut dir = vol.read_dir("/")?;
    let mut listed = 0;
    while let Some(entry) = dir.next_entry() {
        let entry = entry?;
        println!("{}", entry.name().to_str()?);
        listed += 1;
    }
    ensure!(listed == 9);
    ensure!(vol.read_link("/link")? == b"file0.txt");
    let mut file = vol.open("/link", OpenOptions::new().read())?;
    file.seek(SeekFrom::Start(4))?;
    let mut buf = [0; 4];
    ensure!(file.read(&mut buf)? == 4 && &buf == b"ents");
    file.close()?;
    ensure!(vol.create_dir("/new").unwrap_err().kind() == ErrorKind::ReadOnly);

    let tree = read_tree(&vol, "/")?;
    let destination = tempfile::tempdir()?;
    hadris_fs::host::write_tree(destination.path(), &tree)?;
    ensure!(std::fs::read(destination.path().join("file0.txt"))? == image::file_contents(0));
    ensure!(std::fs::read(destination.path().join("alias.txt"))? == image::file_contents(0));
    ensure!(std::fs::read(destination.path().join("sparse.txt"))? == image::holey_contents(0));
    #[cfg(unix)]
    ensure!(
        std::fs::read_link(destination.path().join("link"))? == std::path::Path::new("file0.txt")
    );

    let bytes = image::multi_volume_image();
    let Err(error) = ApfsFs::mount(device(bytes.clone()), MountOptions::new()) else {
        anyhow::bail!("an ambiguous APFS container mounted");
    };
    ensure!(error.into_device().into_inner() == bytes);
    let mut fs = ApfsFs::mount_volume(
        device(bytes),
        MountOptions::new(),
        VolumeSelector::Name("Other"),
    )?;
    hadris_fs::sync::contract::check_read_only(&mut fs)?;
    ensure!(!fs.capabilities().writable());
    println!("APFS inspection, linked and sparse reads, extraction and explicit selection passed");
    Ok(())
}
