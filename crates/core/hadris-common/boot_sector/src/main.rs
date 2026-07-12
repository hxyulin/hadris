#![no_std]
#![no_main]

use core::panic::PanicInfo;
use core::arch::global_asm;

global_asm!(include_str!("boot_sector.s"));

#[used]
#[unsafe(link_section = ".data")]
#[unsafe(export_name = "message")]
pub static MESSAGE: &[u8] = b"This image is not bootable. Press Ctrl+Alt+Del to restart.\0";

#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    loop {}
}
