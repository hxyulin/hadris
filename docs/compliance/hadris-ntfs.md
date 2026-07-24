# `hadris-ntfs` compliance scope

The only pinned Microsoft source is a descriptive Master File Table overview
for NTFS version 3. The resulting informative mappings are in
[`spec/requirements/hadris-ntfs.json`](../../spec/requirements/hadris-ntfs.json).
They are not a full NTFS conformance profile.

The source does not define boot-sector fields, update-sequence arrays, mapping
pairs, directory-index wire layouts, attribute offsets, compression,
encryption, or filename encoding. Existing implementation and tests for those
areas remain useful, but cannot be labeled source-verified from this document.

The most important source-backed gap is attribute-list processing: files that
spill attributes into extension MFT records cannot yet be assembled.
