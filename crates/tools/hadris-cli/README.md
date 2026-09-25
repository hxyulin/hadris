# hadris-cli

`hadris` creates, inspects, extracts and checks FAT12/16/32, exFAT,
ISO 9660, UDF and cpio images, and ISO 9660 and UDF bridge images, with one
binary.

```bash
cargo install hadris-cli
```

Or build it from the workspace:

```bash
cargo build --release -p hadris-cli
```

## Commands

| Command | Subcommands |
|---|---|
| `hadris fat` | `info`, `stat`, `ls`, `tree`, `cat`, `extract`, `create`, `verify`, `fragmentation`, `chain` |
| `hadris iso` | `info`, `ls`, `tree`, `cat`, `extract`, `create`, `verify`, `mkisofs` |
| `hadris udf` | `info`, `ls`, `tree`, `cat`, `extract`, `create`, `verify`, `bridge`, `compare` |
| `hadris cpio` | `info`, `ls`, `cat`, `extract`, `create` |
| `hadris detect` | Lists every format an image or device holds |

`list` is an alias of `ls` and `check` of `verify` in every format. Run
`hadris <format> <command> --help` for the options of a command.

```bash
hadris detect disk.img
hadris fat create ./contents -o card.img --fat-type exfat
hadris iso create ./root -o disc.iso -J -R --boot boot/bios.img
hadris udf bridge ./root -o bridge.iso -J
hadris udf compare bridge.iso
hadris cpio create ./rootfs -o - > initramfs.cpio
hadris iso extract disc.iso -p /docs -o out
```

## Shared rules

- The image comes first, then the path inside it. Paths inside an image use
  `/` and may start with `/` or `./`: `/docs/a.txt`, `docs/a.txt` and
  `./docs/a.txt` name the same file in every format.
- `create` takes the source directory and `-o/--output`. An existing output
  is refused unless `-f/--force` is given, in which case a file is replaced
  once the new image is complete and a device is written in place. A failed
  `create` leaves no partial output. cpio also writes to standard output with
  `-o -`.
- Unreadable source entries are skipped with a warning, and images are dated
  with `SOURCE_DATE_EPOCH` when it is set.
- `extract` writes into `-o/--output` (default `.`), and `-p/--path` picks
  one file or directory. The image root is merged into it and any other path
  lands at `<output>/<name>`. Existing directories
  are merged, existing files and symlinks are never replaced, and nothing is
  written outside the output directory.
- `-V/--volume-name` names a new volume, and `-v/--verbose` prints more.
- `verify` exits non-zero when it finds a problem.

The 2.x binaries `hadris-fat`, `hadris-iso`, `hadris-udf`, `hadris-cpio`,
`hadris-cd`, `fatutil`, `cpioutil`, `hadris-iso-cli` and `hadris-udf-cli`
are replaced by these subcommands and no longer installed.

## Documentation

- [Detect and open images](https://hxyulin.github.io/hadris/guides/detect-open-images)
- [Read a FAT image](https://hxyulin.github.io/hadris/guides/read-fat-image)
- [Create a UDF or bridge image](https://hxyulin.github.io/hadris/creation/udf)
- [Library API](https://docs.rs/hadris)

## License

Licensed under the [MIT license](../../../LICENSE-MIT).
