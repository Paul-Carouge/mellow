//! Invite limits and lifetimes as people read them (`web/src/components/invites.ts`, DESIGN.md
//! §5.3 "Invite people", §5.8 "Invites"). The web words durations with `Intl`; here they take a
//! [`Locale`] like dates do.

use mellow_protocol::CreateInviteRequest;

use super::format::Locale;

const SECONDS_PER_MINUTE: u32 = 60;
const SECONDS_PER_HOUR: u32 = 60 * SECONDS_PER_MINUTE;
const SECONDS_PER_DAY: u32 = 24 * SECONDS_PER_HOUR;
const MILLISECONDS_PER_SECOND: f64 = 1_000.0;
const MINUTES_PER_HOUR: f64 = 60.0;
const HOURS_PER_DAY: f64 = 24.0;

/// Discord's lifetimes; `None` never expires. The instance accepts up to 30 days.
pub const INVITE_LIFETIME_PRESETS_IN_SECONDS: [Option<u32>; 8] = [
    Some(30 * SECONDS_PER_MINUTE),
    Some(SECONDS_PER_HOUR),
    Some(6 * SECONDS_PER_HOUR),
    Some(12 * SECONDS_PER_HOUR),
    Some(SECONDS_PER_DAY),
    Some(7 * SECONDS_PER_DAY),
    Some(30 * SECONDS_PER_DAY),
    None,
];

/// Discord's use limits; `None` is no limit. The instance accepts up to 10 000.
pub const INVITE_MAXIMUM_USES_PRESETS: [Option<u32>; 7] = [
    None,
    Some(1),
    Some(5),
    Some(10),
    Some(25),
    Some(50),
    Some(100),
];

/// Discord's default: a week, any number of people, so forgotten codes stop working on their own.
pub const DEFAULT_INVITE_LIMITS: CreateInviteRequest = CreateInviteRequest {
    max_uses: None,
    expires_in_seconds: Some(7 * SECONDS_PER_DAY),
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DurationUnit {
    Day,
    Hour,
    Minute,
    Second,
}

impl DurationUnit {
    const fn seconds(self) -> u32 {
        match self {
            Self::Day => SECONDS_PER_DAY,
            Self::Hour => SECONDS_PER_HOUR,
            Self::Minute => SECONDS_PER_MINUTE,
            Self::Second => 1,
        }
    }

    /// The unit's long name, singular or plural, as `Intl` writes it.
    fn word(self, locale: Locale, is_one: bool) -> &'static str {
        match (locale, self, is_one) {
            (Locale::English, Self::Day, true) => "day",
            (Locale::English, Self::Day, false) => "days",
            (Locale::English, Self::Hour, true) => "hour",
            (Locale::English, Self::Hour, false) => "hours",
            (_, Self::Minute, true) => "minute",
            (_, Self::Minute, false) => "minutes",
            (Locale::English, Self::Second, true) => "second",
            (Locale::English, Self::Second, false) => "seconds",
            (Locale::French, Self::Day, true) => "jour",
            (Locale::French, Self::Day, false) => "jours",
            (Locale::French, Self::Hour, true) => "heure",
            (Locale::French, Self::Hour, false) => "heures",
            (Locale::French, Self::Second, true) => "seconde",
            (Locale::French, Self::Second, false) => "secondes",
        }
    }
}

/// French counts 0 and 1 as singular, English only 1.
fn is_plural_one(locale: Locale, count: u64) -> bool {
    match locale {
        Locale::English => count == 1,
        Locale::French => count <= 1,
    }
}

/// "7 days", "1 hour", "30 minutes": the largest unit that counts the duration exactly, the
/// number kept with its unit by a no-break space.
#[must_use]
pub fn duration_label(locale: Locale, seconds: u32) -> String {
    let unit = [
        DurationUnit::Day,
        DurationUnit::Hour,
        DurationUnit::Minute,
        DurationUnit::Second,
    ]
    .into_iter()
    .find(|unit| seconds.is_multiple_of(unit.seconds()))
    .unwrap_or(DurationUnit::Second);
    let count = u64::from(seconds / unit.seconds());
    format!(
        "{count}\u{a0}{}",
        unit.word(locale, is_plural_one(locale, count))
    )
}

