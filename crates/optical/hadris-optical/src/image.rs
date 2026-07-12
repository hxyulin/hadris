use crate::detect::OpticalFormats;
use crate::error::OpticalFormat;

/// Policy used to choose one filesystem from an optical image.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum OpenPolicy {
    /// Prefer UDF on bridge images, falling back to ISO 9660.
    #[default]
    PreferUdf,
    /// Prefer ISO 9660 on bridge images, falling back to UDF.
    PreferIso9660,
    /// Require ISO 9660.
    Iso9660,
    /// Require UDF.
    Udf,
}

impl OpenPolicy {
    pub(crate) fn select(self, formats: OpticalFormats) -> Option<OpticalFormat> {
        match self {
            Self::PreferUdf if formats.udf().is_some() => Some(OpticalFormat::Udf),
            Self::PreferUdf if formats.has_iso9660() => Some(OpticalFormat::Iso9660),
            Self::PreferIso9660 if formats.has_iso9660() => Some(OpticalFormat::Iso9660),
            Self::PreferIso9660 if formats.udf().is_some() => Some(OpticalFormat::Udf),
            Self::Iso9660 if formats.has_iso9660() => Some(OpticalFormat::Iso9660),
            Self::Udf if formats.udf().is_some() => Some(OpticalFormat::Udf),
            _ => None,
        }
    }

    pub(crate) const fn required(self) -> Option<OpticalFormat> {
        match self {
            Self::Iso9660 => Some(OpticalFormat::Iso9660),
            Self::Udf => Some(OpticalFormat::Udf),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::detect::{OpticalFormats, UdfVrs};

    #[test]
    fn bridge_preferences_select_the_requested_default() {
        let formats = OpticalFormats::new(true, Some(UdfVrs::Nsr03));
        assert_eq!(
            OpenPolicy::PreferUdf.select(formats),
            Some(OpticalFormat::Udf)
        );
        assert_eq!(
            OpenPolicy::PreferIso9660.select(formats),
            Some(OpticalFormat::Iso9660)
        );
    }
}
