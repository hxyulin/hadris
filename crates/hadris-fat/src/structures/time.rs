use hadris_core::{str::FixedByteStr, UtcTime};

/// High precision Fat Time
/// Stores the time to the precision of a tenth of a second
/// For normal precision, use FatTime
#[derive(Clone, Copy)]
pub struct FatTimeHighP {
    /// The tenths of a second
    pub(crate) tenths: u8,
    pub(crate) time: FatTime,
}

impl FatTimeHighP {
    pub fn new(tenths: u8, time: u16, date: u16) -> Self {
        Self {
            tenths,
            time: FatTime::new(time, date),
        }
    }

    pub fn year(&self) -> u16 {
        self.time.year()
    }

    pub fn month(&self) -> u8 {
        self.time.month()
    }

    pub fn day(&self) -> u8 {
        self.time.day()
    }

    pub fn hour(&self) -> u8 {
        self.time.hour()
    }

    pub fn minute(&self) -> u8 {
        self.time.minute()
    }

    pub fn second(&self) -> u8 {
        self.time.second() + (self.tenths as u8 / 100)
    }

    pub fn hundreths(&self) -> u8 {
        self.tenths % 100
    }
}

impl core::fmt::Debug for FatTimeHighP {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        // So we display in this format:
        // MM/DD/YY HH:MM:SS.mm
        use core::fmt::Write;
        let mut str = FixedByteStr::<24>::new();
        write!(
            str,
            "{:02}/{:02}/{:04} {:02}:{:02}:{:02}.{:02}",
            self.month(),
            self.day(),
            self.year(),
            self.hour(),
            self.minute(),
            self.second(),
            self.hundreths()
        )
        .unwrap();

        f.debug_tuple("FatTimeHighP").field(&str.as_str()).finish()
    }
}

/// Fat Time
/// Stores the time to the precision of a second
#[derive(Clone, Copy)]
pub struct FatTime {
    /// The time of day (granularity is 2 seconds)
    /// It is stored like this:
    /// Bits 0-4: Seconds
    /// Bits 5-10: Minutes
    /// Bits 11-15: Hours
    pub(crate) time: u16,
    pub(crate) date: u16,
}

impl FatTime {
    pub fn new(time: u16, date: u16) -> Self {
        Self { time, date }
    }

    pub fn year(&self) -> u16 {
        ((self.date >> 9) & 0x7F) + 1980
    }

    pub fn month(&self) -> u8 {
        ((self.date >> 5) & 0x0F) as u8
    }

    pub fn day(&self) -> u8 {
        (self.date & 0x1F) as u8
    }

    pub fn hour(&self) -> u8 {
        ((self.time >> 11) & 0x1F) as u8
    }

    pub fn minute(&self) -> u8 {
        ((self.time >> 5) & 0x3F) as u8
    }

    pub fn second(&self) -> u8 {
        (self.time & 0x1F) as u8
    }
}

impl core::fmt::Debug for FatTime {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        // So we display in this format:
        // MM/DD/YY HH:MM:SS, which is 17 characters, but we make a 20 byte string
        use core::fmt::Write;
        let mut str = FixedByteStr::<20>::new();
        let year = (self.date >> 9) & 0x7F + 1980;
        let month = (self.date >> 5) & 0x0F;
        let day = self.date & 0x1F;
        let hour = self.time >> 11;
        let minute = (self.time >> 5) & 0x3F;
        let second = self.time & 0x1F;
        write!(
            str,
            "{:02}/{:02}/{:04} {:02}:{:02}:{:02}",
            month, day, year, hour, minute, second
        )
        .unwrap();

        f.debug_tuple("FatTime").field(&str.as_str()).finish()
    }
}

#[cfg(feature = "std")]
mod std_impls {
    use super::*;

    impl TryFrom<std::time::SystemTime> for FatTimeHighP {
        type Error = &'static str;

        fn try_from(value: std::time::SystemTime) -> Result<Self, Self::Error> {
            let date_time: chrono::DateTime<chrono::Utc> = value.into();
            Self::try_from(date_time)
        }
    }

    impl TryFrom<std::time::SystemTime> for FatTime {
        type Error = &'static str;

        fn try_from(value: std::time::SystemTime) -> Result<Self, Self::Error> {
            let date_time: chrono::DateTime<chrono::Utc> = value.into();
            Self::try_from(date_time)
        }
    }
}

impl TryFrom<UtcTime> for FatTimeHighP {
    type Error = &'static str;

    fn try_from(value: UtcTime) -> Result<Self, Self::Error> {
        use chrono::{Datelike, Timelike};

        // Compute FAT date fields
        let year = value.year();
        if year < 1980 || year > 2107 {
            return Err("Year out of FAT32 range (1980-2107)");
        }

        let year_fat = (year - 1980) as u16;
        let month = value.month() as u16;
        let day = value.day() as u16;

        // Compute FAT time fields
        let hour = value.hour() as u16;
        let minute = value.minute() as u16;
        let second = (value.second() / 2) as u16; // FAT stores seconds in 2-second increments
        let hundreths = (value.timestamp_subsec_millis() / 10) as u8; // FAT stores hundredths

        // Encode to FAT format
        let time = (hour << 11) | (minute << 5) | second;
        let date = (year_fat << 9) | (month << 5) | day;

        Ok(Self::new(hundreths, time, date))
    }
}

impl TryFrom<UtcTime> for FatTime {
    type Error = &'static str;

    fn try_from(value: UtcTime) -> Result<Self, Self::Error> {
        use chrono::{Datelike, Timelike};

        // Compute FAT date fields
        let year = value.year();
        if year < 1980 || year > 2107 {
            return Err("Year out of FAT32 range (1980-2107)");
        }

        let year_fat = (year - 1980) as u16;
        let month = value.month() as u16;
        let day = value.day() as u16;

        // Compute FAT time fields
        let hour = value.hour() as u16;
        let minute = value.minute() as u16;
        let second = (value.second() / 2) as u16; // FAT stores seconds in 2-second increments

        // Encode to FAT format
        let time = (hour << 11) | (minute << 5) | second;
        let date = (year_fat << 9) | (month << 5) | day;

        Ok(Self::new(time, date))
    }
}
