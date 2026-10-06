//! The exFAT reader session.

#![cfg_attr(target_os = "none", no_std, no_main)]

use hadris_example_firmware::sync::exfat;

#[cfg(target_os = "none")]
#[unsafe(no_mangle)]
extern "C" fn _start() -> ! {
    core::hint::black_box(exfat(hadris_example_firmware::Card));
    loop {
        core::hint::spin_loop();
    }
}

#[cfg(not(target_os = "none"))]
fn main() {
    use hadris_fat::exfat::{ExFatOptions, sync::format};
    use hadris_storage::{BlockSize, MemDevice};

    let mut dev = MemDevice::new(vec![0u8; 16 << 20], BlockSize::new(512).unwrap());
    format(&mut dev, &ExFatOptions::new()).expect("format");
    exfat(dev).expect("exFAT session");
    println!("exFAT session done");
}