/// When an invite stops working, as its row says it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InviteExpiry {
    Never,
    Expired,
    /// Rounded to the nearest unit, so a week-long invite created a second ago says 7 days.
    In {
        count: u64,
        unit: DurationUnit,
    },
}

/// `expiryLabel`'s rounding: minutes under an hour (at least one), hours under a day, then days.
#[must_use]
#[allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss
)] // Milliseconds since 1970 fit an f64 exactly; the rounded counts are small and positive.
pub fn invite_expiry(
    expires_at_unix_milliseconds: Option<u64>,
    now_unix_milliseconds: u64,
) -> InviteExpiry {
    let Some(expires_at) = expires_at_unix_milliseconds else {
        return InviteExpiry::Never;
    };
    let remaining_seconds =
        (expires_at as f64 - now_unix_milliseconds as f64) / MILLISECONDS_PER_SECOND;
    if remaining_seconds <= 0.0 {
        return InviteExpiry::Expired;
    }
    let minutes = (remaining_seconds / f64::from(SECONDS_PER_MINUTE))
        .round()
        .max(1.0);
    if minutes < MINUTES_PER_HOUR {
        return InviteExpiry::In {
            count: minutes as u64,
            unit: DurationUnit::Minute,
        };
    }
    let hours = (remaining_seconds / f64::from(SECONDS_PER_HOUR)).round();
    if hours < HOURS_PER_DAY {
        return InviteExpiry::In {
            count: hours as u64,
            unit: DurationUnit::Hour,
        };
    }
    InviteExpiry::In {
        count: (remaining_seconds / f64::from(SECONDS_PER_DAY)).round() as u64,
        unit: DurationUnit::Day,
    }
}

/// "in 3 days", « dans 3 jours »: `Intl.RelativeTimeFormat` with `numeric: "always"`.
#[must_use]
pub fn relative_future_label(locale: Locale, count: u64, unit: DurationUnit) -> String {
    let word = unit.word(locale, is_plural_one(locale, count));
    match locale {
        Locale::English => format!("in {count} {word}"),
        Locale::French => format!("dans {count} {word}"),
    }
}

/// "AE8C-78A9-K2QF" as its groups, shown with the dashes stepping back (`inviteCodeGroups`).
/// Codes from before v0.18.0 have no dash: one group, case kept.
#[must_use]
pub fn invite_code_groups(code: &str) -> Vec<&str> {
    code.split('-').filter(|group| !group.is_empty()).collect()
}

