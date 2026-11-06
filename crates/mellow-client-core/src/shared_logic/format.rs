//! Display names, dates, counts and typing lines (`web/src/components/format.ts`).
//!
//! Dates are read in the reader's time zone by the caller, which turns an instant into a
//! [`LocalDateTime`] (the offset differs per instant across a clock change); everything here is
//! then plain calendar arithmetic. Dates and numbers are written as `Intl` writes them for the
//! web's `en` and `fr` locales; sentences around them ("Today at …") belong to the client's catalogue.

use mellow_protocol::{MAXIMUM_COUNTED_UNREAD_MENTIONS, PublicUser};

use super::unicode_classes::{is_js_whitespace, js_trim};

const MILLISECONDS_PER_SECOND: i64 = 1_000;
const SECONDS_PER_MINUTE: i64 = 60;
const SECONDS_PER_DAY: i64 = 86_400;
const MAXIMUM_SERVER_INITIALS: usize = 3;
/// Beyond this many, DESIGN.md §5.5 stops counting: "Several people are typing…".
const MAXIMUM_TYPING_PEOPLE_COUNTED: usize = 5;
/// `color.avatar.tint-1` to `tint-8` in design/tokens.json.
const AVATAR_TINT_COUNT: u32 = 8;
const FNV_OFFSET_BASIS: u32 = 0x811c_9dc5;
const FNV_PRIME: u32 = 0x0100_0193;
/// Binary steps with Discord's labels: the instance's "25 MB" limit is 25 × 1024 × 1024 bytes.
const BYTES_PER_UNIT_STEP: u64 = 1_024;
const TRUNCATION_ELLIPSIS: char = '…';
/// Longer endings are not extensions people recognise; they are cut like the rest of the name.
const MAXIMUM_KEPT_EXTENSION_LENGTH: usize = 8;
/// Shown whole before the extension when a name is cut, so "…(2).jpg" stays distinguishable.
const KEPT_CHARACTERS_BEFORE_EXTENSION: usize = 4;

/// The languages dates and numbers are written in, as the web's `Intl` writes `en` and `fr`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Locale {
    English,
    French,
}

/// A day of the proleptic Gregorian calendar.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct CivilDate {
    pub year: i32,
    /// 1 to 12.
    pub month: u8,
    /// 1 to 31.
    pub day: u8,
}

/// A moment as the reader's clock shows it, to the minute.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct LocalDateTime {
    pub date: CivilDate,
    /// 0 to 23.
    pub hour: u8,
    /// 0 to 59.
    pub minute: u8,
}

