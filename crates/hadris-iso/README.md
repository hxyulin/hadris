# ISO-RS (iso9660-rs)

A rust implementation of the ISO-9660 filesystem.
Currently there are a few limitations:

- Only supports spec compliant filesystems, and no extensions.
- Sector size has to be 2KiB (2048 bytes), this is the common size, and should be supported by most filesystems.

This project also contains the specification for the ISO-9660 filesystem, which is a work in progress.

## Current Progress

- [x] Parsing of the ISO-9660 filesystem
    - [x] Parsing of the primary volume descriptor
    - [x] Parsing of the volume descriptor list
    - [x] Parsing of the path table
    - [x] Parsing of the root directory
    - [x] Parsing of the directory records
- [x] Writing of the ISO-9660 filesystem
    - [x] Basic support for writing ISO-9660 filesystems (only to the root directory, but arbituary size)
    - [x] Support for writing to root directory bigger than 1 sector
    - [x] Support for writing to the path table
    - [x] Support for writing directories
    - [x] Support for writing files in non-root directories
- [x] Support for El Torito booting
   - [x] Support for parsing El Torito Boot Records
   - [x] Support for loading Boot files
- [ ] Testing   
    - [ ] Tests for reading basic structures
    - [ ] Tests for writing basic structures
    - [ ] Tests for reading and writing to the root directory
    - [ ] Tests for writing to the path table
    - [ ] Tests for writing directories
    - [ ] Tests for writing files in non-root directories

Other future goals:
Improve API to allow for more flexibility from users
