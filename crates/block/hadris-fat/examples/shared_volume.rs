use hadris_fat::time::TimeProvider;
use hadris_fat::{FatDateTime, FatVolume, FatVolumeBuilder};
use std::fs::File;
use std::sync::{Arc, Mutex};

#[derive(Debug)]
struct SystemClock;

impl TimeProvider for SystemClock {
    fn now(&self) -> FatDateTime {
        FatDateTime::now()
    }
}

static CLOCK: SystemClock = SystemClock;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "disk.img".to_owned());
    let file = File::options().read(true).write(true).open(path)?;
    let volume: FatVolume<_> = FatVolumeBuilder::new(file).time_provider(&CLOCK).open()?;

    // FatVolume is Send when its backing storage is Send. A mutex provides
    // exclusive access while Arc lets workers share ownership of the handle.
    let volume = Arc::new(Mutex::new(volume));
    let worker_volume = Arc::clone(&volume);
    let fat_type = std::thread::spawn(move || worker_volume.lock().unwrap().fat_type())
        .join()
        .expect("volume worker panicked");

    println!("mounted {fat_type}");
    Ok(())
}