impl CivilDate {
    /// The date `days` after 1970-01-01 (Howard Hinnant's `civil_from_days`).
    #[must_use]
    pub fn from_days_since_unix_epoch(days: i64) -> Self {
        let shifted = days + 719_468;
        let era = shifted.div_euclid(146_097);
        let day_of_era = shifted.rem_euclid(146_097);
        let year_of_era =
            (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
        let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
        let month_from_march = (5 * day_of_year + 2) / 153;
        let day = day_of_year - (153 * month_from_march + 2) / 5 + 1;
        let month = if month_from_march < 10 {
            month_from_march + 3
        } else {
            month_from_march - 9
        };
        let year = year_of_era + era * 400 + i64::from(month <= 2);
        // Bounded by construction: a day is 1 to 31, a month 1 to 12, and an i64 day count of
        // the instants a client meets keeps the year far inside i32.
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        Self {
            year: year as i32,
            month: month as u8,
            day: day as u8,
        }
    }

    /// Days since 1970-01-01 (Howard Hinnant's `days_from_civil`).
    #[must_use]
    pub fn days_since_unix_epoch(self) -> i64 {
        let year = i64::from(self.year) - i64::from(self.month <= 2);
        let era = year.div_euclid(400);
        let year_of_era = year.rem_euclid(400);
        let month = i64::from(self.month);
        let day_of_year = (153 * (if month > 2 { month - 3 } else { month + 9 }) + 2) / 5
            + i64::from(self.day)
            - 1;
        let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
        era * 146_097 + day_of_era - 719_468
    }

    /// The date `days` later (earlier when negative).
    #[must_use]
    pub fn plus_days(self, days: i64) -> Self {
        Self::from_days_since_unix_epoch(self.days_since_unix_epoch() + days)
    }

    /// 0 for Sunday to 6 for Saturday.
    #[must_use]
    pub fn weekday_from_sunday(self) -> usize {
        // 1970-01-01 was a Thursday; the remainder is 0 to 6.
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let weekday = (self.days_since_unix_epoch() + 4).rem_euclid(7) as usize;
        weekday
    }

    /// `YYYY-MM-DD`, as search filters write days.
    #[must_use]
    pub fn iso_day(self) -> String {
        format!("{}-{:02}-{:02}", self.year, self.month, self.day)
    }

    /// Reads `YYYY-MM-DD`; `None` for anything else or a day the month does not have.
    #[must_use]
    pub fn from_iso_day(text: &str) -> Option<Self> {
        let bytes = text.as_bytes();
        let has_iso_shape = bytes.len() == 10
            && bytes.iter().enumerate().all(|(index, byte)| {
                if index == 4 || index == 7 {
                    *byte == b'-'
                } else {
                    byte.is_ascii_digit()
                }
            });
        if !has_iso_shape {
            return None;
        }
        let year = text.get(0..4)?.parse().ok()?;
        let month = text.get(5..7)?.parse().ok()?;
        let day = text.get(8..10)?.parse().ok()?;
        let is_real_day =
            (1..=12).contains(&month) && day >= 1 && day <= days_in_month(year, month);
        is_real_day.then_some(Self { year, month, day })
    }
}

/// How many days `month` of `year` has.
#[must_use]
pub fn days_in_month(year: i32, month: u8) -> u8 {
    match month {
        2 if year % 4 == 0 && (year % 100 != 0 || year % 400 == 0) => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    }
}

impl LocalDateTime {
    /// The moment `unix_milliseconds` on a clock `utc_offset_seconds` ahead of UTC.
    #[must_use]
    pub fn from_unix_milliseconds(unix_milliseconds: i64, utc_offset_seconds: i32) -> Self {
        let local_seconds =
            unix_milliseconds.div_euclid(MILLISECONDS_PER_SECOND) + i64::from(utc_offset_seconds);
        let seconds_of_day = local_seconds.rem_euclid(SECONDS_PER_DAY);
        // Bounded by the remainder: an hour is 0 to 23, a minute 0 to 59.
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        Self {
            date: CivilDate::from_days_since_unix_epoch(local_seconds.div_euclid(SECONDS_PER_DAY)),
            hour: (seconds_of_day / 3_600) as u8,
            minute: ((seconds_of_day / SECONDS_PER_MINUTE) % SECONDS_PER_MINUTE) as u8,
        }
    }
}

/// A message's time as the shell shows it (DESIGN.md §5.4): "Today at 14:05", "Yesterday at
/// 09:12" (the client's catalogue words them around [`format_time_of_day`]), or the date and
/// time for older messages ([`format_short_date_and_time`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MessageTimestamp {
    Today(LocalDateTime),
    Yesterday(LocalDateTime),
    Earlier(LocalDateTime),
}

/// Today and yesterday by local calendar day.
#[must_use]
pub fn message_timestamp(sent: LocalDateTime, now: LocalDateTime) -> MessageTimestamp {
    if sent.date == now.date {
        MessageTimestamp::Today(sent)
    } else if sent.date == now.date.plus_days(-1) {
        MessageTimestamp::Yesterday(sent)
    } else {
        MessageTimestamp::Earlier(sent)
    }
}

const ENGLISH_MONTHS: [&str; 12] = [
    "January",
    "February",
    "March",
    "April",
    "May",
    "June",
    "July",
    "August",
    "September",
    "October",
    "November",
    "December",
];
const ENGLISH_SHORT_MONTHS: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];
const ENGLISH_WEEKDAYS: [&str; 7] = [
    "Sunday",
    "Monday",
    "Tuesday",
    "Wednesday",
    "Thursday",
    "Friday",
    "Saturday",
];
const FRENCH_MONTHS: [&str; 12] = [
    "janvier",
    "février",
    "mars",
    "avril",
    "mai",
    "juin",
    "juillet",
    "août",
    "septembre",
    "octobre",
    "novembre",
    "décembre",
];
const FRENCH_SHORT_MONTHS: [&str; 12] = [
    "janv.", "févr.", "mars", "avr.", "mai", "juin", "juil.", "août", "sept.", "oct.", "nov.",
    "déc.",
];
const FRENCH_WEEKDAYS: [&str; 7] = [
    "dimanche", "lundi", "mardi", "mercredi", "jeudi", "vendredi", "samedi",
];

