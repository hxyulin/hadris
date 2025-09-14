# Hadris ISO

# About
Hadris ISO is a library for creating and reading ISO images.

## Details

Despite the name, it is actually an implementation of the ECMA-119 standard, which includes ISO9660, but also ISO1999.

It includes many extensions, both official and non-official, here is a list of extensions and their support:
| Name | Supported | Notes |
|------|-----------|-------|
| El Torito | In Progress | Allows booting from ISO9660 |
| Rock Ridge | In Progress | Allows long file names, symlinks, and POSIX permissions |
| Joliet | Yes | Only supports Level 1 |
| ISO1999 | Yes | Enable with `long_file_names` option (up to 207 characters) |

Other than these official extensions, there are also many features that it supports:

- Hybrid Booting (MBR / GPT / APM) (WIP)
- Custom Filename Specifications (WIP)

## Goals

The goal of this library is to provide a conformant and feature-rich ISO image library, with the following goals:

- Be as feature-rich as possible
- Be as strict as possible, but also allow users to override certain settings to be non-strict
- Be as compatible as possible, implementing extensions
- Be as easy to use as possible (provide a simple API, CLI, and examples)
- Be as fast as possible (for now, it is not optimized for speed, as it is still in development)
- Safety is not specifically a goal, but it is a requirement, and is mostly achieved through the use of rust and the `bytemuck` crate

# Contributing
Contributions are welcome! Please feel free to open an issue or submit a pull request. Feature requests are also welcome, but please open an issue first to discuss the feature, as it could be outside the scope of this project.

# License

This project is licensed under the [MIT license](LICENSE-MIT).
This means that you are free to use the source code and the resulting binaries for any purpose, including commercial use.
