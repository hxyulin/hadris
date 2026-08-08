# hadris-optical

`hadris-optical` is the optical-media facade for Hadris. It provides
non-destructive ISO 9660/UDF detection, policy-based opening, and hybrid
ISO+UDF image creation while retaining lossless access to the concrete format
handles.

Detection reports ISO 9660 and UDF independently because bridge images can
validly contain both filesystems.

```toml
[dependencies]
hadris-optical = "2.0.0"
```

```rust,no_run
use hadris_optical::{OpenPolicy, sync::OpenOpticalImage};
use std::fs::File;

let mut image = File::open("disc.iso")?;
let opened = OpenOpticalImage::open(&mut image, OpenPolicy::PreferUdf)?;

if let Some(udf) = opened.as_udf() {
    println!("UDF volume: {}", udf.info().volume_id);
}
# Ok::<(), Box<dyn std::error::Error>>(())
```

`OpenPolicy` can require one format or express a preference for bridge images.
The returned enum preserves the underlying `hadris-iso` or `hadris-udf`
handle.

## Features

| Feature | Default | Purpose |
|---------|---------|---------|
| `std` | yes | Hosted support; enables `alloc` |
| `alloc` | yes | Heap-backed APIs without requiring `std` |
| `sync` | yes | Synchronous I/O APIs |
| `async` | no | Asynchronous read/open APIs |
| `read` | yes | ISO and UDF reading |
| `write` | yes | Leaf-format writing |
| `detect` | via `open` | Non-destructive ISO/UDF detection |
| `open` | yes | Detection plus policy-based opening |
| `iso` | via `open` | Re-export `hadris-iso` |
| `udf` | via `open` | Re-export `hadris-udf` |
| `cd` | yes | Re-export the synchronous hybrid image writer |

The hybrid `cd` writer is currently synchronous. For format-specific controls,
use the re-exported `iso`, `udf`, and `cd` modules directly.
