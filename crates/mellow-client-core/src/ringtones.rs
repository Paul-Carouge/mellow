//! The ringtone of an incoming call (ADR-0062): the owner's "Friendly Chime", and "Jingle of the
//! Handbells" from 1 to 31 December, by the device's local date, without a setting. Both are Ogg
//! Opus files of `design/sounds/` (about 10 s, looped while the call rings), embedded here for
//! the native clients; the web client imports the same files.

/// Which ringtone plays.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ringtone {
    /// "Friendly Chime", the year round.
    Standard,
    /// "Jingle of the Handbells", in December.
    Christmas,
}

/// The month the Christmas ringtone plays in, 1-based.
pub const CHRISTMAS_RINGTONE_MONTH: u8 = 12;

/// The ringtone for a local date's month (1 to 12): the Christmas one all December.
#[must_use]
pub const fn ringtone_for_month(month: u8) -> Ringtone {
    if month == CHRISTMAS_RINGTONE_MONTH {
        Ringtone::Christmas
    } else {
        Ringtone::Standard
    }
}

impl Ringtone {
    /// The file's name in `design/sounds/`.
    #[must_use]
    pub const fn file_name(self) -> &'static str {
        match self {
            Self::Standard => "ringtone.opus",
            Self::Christmas => "ringtone-christmas.opus",
        }
    }

    /// The file itself: Ogg Opus, 48 kHz, stereo.
    #[must_use]
    pub const fn ogg_opus_bytes(self) -> &'static [u8] {
        match self {
            Self::Standard => include_bytes!("../../../design/sounds/ringtone.opus"),
            Self::Christmas => include_bytes!("../../../design/sounds/ringtone-christmas.opus"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_christmas_ringtone_plays_all_december_and_only_then() {
        assert_eq!(ringtone_for_month(12), Ringtone::Christmas);
        for month in 1..=11 {
            assert_eq!(ringtone_for_month(month), Ringtone::Standard, "{month}");
        }
    }

    #[test]
    fn each_ringtone_is_an_ogg_file() {
        for ringtone in [Ringtone::Standard, Ringtone::Christmas] {
            assert!(
                ringtone.ogg_opus_bytes().starts_with(b"OggS"),
                "{}",
                ringtone.file_name()
            );
        }
    }
}
