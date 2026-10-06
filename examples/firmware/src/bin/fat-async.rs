//! Every FAT call on the async API, polled by a minimal executor.

#![cfg_attr(target_os = "none", no_std, no_main)]

use hadris_example_firmware::local::{block_on, fat};
use hadris_fat::embedded::Options;

#[cfg(target_os = "none")]
#[unsafe(no_mangle)]
extern "C" fn _start() -> ! {
    core::hint::black_box(block_on(fat(hadris_example_firmware::Card, Options::new())));
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
    block_on(fat(dev, Options::new())).expect("FAT session");
    println!("async FAT session done");
}
