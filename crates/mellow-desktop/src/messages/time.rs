//! A message's moments in the reader's time zone and words (`format.ts`: `messageTimestampLabel`,
//! `timeOfDayLabel`, `fullTimestampLabel`, `dayDividerLabel`); client-core shapes the dates.

use chrono::{Local, Offset as _, TimeZone as _};
use mellow_client_core::shared_logic::format::{
    CivilDate, LocalDateTime, Locale, MessageTimestamp, format_full_date_and_time,
    format_long_date, format_short_date_and_time, format_time_of_day, message_timestamp,
};

use crate::i18n::{Language, Strings};

pub(crate) fn locale(strings: Strings) -> Locale {
    match strings.language() {
        Language::English => Locale::English,
        Language::French => Locale::French,
    }
}

/// `unix_milliseconds` on the reader's clock, with the offset their time zone had then.
#[must_use]
pub fn local_date_time(unix_milliseconds: u64) -> LocalDateTime {
    let milliseconds = i64::try_from(unix_milliseconds).unwrap_or(i64::MAX);
    let offset_seconds = Local
        .timestamp_millis_opt(milliseconds)
        .earliest()
        .map_or(0, |moment| moment.offset().fix().local_minus_utc());
    LocalDateTime::from_unix_milliseconds(milliseconds, offset_seconds)
}

/// The reader's day of `unix_milliseconds`, for `build_message_rows`.
#[must_use]
pub fn local_day_of(unix_milliseconds: u64) -> CivilDate {
    local_date_time(unix_milliseconds).date
}

/// "Today at 14:05", "Yesterday at 09:12", or the short date and time.
#[must_use]
pub fn message_timestamp_label(
    sent: LocalDateTime,
    now: LocalDateTime,
    strings: Strings,
) -> String {
    let locale = locale(strings);
    match message_timestamp(sent, now) {
        MessageTimestamp::Today(moment) => {
            strings.time().today_at(&format_time_of_day(locale, moment))
        }
        MessageTimestamp::Yesterday(moment) => strings
            .time()
            .yesterday_at(&format_time_of_day(locale, moment)),
        MessageTimestamp::Earlier(moment) => format_short_date_and_time(locale, moment),
    }
}

/// "08:30 AM", « 08:30 »: the hover time of a continued message.
#[must_use]
pub fn time_of_day_label(moment: LocalDateTime, strings: Strings) -> String {
    format_time_of_day(locale(strings), moment)
}

/// A moment to come (`upcomingTimeLabel`): its time today, else its short date and time ("Until
/// 06:30 PM", "Until 11/9/2026 08:00 AM").
#[must_use]
pub fn upcoming_time_label(moment: LocalDateTime, now: LocalDateTime, strings: Strings) -> String {
    if moment.date == now.date {
        format_time_of_day(locale(strings), moment)
    } else {
        format_short_date_and_time(locale(strings), moment)
    }
}

/// "Monday, October 19, 2026 at 8:30 AM": tooltips and screen readers.
#[must_use]
pub fn full_timestamp_label(moment: LocalDateTime, strings: Strings) -> String {
    format_full_date_and_time(locale(strings), moment)
}

/// "November 5, 2026", « 5 novembre 2026 ».
#[must_use]
pub fn day_divider_label(day: CivilDate, strings: Strings) -> String {
    format_long_date(locale(strings), day)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(day: u8, hour: u8, minute: u8) -> LocalDateTime {
        LocalDateTime {
            date: CivilDate {
                year: 2026,
                month: 11,
                day,
            },
            hour,
            minute,
        }
    }

    #[test]
    fn todays_and_yesterdays_messages_are_worded_around_their_time() {
        let english = Strings::new(Language::English);
        let now = at(7, 18, 0);
        assert_eq!(
            message_timestamp_label(at(7, 14, 5), now, english),
            "Today at 02:05 PM"
        );
        assert_eq!(
            message_timestamp_label(at(6, 9, 12), now, english),
            "Yesterday at 09:12 AM"
        );
        let french = Strings::new(Language::French);
        assert_eq!(
            message_timestamp_label(at(6, 9, 12), now, french),
            "Hier à 09:12"
        );
    }

    #[test]
    fn an_upcoming_moment_shows_its_time_today_and_its_date_later() {
        let english = Strings::new(Language::English);
        let now = at(7, 18, 0);
        assert_eq!(upcoming_time_label(at(7, 18, 15), now, english), "06:15 PM");
        assert_ne!(upcoming_time_label(at(8, 2, 0), now, english), "02:00 AM");
    }

    #[test]
    fn a_day_divider_writes_the_month_in_words() {
        let day = at(5, 0, 0).date;
        assert_eq!(
            day_divider_label(day, Strings::new(Language::English)),
            "November 5, 2026"
        );
        assert_eq!(
            day_divider_label(day, Strings::new(Language::French)),
            "5 novembre 2026"
        );
    }
}
