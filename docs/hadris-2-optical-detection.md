# Hadris 2.0 Optical Detection

Status: implemented initial detection slice

Optical images cannot always be classified as exactly one filesystem. UDF bridge
images deliberately combine ISO 9660 descriptors with a UDF Volume Recognition
Sequence so different readers can select the representation they support.

## API model

`hadris-optical` therefore returns a set-like result:

```rust
pub struct OpticalFormats { /* private fields */ }

impl OpticalFormats {
    pub const fn has_iso9660(self) -> bool;
    pub const fn udf(self) -> Option<UdfVrs>;
    pub const fn is_bridge(self) -> bool;
    pub const fn is_empty(self) -> bool;
}

pub enum UdfVrs { Nsr02, Nsr03 }
```

Both `detect::sync::detect` and `detect::async::detect` scan optical volume
descriptors at sectors 16 through 31 and restore the source's original position.
They return `Ok(None)` when neither supported filesystem is found.

ISO detection requires a version-1 `CD001` descriptor. UDF is reported only
after a complete, ordered `BEA01`, `NSR02` or `NSR03`, and `TEA01` sequence.
Incidental or incomplete NSR descriptors are not accepted.

## Opening policy

Detection does not choose which filesystem to open. A future `OpenOpticalImage`
API will accept a caller policy such as prefer UDF, prefer ISO, or open both. It
must preserve concrete ISO and UDF handles and must not silently hide one side of
a bridge image.

Before that wrapper is added, the next prerequisite is recoverable borrowed
opening for both `IsoImage` and the canonical future `UdfVolume`, following the
ownership rules established by `hadris-block`.
