# hadris-cpio-raw

On-disk layouts and I/O-free codecs extracted from `hadris-cpio`.
Works with `no_std`, without an allocator, device, or sync/async feature.
The package starts at version 0.1.0 and versions independently of the driver.

```toml
[dependencies]
hadris-cpio-raw = "0.1.0"
```

Types are available at the crate root. The driver preserves the same types
under `hadris_cpio::raw` for existing users. Use `hadris-cpio` for
streaming archive reading and writing.

The first publication is part of the RC2 release. Until publication, use a
workspace path dependency.

## Documentation

- [API reference](https://docs.rs/hadris-cpio-raw/0.1.0)
- [Hadris documentation](https://hxyulin.github.io/hadris/)
- [`hadris-cpio`](../hadris-cpio), the driver using these codecs

## License

Licensed under the [MIT license](../../../LICENSE-MIT).
