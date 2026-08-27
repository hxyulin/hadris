//! Single test binary for every format. Tests are addressed as
//! `<format>::<topic>::<name>`, so `cargo test fat::` or
//! `cargo test iso::boot::` selects a slice of the suite.

mod fat;
mod iso;
