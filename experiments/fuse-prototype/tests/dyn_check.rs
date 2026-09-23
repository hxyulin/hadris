//! Whether the sync `FileSystem` trait is usable as a trait object once its
//! device error is fixed, which bears on how much `DynFileSystem` must do.

use std::fs::OpenOptions;

use hadris_fat::sync::{FatFs, format};
use hadris_fat::{FormatOptions, MountOptions};
use hadris_fs::sync::{FileSystem, StdMutex, Volume};
use hadris_fs::{HeapTable, Name};

#[test]
fn sync_file_system_is_dyn_compatible() {
    let path = std::env::temp_dir().join(format!("hadris-fuse-dyn-{}.img", std::process::id()));
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(true)
        .open(&path)
        .unwrap();
    file.set_len(8 << 20).unwrap();
    let file = format(file, FormatOptions::new()).unwrap().into_inner();
    let _ = std::fs::remove_file(&path);
    let fs = FatFs::open_with(file, MountOptions::new().with_table(HeapTable::new())).unwrap();
    let vol: Box<dyn FileSystem<DeviceError = std::io::Error> + Send + Sync> =
        Box::new(Volume::<_, StdMutex>::new(fs));
    let root = vol.root();
    let err = vol.lookup(root, Name::new("missing").unwrap()).unwrap_err();
    assert_eq!(err.kind(), hadris_fs::ErrorKind::NotFound);
    let node = vol.resolve("/").unwrap();
    assert_eq!(node, root);
}
