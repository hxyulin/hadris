---
title: Adapt a custom device
---

# Adapt a custom device or firmware reader

Format crates consume `hadris-io` traits rather than requiring `std::io`.
Hosted `std::io` readers work directly. Embedded devices implementing
`embedded-io` can be wrapped without erasing their source error.

```toml
[dependencies]
embedded-io = "0.7"
hadris-fat = {
  version = "2.0.0",
  default-features = false,
  features = ["read", "sync"]
}
hadris-io = { version = "2.0.0", default-features = false, features = ["sync"] }
```

```rust,ignore
use embedded_io::{ErrorType, Read, Seek, SeekFrom};

struct FirmwareDisk {
    // Firmware protocol handle and current position.
}

impl ErrorType for FirmwareDisk {
    type Error = DeviceError;
}

impl Read for FirmwareDisk {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize, Self::Error> {
        // Read from the firmware or device protocol into `buf`.
        todo!()
    }
}

impl Seek for FirmwareDisk {
    fn seek(&mut self, position: SeekFrom) -> Result<u64, Self::Error> {
        // Validate the requested position and update the device cursor.
        todo!()
    }
}

let disk = FirmwareDisk { /* ... */ };
let reader = hadris_io::sync::FromEmbedded::new(disk);
let volume = hadris_fat::sync::FatVolume::open(reader)?;
# Ok::<(), hadris_fat::Error>(())
```

The device must provide the access pattern required by the format. Mounted
filesystems generally need `Read + Seek`; mutation adds `Write`. Return short
reads only when the device genuinely has fewer bytes available, and reject
seeks outside the device rather than wrapping arithmetic.

For logical-block-native hardware, implement the traits in `hadris-storage`
and use its seekable block-device adapter. Keep the physical block size and the
filesystem's logical sector size distinct.

For memory-backed parsing without `std`, use `hadris_io::Cursor` over a caller
provided byte slice.
