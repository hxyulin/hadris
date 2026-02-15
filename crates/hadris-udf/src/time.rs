//! UDF timestamp handling

/// UDF timestamp structure
///
/// Represents date and time in UDF format (ECMA-167 1/7.3)
#[repr(C)]
#[derive(Debug, Clone, Copy, Default, bytemuck::Zeroable, bytemuck::Pod)]
pub struct UdfTimestamp {
    /// Type and timezone
    /// Bits 0-11: Timezone offset in minutes from UTC (-1440 to 1440)
    /// Bits 12-15: Type (0=UTC, 1=local, 2=agreement)
    pub type_and_tz: u16,
    /// Year (1-9999)
    pub year: u16,
    /// Month (1-12)
    pub month: u8,
    /// Day (1-31)
    pub day: u8,
    /// Hour (0-23)
    pub hour: u8,
    /// Minute (0-59)
    pub minute: u8,
    /// Second (0-59)
    pub second: u8,
    /// Centiseconds (0-99)
    pub centiseconds: u8,
    /// Hundreds of microseconds (0-99)
    pub hundreds_of_microseconds: u8,
    /// Microseconds (0-99)
    pub microseconds: u8,
}

impl UdfTimestamp {
    /// Get the timezone type
    pub fn timezone_type(&self) -> TimezoneType {
        match (self.type_and_tz >> 12) & 0x0F {
            0 => TimezoneType::Utc,
            1 => TimezoneType::Local,
            2 => TimezoneType::Agreement,
            _ => TimezoneType::Reserved,
        }
    }

    /// Get the timezone offset in minutes from UTC
    ///
    /// Returns None if the timezone is not specified
    pub fn timezone_offset(&self) -> Option<i16> {
        let tz_type = self.timezone_type();
        if matches!(tz_type, TimezoneType::Utc | TimezoneType::Local) {
            let offset = (self.type_and_tz & 0x0FFF) as i16;
            // Sign extend from 12 bits
            let offset = if offset & 0x0800 != 0 {
                offset | !0x0FFF
            } else {
                offset
            };
            Some(offset)
        } else {
            None
        }
    }

    /// Check if this timestamp is valid
    pub fn is_valid(&self) -> bool {
        self.month >= 1
            && self.month <= 12
            && self.day >= 1
            && self.day <= 31
            && self.hour <= 23
            && self.minute <= 59
            && self.second <= 59
            && self.centiseconds <= 99
            && self.hundreds_of_microseconds <= 99
            && self.microseconds <= 99
    }
}

/// Timezone type
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TimezoneType {
    /// Coordinated Universal Time
    Utc,
    /// Local time
    Local,
    /// Agreed upon by sender and receiver
    Agreement,
    /// Reserved for future use
    Reserved,
}

#[cfg(test)]
mod tests {
    use super::*;

    static_assertions::const_assert_eq!(size_of::<UdfTimestamp>(), 12);

    #[test]
    fn test_timestamp_default() {
        let ts = UdfTimestamp::default();
        assert!(!ts.is_valid()); // Month and day are 0
    }

    #[test]
    fn test_timestamp_valid() {
        let ts = UdfTimestamp {
            type_and_tz: 0x1000, // UTC
            year: 2024,
            month: 1,
            day: 15,
            hour: 10,
            minute: 30,
            second: 45,
            centiseconds: 50,
            hundreds_of_microseconds: 25,
            microseconds: 10,
        };
        assert!(ts.is_valid());
        assert_eq!(ts.timezone_type(), TimezoneType::Local);
    }

    #[test]
    fn test_timezone_offset() {
        // UTC+0
        let ts = UdfTimestamp {
            type_and_tz: 0x0000, // UTC, offset 0
            ..Default::default()
        };
        assert_eq!(ts.timezone_offset(), Some(0));

        // UTC+5:30 (330 minutes)
        let ts = UdfTimestamp {
            type_and_tz: 0x014A, // UTC, offset 330
            ..Default::default()
        };
        assert_eq!(ts.timezone_offset(), Some(330));
    }
}