fn month_name(names: &'static [&'static str; 12], date: CivilDate) -> &'static str {
    names
        .get(usize::from(date.month).saturating_sub(1))
        .copied()
        .unwrap_or_default()
}

/// The 12-hour clock's hour and its period, as English writes times.
fn twelve_hour_clock(hour: u8) -> (u8, &'static str) {
    let period = if hour < 12 { "AM" } else { "PM" };
    let twelve_hour = match hour % 12 {
        0 => 12,
        other => other,
    };
    (twelve_hour, period)
}

/// "08:30 AM", « 08:30 ».
#[must_use]
pub fn format_time_of_day(locale: Locale, moment: LocalDateTime) -> String {
    match locale {
        Locale::English => {
            let (hour, period) = twelve_hour_clock(moment.hour);
            format!("{hour:02}:{:02} {period}", moment.minute)
        }
        Locale::French => format!("{:02}:{:02}", moment.hour, moment.minute),
    }
}

/// The short time inside dates: "8:30 AM", « 08:30 ».
fn short_time(locale: Locale, moment: LocalDateTime) -> String {
    match locale {
        Locale::English => {
            let (hour, period) = twelve_hour_clock(moment.hour);
            format!("{hour}:{:02} {period}", moment.minute)
        }
        Locale::French => format!("{:02}:{:02}", moment.hour, moment.minute),
    }
}

/// "10/19/26, 8:30 AM", « 19/10/2026 08:30 ».
#[must_use]
pub fn format_short_date_and_time(locale: Locale, moment: LocalDateTime) -> String {
    let CivilDate { year, month, day } = moment.date;
    match locale {
        Locale::English => format!(
            "{month}/{day}/{:02}, {}",
            year.rem_euclid(100),
            short_time(locale, moment)
        ),
        Locale::French => format!("{day:02}/{month:02}/{year} {}", short_time(locale, moment)),
    }
}

/// "Monday, October 19, 2026 at 8:30 AM", « lundi 19 octobre 2026 à 08:30 »: tooltips and screen readers.
#[must_use]
pub fn format_full_date_and_time(locale: Locale, moment: LocalDateTime) -> String {
    let date = moment.date;
    let weekday = date.weekday_from_sunday();
    match locale {
        Locale::English => format!(
            "{}, {} {}, {} at {}",
            ENGLISH_WEEKDAYS.get(weekday).copied().unwrap_or_default(),
            month_name(&ENGLISH_MONTHS, date),
            date.day,
            date.year,
            short_time(locale, moment)
        ),
        Locale::French => format!(
            "{} {} {} {} à {}",
            FRENCH_WEEKDAYS.get(weekday).copied().unwrap_or_default(),
            date.day,
            month_name(&FRENCH_MONTHS, date),
            date.year,
            short_time(locale, moment)
        ),
    }
}

/// A day divider's date, the month in words: "November 5, 2026", « 5 novembre 2026 ».
#[must_use]
pub fn format_long_date(locale: Locale, date: CivilDate) -> String {
    match locale {
        Locale::English => format!(
            "{} {}, {}",
            month_name(&ENGLISH_MONTHS, date),
            date.day,
            date.year
        ),
        Locale::French => format!(
            "{} {} {}",
            date.day,
            month_name(&FRENCH_MONTHS, date),
            date.year
        ),
    }
}

/// "Nov 3, 2026", « 3 nov. 2026 ».
#[must_use]
pub fn format_medium_date(locale: Locale, date: CivilDate) -> String {
    match locale {
        Locale::English => format!(
            "{} {}, {}",
            month_name(&ENGLISH_SHORT_MONTHS, date),
            date.day,
            date.year
        ),
        Locale::French => format!(
            "{} {} {}",
            date.day,
            month_name(&FRENCH_SHORT_MONTHS, date),
            date.year
        ),
    }
}

/// "Nov 5", « 5 nov. ».
#[must_use]
pub fn format_day_and_month(locale: Locale, date: CivilDate) -> String {
    match locale {
        Locale::English => format!("{} {}", month_name(&ENGLISH_SHORT_MONTHS, date), date.day),
        Locale::French => format!("{} {}", date.day, month_name(&FRENCH_SHORT_MONTHS, date)),
    }
}