/// "A E 8 C, 7 8 A 9, K 2 Q F": one symbol at a time, a pause between groups, for screen readers
/// (`spokenInviteCode`).
#[must_use]
pub fn spoken_invite_code(code: &str) -> String {
    invite_code_groups(code)
        .iter()
        .map(|group| {
            group
                .chars()
                .map(String::from)
                .collect::<Vec<_>>()
                .join(" ")
        })
        .collect::<Vec<_>>()
        .join(", ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_a_code_into_its_groups() {
        assert_eq!(
            invite_code_groups("AE8C-78A9-K2QF"),
            ["AE8C", "78A9", "K2QF"]
        );
        // Codes from before v0.18.0 have no dash: one group, case kept.
        assert_eq!(invite_code_groups("aB3dE5gH7k"), ["aB3dE5gH7k"]);
    }

    #[test]
    fn spells_a_code_out_for_screen_readers_pausing_between_groups() {
        assert_eq!(
            spoken_invite_code("AE8C-78A9-K2QF"),
            "A E 8 C, 7 8 A 9, K 2 Q F"
        );
        assert_eq!(spoken_invite_code("aB3"), "a B 3");
    }

    // Ported from `invites.test.ts`.

    /// `MAXIMUM_INVITE_LIFETIME` and `MAXIMUM_INVITE_USES` in `guild_rules.rs`.
    const MAXIMUM_LIFETIME_SECONDS: u32 = 30 * 24 * 60 * 60;
    const MAXIMUM_USES: u32 = 10_000;
    const NOW: u64 = 1_793_361_600_000;
    const HOUR: u64 = 60 * 60 * 1_000;
    const DAY: u64 = 24 * HOUR;

    fn expiry_label(expires_at: Option<u64>, locale: Locale) -> String {
        match invite_expiry(expires_at, NOW) {
            InviteExpiry::Never => "Never expires".to_owned(),
            InviteExpiry::Expired => "Expired".to_owned(),
            InviteExpiry::In { count, unit } => {
                format!("Expires {}", relative_future_label(locale, count, unit))
            }
        }
    }

    #[test]
    fn offers_discords_lifetimes_and_use_limits_all_within_the_instances_limits() {
        let labels: Vec<String> = INVITE_LIFETIME_PRESETS_IN_SECONDS
            .iter()
            .map(|seconds| {
                seconds.map_or("Never".to_owned(), |seconds| {
                    duration_label(Locale::English, seconds)
                })
            })
            .collect();
        assert_eq!(
            labels,
            [
                "30\u{a0}minutes",
                "1\u{a0}hour",
                "6\u{a0}hours",
                "12\u{a0}hours",
                "1\u{a0}day",
                "7\u{a0}days",
                "30\u{a0}days",
                "Never"
            ]
        );
        assert!(
            INVITE_LIFETIME_PRESETS_IN_SECONDS
                .iter()
                .flatten()
                .all(|seconds| (1..=MAXIMUM_LIFETIME_SECONDS).contains(seconds))
        );
        assert!(
            INVITE_MAXIMUM_USES_PRESETS
                .iter()
                .flatten()
                .all(|uses| (1..=MAXIMUM_USES).contains(uses))
        );
    }

    #[test]
    fn defaults_to_a_week_with_no_use_limit_one_of_the_presets() {
        assert_eq!(
            DEFAULT_INVITE_LIMITS.expires_in_seconds,
            Some(7 * 24 * 60 * 60)
        );
        assert_eq!(DEFAULT_INVITE_LIMITS.max_uses, None);
        assert!(
            INVITE_LIFETIME_PRESETS_IN_SECONDS.contains(&DEFAULT_INVITE_LIMITS.expires_in_seconds)
        );
        assert!(INVITE_MAXIMUM_USES_PRESETS.contains(&DEFAULT_INVITE_LIMITS.max_uses));
    }

    #[test]
    fn names_the_presets_in_french_and_counts_in_the_largest_exact_unit() {
        assert_eq!(duration_label(Locale::French, 30 * 60), "30\u{a0}minutes");
        assert_eq!(duration_label(Locale::French, 60 * 60), "1\u{a0}heure");
        assert_eq!(
            duration_label(Locale::French, 7 * 24 * 60 * 60),
            "7\u{a0}jours"
        );
        assert_eq!(duration_label(Locale::English, 90 * 60), "90\u{a0}minutes");
        assert_eq!(
            duration_label(Locale::English, 36 * 60 * 60),
            "36\u{a0}hours"
        );
        assert_eq!(duration_label(Locale::English, 45), "45\u{a0}seconds");
    }

    #[test]
    fn says_never_or_expired_and_rounds_to_the_nearest_unit() {
        assert_eq!(expiry_label(None, Locale::English), "Never expires");
        assert_eq!(expiry_label(Some(NOW), Locale::English), "Expired");
        assert_eq!(expiry_label(Some(NOW - HOUR), Locale::English), "Expired");
        let label = |offset: u64| expiry_label(Some(NOW + offset), Locale::English);
        assert_eq!(label(7 * DAY - 1_000), "Expires in 7 days");
        assert_eq!(label(3 * DAY + 2 * HOUR), "Expires in 3 days");
        assert_eq!(label(23 * HOUR + 50 * 60 * 1_000), "Expires in 1 day");
        assert_eq!(label(5 * HOUR), "Expires in 5 hours");
        assert_eq!(label(59 * 60 * 1_000 + 40_000), "Expires in 1 hour");
        assert_eq!(label(12 * 60 * 1_000), "Expires in 12 minutes");
        assert_eq!(label(10_000), "Expires in 1 minute");
        assert_eq!(
            expiry_label(Some(NOW + 3 * DAY), Locale::French),
            "Expires dans 3 jours"
        );
    }
}
