use core::fmt;

use crate::{DetailCode, Location};

/// How much a [`Finding`] matters, ordered from `Notice` to `Error`.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Severity {
    /// Worth knowing, not wrong: a dirty flag, a stale free count.
    Notice,
    /// Allowed by the specification but likely to trip other
    /// implementations.
    Warning,
    /// The volume violates its format; data may be lost or misread.
    Error,
}

/// One problem a checker found: a static message, the format's detail code,
/// a severity, where on the device, and the path of the node it belongs to.
///
/// The detail code is the one the format's errors use, read back with the
/// format's `Detail::from_code`. The path borrows the checker's scratch
/// buffer, so a finding lives only for the callback; copy what must
/// outlive it.
#[derive(Clone, Copy)]
pub struct Finding<'a> {
    message: &'static str,
    detail: DetailCode,
    severity: Severity,
    location: Option<Location>,
    path: Option<&'a [u8]>,
}

impl<'a> Finding<'a> {
    /// A finding with severity [`Severity::Error`] until
    /// [`with_severity`](Self::with_severity) sets another.
    pub const fn new(message: &'static str, detail: DetailCode) -> Self {
        Self {
            message,
            detail,
            severity: Severity::Error,
            location: None,
            path: None,
        }
    }

    /// Sets the severity.
    pub const fn with_severity(self, severity: Severity) -> Self {
        Self { severity, ..self }
    }

    /// Sets where on the device the problem is.
    pub const fn with_location(self, location: Location) -> Self {
        Self {
            location: Some(location),
            ..self
        }
    }

    /// Sets the path of the node the problem belongs to.
    pub const fn with_path(self, path: &'a [u8]) -> Self {
        Self {
            path: Some(path),
            ..self
        }
    }

    /// What is wrong.
    pub const fn message(&self) -> &'static str {
        self.message
    }

    /// The format's detail code.
    pub const fn detail(&self) -> DetailCode {
        self.detail
    }

    /// How much it matters.
    pub const fn severity(&self) -> Severity {
        self.severity
    }

    /// Where on the device, when known.
    pub const fn location(&self) -> Option<Location> {
        self.location
    }

    /// The path from the root of the node the finding is about, `None` for
    /// volume-level findings.
    pub const fn path(&self) -> Option<&'a [u8]> {
        self.path
    }
}

struct Escaped<'a>(&'a [u8]);

impl fmt::Display for Escaped<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for chunk in self.0.utf8_chunks() {
            f.write_str(chunk.valid())?;
            for byte in chunk.invalid() {
                write!(f, "\\x{byte:02x}")?;
            }
        }
        Ok(())
    }
}

impl fmt::Debug for Escaped<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "\"{self}\"")
    }
}

/// `message: path (location)`, with path bytes that are not UTF-8 escaped.
impl fmt::Display for Finding<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.message)?;
        if let Some(path) = self.path {
            write!(f, ": {}", Escaped(path))?;
        }
        if let Some(location) = self.location {
            write!(f, " ({location})")?;
        }
        Ok(())
    }
}

impl fmt::Debug for Finding<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Finding")
            .field("message", &self.message)
            .field("detail", &self.detail)
            .field("severity", &self.severity)
            .field("location", &self.location)
            .field("path", &self.path.map(Escaped))
            .finish()
    }
}

/// What a check returns once it has passed every finding to its callback.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CheckReport {
    findings: u64,
    passes: u32,
}

impl CheckReport {
    /// A report of `findings` findings over `passes` walks of the tree.
    pub const fn new(findings: u64, passes: u32) -> Self {
        Self { findings, passes }
    }

    /// The number of findings.
    pub const fn findings(&self) -> u64 {
        self.findings
    }

    /// Whether nothing was found.
    pub const fn is_clean(&self) -> bool {
        self.findings == 0
    }

    /// How many times the tree was walked: once for each window of
    /// clusters the checker's scratch buffer covers.
    pub const fn passes(&self) -> u32 {
        self.passes
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::format;

    #[test]
    fn display_escapes_path_bytes() {
        let finding = Finding::new("lost clusters", DetailCode::new("test", 1))
            .with_path(b"/a\xffb")
            .with_location(Location::Cluster(7));
        assert_eq!(format!("{finding}"), "lost clusters: /a\\xffb (cluster 7)");
        assert_eq!(finding.severity(), Severity::Error);
        assert!(Severity::Notice < Severity::Warning && Severity::Warning < Severity::Error);
        assert_eq!(
            format!("{}", Finding::new("dirty", DetailCode::new("test", 2))),
            "dirty"
        );
    }
}