/// A time still to come: "18:40" when it is today, else the short date and time (a mute's end).
#[must_use]
pub fn format_upcoming_time(locale: Locale, moment: LocalDateTime, now: LocalDateTime) -> String {
    if moment.date == now.date {
        format_time_of_day(locale, moment)
    } else {
        format_short_date_and_time(locale, moment)
    }
}

/// A day beside an earlier one: "Nov 5" in the earlier one's year, else "Nov 5, 2026" (also without one).
#[must_use]
pub fn format_medium_date_without_repeated_year(
    locale: Locale,
    date: CivilDate,
    earlier: Option<CivilDate>,
) -> String {
    if earlier.is_some_and(|earlier| earlier.year == date.year) {
        format_day_and_month(locale, date)
    } else {
        format_medium_date(locale, date)
    }
}

/// A `YYYY-MM-DD` day as the reader writes dates: "Oct 30, 2026", « 30 oct. 2026 »; `None` when it is no day.
#[must_use]
pub fn format_calendar_day(locale: Locale, iso_day: &str) -> Option<String> {
    CivilDate::from_iso_day(iso_day).map(|date| format_medium_date(locale, date))
}

/// First letter of up to three words, as Discord labels servers without an icon.
#[must_use]
pub fn server_initials(name: &str) -> String {
    name.split(is_js_whitespace)
        .filter(|word| !word.is_empty())
        .take(MAXIMUM_SERVER_INITIALS)
        .filter_map(|word| word.chars().next())
        .collect()
}

/// The name shown for a person: their display name, else their username.
#[must_use]
pub fn display_name(user: &PublicUser) -> &str {
    let shown = js_trim(&user.display_name);
    if shown.is_empty() {
        &user.username
    } else {
        shown
    }
}

/// First character of the display name, uppercase, shown on default avatars (DESIGN.md §2.3).
#[must_use]
pub fn avatar_initial(user: &PublicUser) -> String {
    display_name(user)
        .chars()
        .next()
        .map(char::to_uppercase)
        .map(Iterator::collect)
        .unwrap_or_default()
}

/// The default avatar's tint, 1 to 8 (`color.avatar.tint-N`): the same person keeps the same
/// colour everywhere. FNV-1a over the id's digits, so neighbouring Snowflakes still spread.
#[must_use]
pub fn avatar_tint_number(user_id: &str) -> u32 {
    let hash = user_id.encode_utf16().fold(FNV_OFFSET_BASIS, |hash, unit| {
        (hash ^ u32::from(unit)).wrapping_mul(FNV_PRIME)
    });
    hash % AVATAR_TINT_COUNT + 1
}

/// Who is typing, for the line under the composer (DESIGN.md §5.5); names are set in bold.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TypingLine<'a> {
    Nobody,
    /// "Ana is typing…"
    One(&'a str),
    /// "Ana and Léo are typing…"
    Two(&'a str, &'a str),
    /// "Ana, Léo and 3 others are typing…"
    TwoAndOthers {
        first: &'a str,
        second: &'a str,
        others: usize,
    },
    /// "Several people are typing…"
    Several,
}

#[must_use]
pub fn typing_line(names: &[String]) -> TypingLine<'_> {
    match names {
        [] => TypingLine::Nobody,
        _ if names.len() > MAXIMUM_TYPING_PEOPLE_COUNTED => TypingLine::Several,
        [first] => TypingLine::One(first),
        [first, second] => TypingLine::Two(first, second),
        [first, second, rest @ ..] => TypingLine::TwoAndOthers {
            first,
            second,
            others: rest.len(),
        },
    }
}

