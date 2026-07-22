# Hadris 2 optical opening

`hadris-optical` provides a category-level opener when its `open` feature and
an I/O mode are enabled:

```rust,ignore
let image = hadris_optical::sync::OpenOpticalImage::open(
    &mut source,
    hadris_optical::OpenPolicy::PreferUdf,
)?;
```

Detection reports ISO 9660 and UDF independently. `OpenPolicy` makes
bridge-image selection explicit: `PreferUdf` (the default) falls back to ISO,
`PreferIso9660` falls back to UDF, and `Iso9660`/`Udf` require that exact format.

The enum retains the concrete ISO or UDF handle. Callers can inspect `format`,
use `as_iso9660` or `as_udf`, and recover the original borrowed source with
`into_inner`. `open_detected` accepts a prior `OpticalFormats` result. Sync and
async APIs have the same shape.

One opener deliberately owns one filesystem handle. Opening both sides of a
bridge simultaneously requires a future shared or reopenable source adapter.

## Writer qualification

`OpticalImageWriter` bridge images are continuously opened through both the ISO 9660 and
UDF policies. Detection remains only a probe: the unified opener still performs
and propagates each concrete reader's full validation.
