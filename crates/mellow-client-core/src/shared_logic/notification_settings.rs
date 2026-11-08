//! The reader's notification choices (`web/src/lib/notification-settings.ts`,
//! docs/design/web-notifications.md): what this device remembers, checked on the way in, and
//! when mutes end. Pure: storage belongs to the caller, which keeps the web's JSON shape.

use std::collections::BTreeMap;

use mellow_protocol::Snowflake;
use serde_json::{Map, Value, json};

/// What a place notifies about ("Notify me about").
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NotifyLevel {
    All,
    Mentions,
    Nothing,
}

impl NotifyLevel {
    pub const ALL: [Self; 3] = [Self::All, Self::Mentions, Self::Nothing];

    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::All => "all",
            Self::Mentions => "mentions",
            Self::Nothing => "nothing",
        }
    }

    fn from_stored(stored: &Value) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|level| stored.as_str() == Some(level.as_str()))
    }
}

/// The mute lengths offered in the dialog, as on Discord.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MuteDuration {
    FifteenMinutes,
    OneHour,
    EightHours,
    TwentyFourHours,
    UntilTurnedBackOn,
}

impl MuteDuration {
    pub const ALL: [Self; 5] = [
        Self::FifteenMinutes,
        Self::OneHour,
        Self::EightHours,
        Self::TwentyFourHours,
        Self::UntilTurnedBackOn,
    ];
}

const MINUTE_MILLISECONDS: u64 = 60_000;
const HOUR_MILLISECONDS: u64 = 60 * MINUTE_MILLISECONDS;

/// When a mute ends: a Unix time in milliseconds, or never (until the reader turns it back on).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MuteEnd {
    At(u64),
    Never,
}

/// A server's, a channel's or a conversation's own choices; absent fields follow the default.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PlaceNotificationSettings {
    pub level: Option<NotifyLevel>,
    pub muted_until: Option<MuteEnd>,
}

// The web's stored settings, one independent switch per field: no state machine hides here.
#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NotificationSettings {
    /// The reader turned desktop notifications on (the system's permission is checked apart).
    pub are_desktop_notifications_enabled: bool,
    pub should_show_previews: bool,
    pub is_sound_enabled: bool,
    /// The one-time permission bar was answered or dismissed.
    pub has_answered_permission_bar: bool,
    /// By server, channel or conversation id: Snowflakes never collide across kinds.
    pub places: BTreeMap<Snowflake, PlaceNotificationSettings>,
}

impl Default for NotificationSettings {
    fn default() -> Self {
        Self {
            are_desktop_notifications_enabled: false,
            should_show_previews: true,
            is_sound_enabled: true,
            has_answered_permission_bar: false,
            places: BTreeMap::new(),
        }
    }
}

/// At most this many places are kept, so a hostile or corrupted entry cannot grow without bound.
const MAXIMUM_STORED_PLACES: usize = 2_000;
const MAXIMUM_SNOWFLAKE_DIGITS: usize = 20;
/// The last millisecond of year 9999: a later mute end is corrupt, and would overflow date
/// formatting and platform timers.
const LATEST_STORED_MUTE_END: u64 = 253_402_300_799_999;

/// The web's storage key, kept so settings files read alike.
#[must_use]
pub fn notification_settings_storage_key(user_id: Snowflake) -> String {
    format!("mellow.notifications.{}", user_id.raw())
}

#[must_use]
pub fn mute_end_for(duration: MuteDuration, now: u64) -> MuteEnd {
    let length = match duration {
        MuteDuration::FifteenMinutes => 15 * MINUTE_MILLISECONDS,
        MuteDuration::OneHour => HOUR_MILLISECONDS,
        MuteDuration::EightHours => 8 * HOUR_MILLISECONDS,
        MuteDuration::TwentyFourHours => 24 * HOUR_MILLISECONDS,
        MuteDuration::UntilTurnedBackOn => return MuteEnd::Never,
    };
    MuteEnd::At(now.saturating_add(length))
}

/// Active until its end, not at it.
#[must_use]
pub fn is_mute_active(muted_until: Option<MuteEnd>, now: u64) -> bool {
    match muted_until {
        Some(MuteEnd::Never) => true,
        Some(MuteEnd::At(end)) => end > now,
        None => false,
    }
}

/// The earliest time a mute still running ends, to wake up then; `None` when none will.
#[must_use]
pub fn next_mute_end(settings: &NotificationSettings, now: u64) -> Option<u64> {
    settings
        .places
        .values()
        .filter_map(|place| match place.muted_until {
            Some(MuteEnd::At(end)) if end > now => Some(end),
            _ => None,
        })
        .min()
}

/// `settings` with `place_id`'s choices replaced; an entry left with nothing is dropped.
#[must_use]
pub fn with_place_settings(
    settings: &NotificationSettings,
    place_id: Snowflake,
    place: PlaceNotificationSettings,
) -> NotificationSettings {
    let mut updated = settings.clone();
    if place == PlaceNotificationSettings::default() {
        updated.places.remove(&place_id);
    } else {
        updated.places.insert(place_id, place);
    }
    updated
}