/// A file's size as the shell shows it (the catalogue words "1 byte", « 3 octets »).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FileSize {
    Bytes(u64),
    /// `value` already written in the reader's locale, at most one decimal: "2.4", « 2,4 ».
    InUnit {
        value: String,
        unit: FileSizeUnit,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileSizeUnit {
    Kilobytes,
    Megabytes,
    Gigabytes,
}

/// "512 bytes", "1 KB", "2.4 MB": below 1024 bytes in bytes, else in the largest binary unit
/// whose rounded value stays under 1024 (1023.96 KB reads "1 MB").
#[must_use]
pub fn human_file_size(locale: Locale, bytes: u64) -> FileSize {
    if bytes < BYTES_PER_UNIT_STEP {
        return FileSize::Bytes(bytes);
    }
    // Precision loss only past 2^53 bytes, far beyond any file.
    #[allow(clippy::cast_precision_loss)]
    let mut value = bytes as f64;
    let step = 1_024.0;
    let mut unit = FileSizeUnit::Kilobytes;
    for candidate in [
        FileSizeUnit::Kilobytes,
        FileSizeUnit::Megabytes,
        FileSizeUnit::Gigabytes,
    ] {
        value /= step;
        unit = candidate;
        if (value * 10.0).round() / 10.0 < step {
            break;
        }
    }
    FileSize::InUnit {
        value: format_number_with_one_decimal(locale, value),
        unit,
    }
}

/// `value.toLocaleString(locale, { maximumFractionDigits: 1 })` for a non-negative value.
fn format_number_with_one_decimal(locale: Locale, value: f64) -> String {
    // A file size in units is below 2^53 tenths, so the cast keeps every digit.
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let tenths = (value * 10.0).round().max(0.0) as u64;
    let (group_separator, decimal_separator) = match locale {
        Locale::English => (",", "."),
        Locale::French => ("\u{202f}", ","),
    };
    let integer_digits = (tenths / 10).to_string();
    let mut grouped = String::with_capacity(integer_digits.len() + 4);
    for (index, digit) in integer_digits.chars().enumerate() {
        if index > 0 && (integer_digits.len() - index).is_multiple_of(3) {
            grouped.push_str(group_separator);
        }
        grouped.push(digit);
    }
    match tenths % 10 {
        0 => grouped,
        fraction => format!("{grouped}{decimal_separator}{fraction}"),
    }
}

fn extension_character_count(file_name: &str) -> usize {
    match file_name.rfind('.') {
        Some(start) if start > 0 => file_name
            .get(start..)
            .map_or(0, |extension| extension.chars().count()),
        _ => 0,
    }
}

/// Shortens a file name in the middle, keeping its end and extension readable
/// ("holiday-photo…er.jpg"); names that fit are returned unchanged.
#[must_use]
pub fn middle_truncate(file_name: &str, maximum_characters: usize) -> String {
    let characters: Vec<char> = file_name.chars().collect();
    if characters.len() <= maximum_characters {
        return file_name.to_owned();
    }
    let extension_length = extension_character_count(file_name);
    let kept_extension_length = if extension_length <= MAXIMUM_KEPT_EXTENSION_LENGTH {
        extension_length
    } else {
        0
    };
    let available = maximum_characters.saturating_sub(1).max(2);
    // The end keeps the extension plus a few characters before it, so "…(2).jpg" stays distinguishable.
    let end_length = (available / 3)
        .max(kept_extension_length + 2)
        .min(available - 1);
    let start_length = available - end_length;
    let start: String = characters.iter().take(start_length).collect();
    let end: String = characters
        .iter()
        .skip(characters.len().saturating_sub(end_length))
        .collect();
    format!("{start}{TRUNCATION_ELLIPSIS}{end}")
}

/// A file name split for truncation by the layout, which can only cut the end of a text: the
/// start may end with an ellipsis, the end (extension and a few characters) always shows.
#[must_use]
pub fn file_name_start_and_end(file_name: &str) -> (String, String) {
    let characters: Vec<char> = file_name.chars().collect();
    let extension_length = extension_character_count(file_name);
    let kept_extension_length = if extension_length <= MAXIMUM_KEPT_EXTENSION_LENGTH {
        extension_length
    } else {
        0
    };
    let end_length = (kept_extension_length + KEPT_CHARACTERS_BEFORE_EXTENSION)
        .min(characters.len().saturating_sub(1));
    let start_length = characters.len() - end_length;
    (
        characters.iter().take(start_length).collect(),
        characters.iter().skip(start_length).collect(),
    )
}

/// The red badge's text: the server stops counting at `MAXIMUM_COUNTED_UNREAD_MENTIONS`.
#[must_use]
pub fn mention_badge_text(mention_count: u32) -> String {
    if mention_count >= MAXIMUM_COUNTED_UNREAD_MENTIONS {
        format!("{}+", MAXIMUM_COUNTED_UNREAD_MENTIONS - 1)
    } else {
        mention_count.to_string()
    }
}

#[cfg(test)]
mod tests {
    use mellow_protocol::Snowflake;

    use super::*;

    fn at(year: i32, month: u8, day: u8, hour: u8, minute: u8) -> LocalDateTime {
        LocalDateTime {
            date: CivilDate { year, month, day },
            hour,
            minute,
        }
    }

    #[test]
    fn converts_days_and_instants_both_ways() {
        let date = CivilDate {
            year: 2026,
            month: 10,
            day: 19,
        };
        assert_eq!(
            CivilDate::from_days_since_unix_epoch(date.days_since_unix_epoch()),
            date
        );
        assert_eq!(
            CivilDate {
                year: 2024,
                month: 3,
                day: 1
            }
            .plus_days(-1),
            CivilDate {
                year: 2024,
                month: 2,
                day: 29
            }
        );
        assert_eq!(
            CivilDate {
                year: 2026,
                month: 1,
                day: 1
            }
            .plus_days(-1),
            CivilDate {
                year: 2025,
                month: 12,
                day: 31
            }
        );
        // 2026-01-01T00:00:00Z, the Mellow epoch, is 01:00 in Paris in winter.
        assert_eq!(
            LocalDateTime::from_unix_milliseconds(1_767_225_600_000, 3_600),
            at(2026, 1, 1, 1, 0)
        );
        assert_eq!(
            LocalDateTime::from_unix_milliseconds(1_767_225_600_000, -60),
            at(2025, 12, 31, 23, 59)
        );
        assert_eq!(
            CivilDate {
                year: 2026,
                month: 10,
                day: 19
            }
            .weekday_from_sunday(),
            1
        );
    }

    #[test]
    fn reads_iso_days() {
        assert_eq!(
            CivilDate::from_iso_day("2024-02-29"),
            Some(CivilDate {
                year: 2024,
                month: 2,
                day: 29
            })
        );
        assert_eq!(CivilDate::from_iso_day("2023-02-29"), None);
        assert_eq!(CivilDate::from_iso_day("2026-13-01"), None);
        assert_eq!(CivilDate::from_iso_day("2026-1-01"), None);
        assert_eq!(
            CivilDate {
                year: 2026,
                month: 3,
                day: 5
            }
            .iso_day(),
            "2026-03-05"
        );
    }

    #[test]
    fn reads_the_time_of_a_snowflake_from_the_mellow_epoch() {
        let sent_at = 1_767_225_600_000 + 123_456_789;
        let id = Snowflake::from_raw(((sent_at - 1_767_225_600_000) << 22) | (1_023 << 12));
        assert_eq!(id.unix_milliseconds(), sent_at);
    }

    #[test]
    fn says_today_and_yesterday_by_local_calendar_day() {
        let now = at(2026, 10, 21, 15, 0);
        assert_eq!(
            message_timestamp(at(2026, 10, 21, 0, 5), now),
            MessageTimestamp::Today(at(2026, 10, 21, 0, 5))
        );
        assert_eq!(
            message_timestamp(at(2026, 10, 20, 23, 59), now),
            MessageTimestamp::Yesterday(at(2026, 10, 20, 23, 59))
        );
        assert_eq!(
            message_timestamp(at(2026, 10, 19, 8, 30), now),
            MessageTimestamp::Earlier(at(2026, 10, 19, 8, 30))
        );
        assert_eq!(
            message_timestamp(at(2025, 12, 31, 23, 0), at(2026, 1, 1, 0, 1)),
            MessageTimestamp::Yesterday(at(2025, 12, 31, 23, 0))
        );
    }

    /// Expected strings are `Intl.DateTimeFormat` output for "en" and "fr" (Node 22, ICU 76).
    #[test]
    fn writes_dates_as_intl_does() {
        let moment = at(2026, 10, 19, 8, 30);
        let midnight = at(2026, 11, 5, 0, 5);
        let afternoon = at(2027, 1, 4, 13, 7);
        assert_eq!(format_time_of_day(Locale::English, moment), "08:30 AM");
        assert_eq!(format_time_of_day(Locale::English, midnight), "12:05 AM");
        assert_eq!(format_time_of_day(Locale::English, afternoon), "01:07 PM");
        assert_eq!(format_time_of_day(Locale::French, midnight), "00:05");
        assert_eq!(
            format_short_date_and_time(Locale::English, moment),
            "10/19/26, 8:30 AM"
        );
        assert_eq!(
            format_short_date_and_time(Locale::English, afternoon),
            "1/4/27, 1:07 PM"
        );
        assert_eq!(
            format_short_date_and_time(Locale::French, midnight),
            "05/11/2026 00:05"
        );
        assert_eq!(
            format_full_date_and_time(Locale::English, moment),
            "Monday, October 19, 2026 at 8:30 AM"
        );
        assert_eq!(
            format_full_date_and_time(Locale::French, at(2026, 5, 1, 23, 59)),
            "vendredi 1 mai 2026 à 23:59"
        );
        assert_eq!(
            format_medium_date(Locale::English, afternoon.date),
            "Jan 4, 2027"
        );
        assert_eq!(
            format_medium_date(Locale::French, afternoon.date),
            "4 janv. 2027"
        );
        assert_eq!(
            format_long_date(Locale::English, midnight.date),
            "November 5, 2026"
        );
        assert_eq!(
            format_long_date(Locale::French, midnight.date),
            "5 novembre 2026"
        );
        assert_eq!(
            format_day_and_month(Locale::French, at(2026, 6, 7, 12, 0).date),
            "7 juin"
        );
        assert_eq!(
            format_calendar_day(Locale::French, "2026-10-30").as_deref(),
            Some("30 oct. 2026")
        );
    }

    #[test]
    fn writes_upcoming_times_as_a_time_today_and_a_date_later() {
        let now = at(2026, 10, 21, 15, 0);
        assert_eq!(
            format_upcoming_time(Locale::French, at(2026, 10, 21, 18, 40), now),
            "18:40"
        );
        assert_eq!(
            format_upcoming_time(Locale::French, at(2026, 10, 22, 18, 40), now),
            "22/10/2026 18:40"
        );
    }

    #[test]
    fn leaves_out_a_year_the_earlier_date_already_shows() {
        let opened = CivilDate {
            year: 2026,
            month: 11,
            day: 3,
        };
        let same_year = CivilDate {
            year: 2026,
            month: 11,
            day: 5,
        };
        let next_year = CivilDate {
            year: 2027,
            month: 1,
            day: 4,
        };
        assert_eq!(
            format_medium_date_without_repeated_year(Locale::English, same_year, Some(opened)),
            "Nov 5"
        );
        assert_eq!(
            format_medium_date_without_repeated_year(Locale::French, same_year, Some(opened)),
            "5 nov."
        );
        assert_eq!(
            format_medium_date_without_repeated_year(Locale::English, next_year, Some(opened)),
            "Jan 4, 2027"
        );
        assert_eq!(
            format_medium_date_without_repeated_year(Locale::English, same_year, None),
            "Nov 5, 2026"
        );
    }

    #[test]
    fn takes_the_first_letter_of_up_to_three_words() {
        assert_eq!(server_initials("  Rust   Lyon "), "RL");
        assert_eq!(server_initials("a b c d"), "abc");
        assert_eq!(server_initials("Élan"), "É");
    }

    #[test]
    fn falls_back_to_the_username_when_the_display_name_is_blank() {
        let zoe = PublicUser {
            id: Snowflake::from_raw(1),
            username: "zoé".into(),
            display_name: " ".into(),
            avatar: None,
            banner: None,
        };
        assert_eq!(avatar_initial(&zoe), "Z");
        assert_eq!(display_name(&zoe), "zoé");
    }

    #[test]
    fn keeps_one_tint_per_person_and_spreads_neighbouring_ids_over_the_palette() {
        assert_eq!(
            avatar_tint_number("106689215642730496"),
            avatar_tint_number("106689215642730496")
        );
        let tints: std::collections::BTreeSet<u32> = (0..64_u64)
            .map(|index| avatar_tint_number(&(106_689_215_642_730_496 + index * 4_096).to_string()))
            .collect();
        assert_eq!(tints, (1..=8).collect());
    }

    #[test]
    fn follows_the_design_wording_from_one_person_to_several() {
        let names = |list: &[&str]| {
            list.iter()
                .map(|name| (*name).to_owned())
                .collect::<Vec<_>>()
        };
        assert_eq!(typing_line(&[]), TypingLine::Nobody);
        assert_eq!(typing_line(&names(&["Ana"])), TypingLine::One("Ana"));
        assert_eq!(
            typing_line(&names(&["Ana", "Léo"])),
            TypingLine::Two("Ana", "Léo")
        );
        assert_eq!(
            typing_line(&names(&["Ana", "Léo", "Zoé"])),
            TypingLine::TwoAndOthers {
                first: "Ana",
                second: "Léo",
                others: 1
            }
        );
        assert_eq!(
            typing_line(&names(&["A", "B", "C", "D", "E"])),
            TypingLine::TwoAndOthers {
                first: "A",
                second: "B",
                others: 3
            }
        );
        assert_eq!(
            typing_line(&names(&["A", "B", "C", "D", "E", "F"])),
            TypingLine::Several
        );
    }

    #[test]
    fn counts_bytes_then_binary_units_with_one_decimal_at_most() {
        let in_unit = |value: &str, unit| FileSize::InUnit {
            value: value.to_owned(),
            unit,
        };
        assert_eq!(human_file_size(Locale::English, 0), FileSize::Bytes(0));
        assert_eq!(
            human_file_size(Locale::English, 1_023),
            FileSize::Bytes(1_023)
        );
        assert_eq!(
            human_file_size(Locale::English, 1_024),
            in_unit("1", FileSizeUnit::Kilobytes)
        );
        assert_eq!(
            human_file_size(Locale::English, 2_516_582),
            in_unit("2.4", FileSizeUnit::Megabytes)
        );
        assert_eq!(
            human_file_size(Locale::English, 25 * 1_024 * 1_024),
            in_unit("25", FileSizeUnit::Megabytes)
        );
        assert_eq!(
            human_file_size(Locale::English, 3 * 1_024 * 1_024 * 1_024),
            in_unit("3", FileSizeUnit::Gigabytes)
        );
        // Moves to the next unit instead of showing 1024 of the previous one.
        assert_eq!(
            human_file_size(Locale::English, 1_024 * 1_024 - 1),
            in_unit("1", FileSizeUnit::Megabytes)
        );
        assert_eq!(
            human_file_size(Locale::French, 2_516_582),
            in_unit("2,4", FileSizeUnit::Megabytes)
        );
        assert_eq!(
            human_file_size(Locale::English, 1_000 * 1_024),
            in_unit("1,000", FileSizeUnit::Kilobytes)
        );
        assert_eq!(
            human_file_size(Locale::French, 1_000 * 1_024),
            in_unit("1\u{202f}000", FileSizeUnit::Kilobytes)
        );
    }

    #[test]
    fn splits_file_names_keeping_the_extension_and_a_few_characters_as_the_end() {
        let split = |name: &str, start: &str, end: &str| {
            assert_eq!(
                file_name_start_and_end(name),
                (start.to_owned(), end.to_owned())
            );
        };
        split(
            "compte-rendu-version-finale.txt",
            "compte-rendu-version-fi",
            "nale.txt",
        );
        split("archive.backup-2026", "archive.backup-", "2026");
        split("README", "RE", "ADME");
        split("a.txt", "a", ".txt");
        split("😀😀😀😀😀😀.png", "😀😀", "😀😀😀😀.png");
    }

    #[test]
    fn cuts_the_middle_of_long_names() {
        assert_eq!(middle_truncate("report.pdf", 20), "report.pdf");
        assert_eq!(
            middle_truncate("exactly-twenty-chars", 20),
            "exactly-twenty-chars"
        );
        assert_eq!(
            middle_truncate("holiday-photos-from-summer.jpg", 20),
            "holiday-photo…er.jpg"
        );
        assert_eq!(
            middle_truncate("😀😀😀😀😀😀😀😀😀😀😀😀.png", 10),
            "😀😀😀…😀😀.png"
        );
        assert_eq!(
            middle_truncate("archive.backup-of-everything", 12),
            "archive.…ing"
        );
    }

    #[test]
    fn counts_mention_badges_up_to_99_then_says_99_plus() {
        assert_eq!(mention_badge_text(1), "1");
        assert_eq!(mention_badge_text(99), "99");
        assert_eq!(mention_badge_text(100), "99+");
    }
}
