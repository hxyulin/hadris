# Hadris

A filesystem crate, written in rust. Designed to be fast, portable, and supports no-std environments. 
Currently, there are many other subcrates that can be used to interact with the API in a more user-friendly way.

See [Subcrates](##subcrates) for more information.

## Features

- Fast
- Portable
- No-std

## Subcrates

### Hadris

The main crate, which contains the API for interacting with the filesystem. By default, it uses allocations to allow for runtime 
file system type creation (e.g. user can choose to create a FAT32 filesystem, or ext4 filesystem).
If you want to use it in a no-std environment, you can disable default features, or use the subcrate for the specific filesystem type.

### Hadris-core
The core subcrate for Hadris. It contains the base traits and types for Hadris. You can also use this subcrate to create your own filesystem types.
Contributions are welcome! For more information see [Hadris-core](https://docs.rs/hadris-core)

### Hadris-fat
A subcrate for the FAT filesystem.
Currently, it is not fully implemented, but allows basic file reading and writing to a FAT32 filesystem. LFN support is planned.
For more information see [Hadris-fat](https://docs.rs/hadris-fat)

### Hadris-cli
A subcrate for the CLI.
The CLI uses clap to parse arguments, and allows reading and writing to the root directory. As well as creating a FAT32 filesystem on an image file (normal file).

For more information see [Hadris-cli](https://docs.rs/hadris-cli)
