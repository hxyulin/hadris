# hadris-apfs-cli

Inspect APFS containers.

```bash
hadris-apfs info /path/to/apfs-container-or-image
```

## Documentation

| Command | Description |
|---------|-------------|
| `info` | Display container and volume information |
| `cat` | Write a file's bytes to stdout |
| `ls` | List a directory (the volume root by default) |
| `stat` | Display inode metadata for a path |

Use `hadris-apfs <command> --help` for command options. All commands support
`--sector-size`, `--gpt` to select the first APFS GPT partition, and
`--partition` to select a partition by index.

- [Library API](https://docs.rs/hadris-apfs)

## License

Licensed under the [MIT license](../../../LICENSE-MIT).
