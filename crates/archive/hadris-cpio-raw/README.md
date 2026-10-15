# hadris-cpio-raw

On-disk layouts and I/O-free codecs extracted from `hadris-cpio`.
Works with `no_std`, without an allocator, device, or sync/async feature.
The package starts at version 0.1.0 and versions independently of the driver.

```toml
[dependencies]
hadris-cpio-raw = "0.1.0"
```

Every item is at the crate root. `hadris-cpio` builds on this crate but
does not re-export it, so this crate can change in a 0.x minor release
without a new major version of `hadris-cpio`. Use `hadris-cpio` for
streaming archive reading and writing.

## Documentation

- [API reference](https://docs.rs/hadris-cpio-raw/0.1.0)
- [Hadris documentation](https://hxyulin.github.io/hadris/)
- [`hadris-cpio`](../hadris-cpio), the driver using these codecs

## License

Licensed under the [MIT license](../../../LICENSE-MIT).
