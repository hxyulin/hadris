# boot_sector

A simple boot sector that prints "This image is not bootable" when loaded, which is embedded as a binary blob for use hadris.

## Building

To build the boot sector, run `cargo build --release` in the project root directory.
Afterwards, objcopy will be used to embed the boot sector into the binary.
```sh
$ cargo build --release
$ objcopy -I elf32-i386 -O binary target/boot_sector/release/boot_sector ../src/boot_sector.bin
```

## Running

You can test the boot sector by using qemu:
```sh
$ qemu-system-i386 -drive format=raw,file=../src/boot_sector.bin
```
