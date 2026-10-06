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
| `hadris apfs` | `info`, `ls`, `stat`, `cat`, `extract` (experimental, read-only) |
| `hadris cpio` | `info`, `ls`, `cat`, `extract`, `create` |
| `hadris detect` | Lists every format an image or device holds |

`list` is an alias of `ls` and `check` of `verify` in every format. Run
`hadris <format> <command> --help` for the options of a command.

```bash
hadris apfs info container.img
hadris apfs ls container.img / --volume-name Data
hadris apfs cat container.img /notes.txt --volume 1
hadris apfs extract container.img /docs --volume 1 -o out
hadris detect disk.img
hadris fat create ./contents -o card.img --fat-type exfat
hadris iso create ./root -o disc.iso -J -R --boot boot/bios.img
hadris udf bridge ./root -o bridge.iso -J
hadris udf compare bridge.iso
hadris cpio create ./rootfs -o - > initramfs.cpio
hadris iso extract disc.iso -p /docs -o out
```

FAT12/16/32 extraction uses a bounded one-entry listing hint by default. Tune
`hadris fat extract` independently for the access pattern:

```bash
hadris fat extract disk.img -o out --cache-directory-entries 256 --cache-blocks 8
hadris fat extract disk.img -o out --cache-chain-positions 64
hadris fat extract disk.img -o out --read-ahead-blocks 128
hadris fat extract disk.img -o out --no-directory-hint
hadris fat extract disk.img -o out --no-cache
```

Metadata blocks, chain positions and the directory-prefix index default to zero
for extraction. Each bound accepts zero to disable that component. `--no-cache`
disables every optional cache and cannot be combined with the other cache flags.
The explicit FAT cache settings are refused on exFAT images; exFAT extraction
continues to work with its existing defaults or `--no-cache`.
`--read-ahead-blocks` applies to both FAT and exFAT. It defaults to zero; 128
blocks allow up to 64 KiB of storage buffering for 512-byte sectors. Read-ahead
expands adjacent reads and keeps scattered misses at the requested size.

APFS opens a container's sole volume automatically. Multi-volume containers
require one selector: `--volume` (the APFS `fs_index` displayed by `info`),
`--volume-name`, `--volume-object-id`, or `--volume-uuid`. Ambiguous names are
refused. `info` lists volume names, object IDs and UUIDs without selecting one.
For whole-disk images, `--gpt` opens the sole APFS GPT partition; multiple
APFS partitions require `--partition <index>`. `--partition` also supports
MBR entries. `--sector-size` selects the disk's logical sector size (default
512 bytes). Password-encrypted, single-key software APFS volumes accept
`--password-stdin` on `ls`, `stat`, `cat` and `extract`; the password is one line
from standard input, with only its LF/CRLF terminator removed. Input is limited
to 4096 bytes including the line ending; a line that reaches the limit without
an LF terminator is rejected. The password buffer is reserved before reading
and zeroized after use. Add
`--crypto-user <uuid>` to select a crypto user explicitly. Passwords are not
accepted as command-line arguments. `info` inspects volume identity without
unlocking. APFS compression, hardware/per-file encryption, snapshots and writing
remain unsupported.

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

- [Migrating from the 2.x CLIs](https://github.com/hxyulin/hadris/blob/main/docs/hadris-3.0.0-migration.md#command-line-tools)
- [Detect and open images](https://hxyulin.github.io/hadris/guides/detect-open-images)
- [Read a FAT image](https://hxyulin.github.io/hadris/guides/read-fat-image)
- [Create a UDF or bridge image](https://hxyulin.github.io/hadris/creation/udf)
- [Library API](https://docs.rs/hadris)

## License

Licensed under the [MIT license](../../../LICENSE-MIT).
