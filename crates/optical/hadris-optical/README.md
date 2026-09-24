# hadris-optical

`hadris-optical` detects and opens optical images: ISO 9660 and UDF on any
`hadris-storage` block device. It sits next to the format crates it builds
on and re-exports them, with the hybrid image writer `hadris-cd` behind the
`cd` feature.

Detection reports ISO 9660 and UDF independently, because a bridge image
holds both.

```toml
[dependencies]
hadris-optical = "2.4.0"
```

```rust,no_run
use hadris_fs::sync::DriverExt;
use hadris_optical::{OpenPolicy, sync::OpenOpticalImage};

let file = std::fs::File::open("disc.iso")?;
let mut image = OpenOpticalImage::open(file, OpenPolicy::PreferUdf)?;
println!("{:?}", image.format());
for entry in image.read_dir("/")? {
    println!("{:?}", entry?.name());
}
# Ok::<(), Box<dyn std::error::Error>>(())
```

- `detect` reads the volume descriptors after byte 32768 on devices with
  blocks of up to 4096 bytes, without an allocator, in each mode.
- `OpenOpticalImage` detects and mounts the filesystem an `OpenPolicy`
  selects: UDF or ISO 9660 first on a bridge image, or one of them only.
  ISO 9660 opens as an `IsoView` of the preferred namespace. It implements
  the `hadris-fs` `FsDriver` trait read-only by delegating to the driver,
  so the path helpers, `Volume` and handles work on either; `as_iso` and
  `as_udf` reach the drivers' native API.
- Every failure is an `Error<E>` with a shared `ErrorKind`, a `Detail` and
  the device's own error, and a failed open gives the device back in an
  `OpenError`.

## Features

| Feature | Default | Purpose |
|---------|---------|---------|
| `std` | yes | Implies `alloc`; `std::io::Error` conversions and `std::fs::File` devices |
| `alloc` | via `std` | `AnyError` conversions and the ISO 9660 and UDF writers |
| `sync` | yes | The blocking API in `sync` |
| `async` | no | The asynchronous API in `r#async` |
| `async-send` | no | The asynchronous API with `Send` futures in `async_send`; enables `async` |
| `cd` | no | Re-export `hadris-cd`, the hybrid image writer; implies `alloc` |

No feature changes what an item does. For format-specific controls, use the
re-exported `iso`, `udf` and `cd` crates directly.

## Documentation

- [Choose a crate](https://hxyulin.github.io/hadris/crates)
- [Detect and open images](https://hxyulin.github.io/hadris/guides/detect-open-images)
- [API reference](https://docs.rs/hadris-optical)

## License

Licensed under the [MIT license](../../../LICENSE-MIT).