/// `settings` without mutes that ended, so storage does not keep them forever.
#[must_use]
pub fn without_ended_mutes(settings: &NotificationSettings, now: u64) -> NotificationSettings {
    let mut updated = settings.clone();
    updated.places.retain(|_, place| {
        if place.muted_until.is_some() && !is_mute_active(place.muted_until, now) {
            place.muted_until = None;
        }
        *place != PlaceNotificationSettings::default()
    });
    updated
}

fn is_snowflake_text(text: &str) -> bool {
    (1..=MAXIMUM_SNOWFLAKE_DIGITS).contains(&text.len())
        && text.bytes().all(|byte| byte.is_ascii_digit())
}

/// What storage held, kept only where it has the expected shape; anything else is the default.
#[must_use]
pub fn parse_notification_settings(stored: &Value) -> NotificationSettings {
    let defaults = NotificationSettings::default();
    let Some(record) = stored.as_object() else {
        return defaults;
    };
    let boolean_or =
        |key: &str, fallback: bool| record.get(key).and_then(Value::as_bool).unwrap_or(fallback);
    NotificationSettings {
        are_desktop_notifications_enabled: boolean_or(
            "areDesktopNotificationsEnabled",
            defaults.are_desktop_notifications_enabled,
        ),
        should_show_previews: boolean_or("shouldShowPreviews", defaults.should_show_previews),
        is_sound_enabled: boolean_or("isSoundEnabled", defaults.is_sound_enabled),
        has_answered_permission_bar: boolean_or(
            "hasAnsweredPermissionBar",
            defaults.has_answered_permission_bar,
        ),
        places: record
            .get("places")
            .and_then(Value::as_object)
            .map(parse_places)
            .unwrap_or_default(),
    }
}

fn parse_places(stored: &Map<String, Value>) -> BTreeMap<Snowflake, PlaceNotificationSettings> {
    stored
        .iter()
        .take(MAXIMUM_STORED_PLACES)
        .filter(|(place_id, _)| is_snowflake_text(place_id))
        .filter_map(|(place_id, value)| {
            let id = Snowflake::from_raw(place_id.parse().ok()?);
            let record = value.as_object()?;
            let muted_until = record.get("mutedUntil").and_then(|end| match end {
                Value::String(text) if text == "never" => Some(MuteEnd::Never),
                Value::Number(number) => number
                    .as_u64()
                    .filter(|end| (1..=LATEST_STORED_MUTE_END).contains(end))
                    .map(MuteEnd::At),
                _ => None,
            });
            let place = PlaceNotificationSettings {
                level: record.get("level").and_then(NotifyLevel::from_stored),
                muted_until,
            };
            (place != PlaceNotificationSettings::default()).then_some((id, place))
        })
        .collect()
}

