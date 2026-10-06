//! A V3 migration using one Hadris dependency: path-based editing, generic
//! FAT/ISO node reads and allocation-free embedded FAT handles.

use std::io::Write;

use hadris::fat::embedded::{MountToken, Options, sync::Fat};
use hadris::fat::sync::FatFs;
use hadris::fat::{FatKind, FatOptions};
use hadris::fs::sync::{FileSystem, Volume};
use hadris::fs::{Content, FsResult, MountOptions, Node, OpenOptions, Resolve, Tree};
use hadris::iso::IsoOptions;
use hadris::iso::sync::IsoFs;
use hadris::storage::{BlockSize, MemDevice};

const CONTENTS: &[u8] = b"one dependency, three V3 interfaces\n";

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut device = MemDevice::new(vec![0; 2 << 20], BlockSize::new(512).unwrap());
    hadris::fat::sync::format(&mut device, &FatOptions::new().with_kind(FatKind::Fat12))?;
    let volume = Volume::new(FatFs::mount(device, MountOptions::new())?);
    volume.create_dir_all("/boot")?;
    let mut file = volume.open("/boot/kernel.bin", OpenOptions::new().write().create_new())?;
    file.write_all(CONTENTS)?;
    file.close()?;
    let fs = volume
        .into_inner()
        .map_err(|_| std::io::Error::other("a handle is still open"))?;
    let device = fs.unmount()?;

    let mut fs = FatFs::mount(device, MountOptions::new().read_only())?;
    let (bytes, len) = first_bytes(&mut fs, b"/boot/kernel.bin")?;
    assert_eq!(&bytes[..len], CONTENTS);
    let device = fs.unmount()?;

    let mut token = MountToken::new();
    let mut fat: Fat<'_, _> = Fat::mount_with(device, &mut token, Options::new().read_only())?;
    let boot = fat.open_dir(fat.root(), "boot")?;
    let file = fat.open(boot, "kernel.bin", OpenOptions::new().read())?;
    let mut bytes = [0; 64];
    let len = fat.read(&file, &mut bytes)?;
    assert_eq!(&bytes[..len], CONTENTS);
    fat.close(file)?;
    fat.unmount()?;

    let mut tree = Tree::new();
    tree.insert("boot/kernel.bin", Node::file(Content::bytes(CONTENTS)))?;
    let mut image = Vec::new();
    hadris::iso::sync::write(&mut image, &tree, &IsoOptions::default().with_rock_ridge())?;
    let mut iso = IsoFs::mount(image, MountOptions::new().read_only())?;
    let (bytes, len) = first_bytes(&mut iso, b"/boot/kernel.bin")?;
    assert_eq!(&bytes[..len], CONTENTS);
    iso.unmount()?;

    println!("V3 paths, generic FAT/ISO reads and embedded FAT handles verified");
    Ok(())
}

fn first_bytes<F: FileSystem>(
    fs: &mut F,
    path: &[u8],
) -> FsResult<([u8; 64], usize), F::DeviceError> {
    let node = fs.resolve(path, Resolve::Lexical)?;
    let mut bytes = [0; 64];
    let result = fs.read(node, 0, &mut bytes);
    fs.forget(node, 1);
    result.map(|len| (bytes, len))
}
