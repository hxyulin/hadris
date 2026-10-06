//! A FAT data logger with the default ASCII fold.

#![cfg_attr(target_os = "none", no_std, no_main)]

use hadris_example_firmware::sync::log;
use hadris_fat::embedded::Options;

#[cfg(target_os = "none")]
#[unsafe(no_mangle)]
extern "C" fn _start() -> ! {
    core::hint::black_box(log(hadris_example_firmware::Card, Options::new()));
    loop {
        core::hint::spin_loop();
    }
}

#[cfg(not(target_os = "none"))]
fn main() {
    use hadris_fat::{FatOptions, sync::format};
    use hadris_storage::{BlockSize, MemDevice};

    let mut dev = MemDevice::new(vec![0u8; 4 << 20], BlockSize::new(512).unwrap());
    format(&mut dev, &FatOptions::new()).expect("format");
    log(dev, Options::new()).expect("FAT log session");
    println!("FAT log session done");
}
