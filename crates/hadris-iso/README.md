# Hadris ISO

# About
Hadris ISO is a library for creating and reading ISO images.

## Details

Despite the name, it is actually an implementation of the ECMA-119 standard, which includes ISO9660, but also ISO1999.

It includes many extensions, both official and nonofficial, here is a list of extensions and their support:
| Name | Supported | Notes |
|------|-----------|-------|
| El Torito | Yes | Allows booting from ISO9660 |
| Rock Ridge | In Progress | Allows long file names, symlinks, and POSIX permissions |
| Joliet | Planned | Support UTF-16 filenames |
| ISO1999 | Planned | Not commonly used |

Other than these official extensions, there are also many features that it supports:

- Hybrid Booting (MBR / GPT / APM)
- Non conformant filenames

## Goals

The goal of this library is to provide a conformant and feature-rich ISO image library, with the following goals:

- Be as feature-rich as possible
- Be as strict as possible, but also allow users to override certain settings to be non-strict
- Be as compatible as possible, implementing extensions
- Be as easy to use as possible (provide a simple API, CLI, and examples)
- Be as fast as possible (for now, it is not optimized for speed, as it is still in development)
- Safety is not specifically a goal, but it is a requirement, and is mostly achieved through the use of rust and the `bytemuck` crate

# Usage

The usage documentation is provided for the 'hadris-iso' crate, but if you want to use bindings for other languages, you can use the 'hadris-iso-bindings' crate, or the 'hadris-iso-cli' for the CLI interface.

To add the crate to your project, add the following to your Cargo.toml:

```toml
[dependencies]
hadris-iso = "0.1.2"
```

## Creating an ISO image

By default, images that are created and read will use the default strictness level, which is `Strictness::Default`.
This means it will create the image even if it is not strictly conformant, and allows reads from non conformant images.
This is the lowest level of strictness, and is the default. The highest level of strictness ensures conformance in every way,
allowing for maximum compatibility with other tools. There is also a `Strictness::Compatible` level, which is designed to be as
compatible as possible, but may not be as conformant from the spec (due to how other tools handle the image).

Creating an image is as simple as specifying the files and options, and then calling `format_new` on the `IsoImage` struct.
If formatting on a file you can use `format_file`, which will create the file if it doesn't exist, or overwrite it if it does.
This simplifies the process, as you do not need to manually call 'FormatOptions::image_len' to determine the size of the image.
This is an example of creating an image from a directory using default options:
```rust
use hadris_iso::{IsoImage, FileInput, FormatOptions};
use std::path::PathBuf;

let options = FormatOptions::new()
    .with_files(FileInput::from_fs(PathBuf::from("path/to/files")).unwrap());
let file = IsoImage::format_file(PathBuf::from("path/to/image"), options).unwrap();
```

# Contributing
Contributions are welcome! Please feel free to open an issue or submit a pull request. Feature requests are also welcome, but please open an issue first to discuss the feature, as it could be outside the scope of this project.

# License

This project is licensed under the [MIT license](LICENSE-MIT).
This means that you are free to use the source code and the resulting binaries for any purpose, including commercial use.
