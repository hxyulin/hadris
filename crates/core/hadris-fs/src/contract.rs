use core::fmt;

use crate::ErrorKind;

/// A rule of the `FsDriver` contract that a driver broke, as reported by
/// the contract kit (`sync::contract::check` and its async twins).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ContractViolation {
    case: &'static str,
    rule: &'static str,
    found: Option<ErrorKind>,
}

impl ContractViolation {
    /// Creates a violation of `rule` in `case`. `found` is the error the
    /// driver returned, or `None` when it succeeded where it should not have
    /// or returned a wrong value.
    pub(crate) const fn new(
        case: &'static str,
        rule: &'static str,
        found: Option<ErrorKind>,
    ) -> Self {
        Self { case, rule, found }
    }

    /// The check that failed.
    pub const fn case(&self) -> &'static str {
        self.case
    }

    /// What the contract requires.
    pub const fn rule(&self) -> &'static str {
        self.rule
    }

    /// The error the driver returned, if any.
    pub const fn found(&self) -> Option<ErrorKind> {
        self.found
    }
}

impl fmt::Display for ContractViolation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.case, self.rule)?;
        match self.found {
            Some(kind) => write!(f, " (got {kind:?})"),
            None => f.write_str(" (it did not)"),
        }
    }
}

impl core::error::Error for ContractViolation {}