/// `settings` in the shape [`parse_notification_settings`] reads, the web's own.
#[must_use]
pub fn notification_settings_to_json(settings: &NotificationSettings) -> Value {
    let places: Map<String, Value> = settings
        .places
        .iter()
        .map(|(id, place)| {
            let mut stored = Map::new();
            if let Some(level) = place.level {
                stored.insert("level".to_owned(), json!(level.as_str()));
            }
            match place.muted_until {
                Some(MuteEnd::At(end)) => {
                    stored.insert("mutedUntil".to_owned(), json!(end));
                }
                Some(MuteEnd::Never) => {
                    stored.insert("mutedUntil".to_owned(), json!("never"));
                }
                None => {}
            }
            (id.raw().to_string(), Value::Object(stored))
        })
        .collect();
    json!({
        "areDesktopNotificationsEnabled": settings.are_desktop_notifications_enabled,
        "shouldShowPreviews": settings.should_show_previews,
        "isSoundEnabled": settings.is_sound_enabled,
        "hasAnsweredPermissionBar": settings.has_answered_permission_bar,
        "places": places,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: u64 = 1_800_000_000_000;
    const HOUR: u64 = 3_600_000;

    fn id(raw: u64) -> Snowflake {
        Snowflake::from_raw(raw)
    }

    fn muted(end: MuteEnd) -> PlaceNotificationSettings {
        PlaceNotificationSettings {
            level: None,
            muted_until: Some(end),
        }
    }

    #[test]
    fn adds_each_offered_length_to_now_and_never_ends_the_last_one() {
        assert_eq!(
            mute_end_for(MuteDuration::FifteenMinutes, NOW),
            MuteEnd::At(NOW + HOUR / 4)
        );
        assert_eq!(
            mute_end_for(MuteDuration::OneHour, NOW),
            MuteEnd::At(NOW + HOUR)
        );
        assert_eq!(
            mute_end_for(MuteDuration::EightHours, NOW),
            MuteEnd::At(NOW + 8 * HOUR)
        );
        assert_eq!(
            mute_end_for(MuteDuration::TwentyFourHours, NOW),
            MuteEnd::At(NOW + 24 * HOUR)
        );
        assert_eq!(
            mute_end_for(MuteDuration::UntilTurnedBackOn, NOW),
            MuteEnd::Never
        );
    }

    #[test]
    fn is_active_until_its_end_not_at_it() {
        assert!(is_mute_active(Some(MuteEnd::At(NOW + 1)), NOW));
        assert!(!is_mute_active(Some(MuteEnd::At(NOW)), NOW));
        assert!(is_mute_active(Some(MuteEnd::Never), NOW));
        assert!(!is_mute_active(None, NOW));
    }

    #[test]
    fn wakes_up_at_the_earliest_end_still_to_come() {
        let settings = NotificationSettings {
            places: BTreeMap::from([
                (id(1), muted(MuteEnd::At(NOW + 2 * HOUR))),
                (id(2), muted(MuteEnd::At(NOW + HOUR))),
                (id(3), muted(MuteEnd::Never)),
                (id(4), muted(MuteEnd::At(NOW - 1))),
            ]),
            ..NotificationSettings::default()
        };
        assert_eq!(next_mute_end(&settings, NOW), Some(NOW + HOUR));
        assert_eq!(next_mute_end(&NotificationSettings::default(), NOW), None);
    }

    #[test]
    fn forgets_ended_mutes_but_keeps_the_level_beside_them() {
        let settings = NotificationSettings {
            places: BTreeMap::from([
                (
                    id(1),
                    PlaceNotificationSettings {
                        level: Some(NotifyLevel::All),
                        muted_until: Some(MuteEnd::At(NOW - 1)),
                    },
                ),
                (id(2), muted(MuteEnd::At(NOW - 1))),
                (id(3), muted(MuteEnd::At(NOW + 1))),
            ]),
            ..NotificationSettings::default()
        };
        assert_eq!(
            without_ended_mutes(&settings, NOW).places,
            BTreeMap::from([
                (
                    id(1),
                    PlaceNotificationSettings {
                        level: Some(NotifyLevel::All),
                        muted_until: None
                    }
                ),
                (id(3), muted(MuteEnd::At(NOW + 1))),
            ])
        );
        assert_eq!(
            without_ended_mutes(&NotificationSettings::default(), NOW),
            NotificationSettings::default()
        );
    }

    #[test]
    fn drops_an_entry_left_with_nothing() {
        let muted_settings = with_place_settings(
            &NotificationSettings::default(),
            id(7),
            muted(MuteEnd::Never),
        );
        assert_eq!(
            muted_settings.places,
            BTreeMap::from([(id(7), muted(MuteEnd::Never))])
        );
        assert_eq!(
            with_place_settings(&muted_settings, id(7), PlaceNotificationSettings::default())
                .places,
            BTreeMap::new()
        );
    }

    #[test]
    fn falls_back_to_the_defaults_for_anything_but_an_object() {
        for stored in [Value::Null, json!(3), json!("x"), json!([])] {
            assert_eq!(
                parse_notification_settings(&stored),
                NotificationSettings::default()
            );
        }
    }

    #[test]
    fn keeps_well_formed_fields_and_drops_the_rest() {
        let parsed = parse_notification_settings(&json!({
            "areDesktopNotificationsEnabled": true,
            "shouldShowPreviews": "yes",
            "isSoundEnabled": false,
            "places": {
                "10": { "level": "nothing", "mutedUntil": NOW },
                "11": { "level": "loud", "mutedUntil": "forever" },
                "12": { "mutedUntil": "never" },
                "not-an-id": { "level": "all" },
                "13": { "mutedUntil": 1.5e300 },
                "15": { "mutedUntil": LATEST_STORED_MUTE_END + 1 },
                "14": "all",
            },
        }));
        assert_eq!(
            parsed,
            NotificationSettings {
                are_desktop_notifications_enabled: true,
                should_show_previews: true,
                is_sound_enabled: false,
                has_answered_permission_bar: false,
                places: BTreeMap::from([
                    (
                        id(10),
                        PlaceNotificationSettings {
                            level: Some(NotifyLevel::Nothing),
                            muted_until: Some(MuteEnd::At(NOW))
                        }
                    ),
                    (id(12), muted(MuteEnd::Never)),
                ]),
            }
        );
    }

    #[test]
    fn round_trips_through_the_stored_shape() {
        let settings = with_place_settings(
            &NotificationSettings {
                is_sound_enabled: false,
                ..NotificationSettings::default()
            },
            id(9),
            PlaceNotificationSettings {
                level: Some(NotifyLevel::All),
                muted_until: Some(MuteEnd::At(NOW)),
            },
        );
        assert_eq!(
            parse_notification_settings(&notification_settings_to_json(&settings)),
            settings
        );
        assert_eq!(
            notification_settings_storage_key(id(1)),
            "mellow.notifications.1"
        );
    }
}
