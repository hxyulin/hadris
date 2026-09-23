use std::fs::File;
use std::sync::Arc;

use hadris_fat::MountOptions;
use hadris_fat::sync::FatFs;
use hadris_fs::SystemClock;
use hadris_fs::sync::{FileSystem, PathExt, Volume};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "disk.img".to_owned());
    let file = File::options().read(true).write(true).open(path)?;
    let fs = FatFs::open_with(file, MountOptions::new().with_clock(SystemClock))?;
    let kind = fs.kind();

    // Volume puts the driver behind a lock, so its path helpers work on
    // `&self` and an Arc shares it between threads.
    let volume = Arc::new(Volume::new(fs));
    let worker = Arc::clone(&volume);
    std::thread::spawn(move || worker.write_file("/hello.txt", b"hello from a thread"))
        .join()
        .expect("volume worker panicked")?;
    volume.sync()?;

    println!("mounted {kind:?}, wrote /hello.txt");
    Ok(())
}
