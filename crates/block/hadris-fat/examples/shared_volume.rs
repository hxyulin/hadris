use std::fs::File;
use std::sync::Arc;

use hadris_fat::sync::FatFs;
use hadris_fs::MountOptions;
use hadris_fs::sync::{FileSystem, Volume};
use hadris_fs::{OpenOptions, SystemClock};
use hadris_storage::host::FileDevice;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "disk.img".to_owned());
    let file = File::options().read(true).write(true).open(path)?;
    let fs = FatFs::mount(
        FileDevice::new(file)?,
        MountOptions::new().with_clock(&SystemClock),
    )?;
    let kind = fs.kind();

    // Volume puts the filesystem behind a lock, so its path methods work on
    // `&self` and an Arc shares it between threads.
    let volume = Arc::new(Volume::new(fs));
    let worker = Arc::clone(&volume);
    std::thread::spawn(move || -> Result<(), hadris_fs::Error<std::io::Error>> {
        let options = OpenOptions::new().write().create().truncate();
        let mut file = worker.open("/hello.txt", options)?;
        file.write(b"hello from a thread")?;
        file.close()
    })
    .join()
    .expect("volume worker panicked")?;
    volume.lock().sync()?;

    println!("mounted {kind:?}, wrote /hello.txt");
    Ok(())
}
