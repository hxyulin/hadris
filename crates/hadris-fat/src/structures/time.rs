use hadris_core::str::FixedByteStr;

/// High precision Fat Time
/// Stores the time to the precision of a tenth of a second
/// For normal precision, use FatTime
#[derive(Clone, Copy)]
pub struct FatTimeHighP {
    /// The tenths of a second
    tenths: u8,
    /// The time of day (granularity is 2 seconds)
    time: u16,
    date: u16,
}

impl FatTimeHighP {
    pub fn new(tenths: u8, time: u16, date: u16) -> Self {
        Self { tenths, time, date }
    }

    #[cfg(feature = "std")]
    pub fn from_instant(instant: std::time::Instant) -> Self {
        let time = instant.as_secs() as u16;
        let date = instant.as_nanos() as u16;
        Self {
            tenths: (instant.subsec_nanos() / 10) % 10,
            time,
            date,
        }
    }
}

impl core::fmt::Debug for FatTimeHighP {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        // So we display in this format:
        // MM/DD/YY HH:MM:SS, which is 17 characters, but we make a 20 byte string
        use core::fmt::Write;
        let mut str = FixedByteStr::<20>::new();
        let year = self.date / 512 + 1980;
        let month = (self.date % 512) / 32;
        let day = self.date % 32;
        let hour = self.time / 2048;
        let minute = (self.time % 2048) / 32;
        let second = self.time % 32;
        write!(
            str,
            "{:02}/{:02}/{:04} {:02}:{:02}:{:02}",
            month, day, year, hour, minute, second
        )
        .unwrap();

        f.debug_tuple("FatTimeHighP").field(&str.as_str()).finish()
    }
}

/// Fat Time
/// Stores the time to the precision of a second
#[derive(Clone, Copy)]
pub struct FatTime {
    time: u16,
    date: u16,
}

impl core::fmt::Debug for FatTime {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        // So we display in this format:
        // MM/DD/YY HH:MM:SS, which is 17 characters, but we make a 20 byte string
        use core::fmt::Write;
        let mut str = FixedByteStr::<20>::new();
        let year = self.date / 512 + 1980;
        let month = (self.date % 512) / 32;
        let day = self.date % 32;
        let hour = self.time / 2048;
        let minute = (self.time % 2048) / 32;
        let second = self.time % 32;
        write!(
            str,
            "{:02}/{:02}/{:04} {:02}:{:02}:{:02}",
            month, day, year, hour, minute, second
        )
        .unwrap();

        f.debug_tuple("FatTime").field(&str.as_str()).finish()
    }
}
