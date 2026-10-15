# Hadris 2.0.0-rc.3 Release Notes

Hadris 2.0.0-rc.3 is the final planned API-reset candidate before 2.0.0. It
corrects contracts discovered during the RC2 optical-format audit. It must soak
for seven days; any functional change requires RC4 and restarts that period.

## Compatibility changes

- `std` and `sync` are independent in `hadris-io` and `hadris-common`.
- ISO interchange level 3 is represented by `BaseIsoLevel::Level3`; it is no
  longer conflated with the ISO 9660:1999 enhanced filename namespace.
- Allocation-free ISO readers expose `IsoNamespace::Enhanced` and prefer
  Joliet, then enhanced, then primary roots.

## Correctness fixes

- ISO 9660:1999 enhanced roots are discovered by both reader surfaces, and
  `has_evd()` has its standards-defined meaning rather than acting as a UDF
  heuristic.
- UDF directory layouts use exact padded FID sizes.
- UDF filenames use conforming OSTA CS0 8-bit or 16-bit encoding with encoded
  identifier length enforcement.

## Promotion policy

Promotion to 2.0.0 changes only package versions, lockfiles, changelog/release
metadata, the tag, and publication metadata. The functional source must remain
identical to this candidate.
