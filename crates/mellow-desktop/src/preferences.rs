//! Per-device settings (ADR-0051 "Session"): one small `key=value` file in the OS configuration
//! directory, the desktop's counterpart of the web client's `localStorage`. A missing or unreadable
//! file gives the defaults, unknown lines are ignored and every value is validated, since anything
//! may have written the file. Reads and writes block: they run off the UI thread, and the app
//! keeps the loaded preferences in [`PreferencesStore`], which every change goes through.

use std::io::{Read as _, Write as _};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, PoisonError};

use gpui::{App, AppContext as _, BorrowAppContext as _, Global};
use mellow_client_core::InstanceAddress;
use mellow_client_core::shared_logic::emoji_picker::{recent_emoji_from, remember_emoji};
use mellow_client_core::shared_logic::notification_settings::{
    NotificationSettings, notification_settings_to_json, parse_notification_settings,
};
use mellow_client_core::shared_logic::voice_settings::{
    PersonVoiceSetting, StoredVoiceToggles, VoiceSettings, parse_person_voice_settings,
    parse_voice_settings, parse_voice_toggles, person_voice_settings_to_json,
    voice_settings_to_json, voice_toggles_to_json,
};
use mellow_protocol::Snowflake;

const FILE_NAME: &str = "preferences";
/// Ours stays far below, even with the notification choices of a few accounts at their bound of
/// places; anything larger was not written by Mellow and is not read.
const MAXIMUM_FILE_LENGTH: u64 = 1024 * 1024;
const INSTANCE_ADDRESS_KEY: &str = "instance-address";
const PLACE_KEY: &str = "place";
const CHANNEL_KEY: &str = "channel";
const MEMBER_SHEET_KEY: &str = "member-sheet";
const CHANNELS_WIDTH_KEY: &str = "channels-width";
const MEMBERS_WIDTH_KEY: &str = "members-width";
const SEARCH_WIDTH_KEY: &str = "search-width";
const NOTIFICATIONS_KEY: &str = "notifications";
const MEDIA_VOLUME_KEY: &str = "media-volume";
const MEDIA_MUTED_KEY: &str = "media-muted";
const RECENT_EMOJI_KEY: &str = "recent-emoji";
const THEME_KEY: &str = "theme";
const LANGUAGE_KEY: &str = "language";
const VOICE_SETTINGS_KEY: &str = "voice-settings";
const VOICE_TOGGLES_KEY: &str = "voice-toggles";
const VOICE_PEOPLE_KEY: &str = "voice-people";
const HOME_PLACE: &str = "home";
const SERVER_PLACE_PREFIX: &str = "server:";
/// The latest servers and places only (the web's `MAXIMUM_REMEMBERED_MEMBER_SHEET_PLACES`), so a
/// hostile or corrupted file cannot grow the lists without bound.
pub const MAXIMUM_REMEMBERED_PLACES: usize = 200;
/// The people whose voice volume or local mute is kept, most recent last: bounded for the same
/// reason as the places.
pub const MAXIMUM_REMEMBERED_VOICE_PEOPLE: usize = 200;
/// The accounts whose notification choices are kept, most recent last: each may hold thousands of
/// places, so only a few accounts fit the file.
const MAXIMUM_REMEMBERED_NOTIFICATION_ACCOUNTS: usize = 4;
/// The accounts whose recently used emoji are kept, most recent last.
const MAXIMUM_REMEMBERED_RECENT_EMOJI_ACCOUNTS: usize = 4;
/// Any width a column may take is far below; larger numbers were not written by Mellow.
const MAXIMUM_STORED_COLUMN_WIDTH: f32 = 10_000.0;

/// The latest save asked for; an older save that starts after it skips its write.
static LATEST_SAVE_REQUEST: AtomicU64 = AtomicU64::new(0);
/// One write at a time, so the file is never written by two threads at once.
static WRITE_LOCK: Mutex<()> = Mutex::new(());

/// Where the reader was: opened again at the next launch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RememberedPlace {
    /// Conversations, with the one open if any.
    Home {
        conversation_id: Option<Snowflake>,
    },
    Server {
        guild_id: Snowflake,
    },
}

/// The theme the reader chose in User settings, Appearance.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ThemeChoice {
    /// The operating system's light or dark appearance, followed live.
    #[default]
    System,
    Light,
    Dark,
}

impl ThemeChoice {
    fn parse(text: &str) -> Self {
        match text {
            "light" => Self::Light,
            "dark" => Self::Dark,
            _ => Self::System,
        }
    }

    fn as_file_text(self) -> Option<&'static str> {
        match self {
            Self::System => None,
            Self::Light => Some("light"),
            Self::Dark => Some("dark"),
        }
    }
}

/// The interface language the reader chose in User settings, Appearance.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum LanguageChoice {
    /// The operating system's preferred languages, English when Mellow speaks none of them.
    #[default]
    System,
    English,
    French,
}

impl LanguageChoice {
    fn parse(text: &str) -> Self {
        match text {
            "en" => Self::English,
            "fr" => Self::French,
            _ => Self::System,
        }
    }

    fn as_file_text(self) -> Option<&'static str> {
        match self {
            Self::System => None,
            Self::English => Some("en"),
            Self::French => Some("fr"),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Preferences {
    /// The instance of the last sign-in, offered again on the instance address screen.
    pub last_instance_address: Option<InstanceAddress>,
    pub last_place: Option<RememberedPlace>,
    /// The channel last opened in each server, most recent last.
    pub last_channel_by_guild: Vec<(Snowflake, Snowflake)>,
    /// Servers and groups whose docked member sheet was left open, most recent last
    /// (`member-sheet-places.ts`).
    pub member_sheet_place_ids: Vec<Snowflake>,
    /// The widths the reader gave the channel list and the docked member sheet, in logical
    /// pixels; the shell keeps them within each column's bounds.
    pub channels_column_width: Option<f32>,
    pub members_column_width: Option<f32>,
    /// The docked search results' width, in logical pixels.
    pub search_column_width: Option<f32>,
    /// Each account's notification choices (the web keeps them per user in `localStorage`),
    /// most recent last.
    pub notification_settings_by_user: Vec<(Snowflake, NotificationSettings)>,
    /// The volume (0 to 1) every attachment starts at, as the web's `mellow.media-volume`; the
    /// file's own volume when unset.
    pub media_volume: Option<f32>,
    pub is_media_muted: bool,
    /// Each account's recently used emoji, latest first (the web keeps them in `localStorage`),
    /// accounts most recent last.
    pub recent_emoji_by_user: Vec<(Snowflake, Vec<String>)>,
    pub theme: ThemeChoice,
    pub language: LanguageChoice,
    /// Devices, input mode, processing and volumes (the web's `mellow.voice-settings`); change
    /// them through `voice::change_voice_settings`, which applies them to a call.
    pub voice_settings: VoiceSettings,
    /// The microphone and headset toggles, kept before joining and across launches.
    pub voice_toggles: StoredVoiceToggles,
    /// Each person's volume and local mute, only those changed from the default, most recent
    /// last.
    pub voice_people: Vec<(Snowflake, PersonVoiceSetting)>,
}

impl Preferences {
    /// The channel last opened in `guild_id`.
    #[must_use]
    pub fn last_channel_of(&self, guild_id: Snowflake) -> Option<Snowflake> {
        self.last_channel_by_guild
            .iter()
            .rev()
            .find(|(guild, _)| *guild == guild_id)
            .map(|(_, channel)| *channel)
    }

    /// Remembers `channel_id` as the one last opened in `guild_id`.
    pub fn remember_channel(&mut self, guild_id: Snowflake, channel_id: Snowflake) {
        self.last_channel_by_guild
            .retain(|(guild, _)| *guild != guild_id);
        self.last_channel_by_guild.push((guild_id, channel_id));
        keep_latest(&mut self.last_channel_by_guild, MAXIMUM_REMEMBERED_PLACES);
    }

    /// The notification choices of `user_id`; the defaults when it has none yet.
    #[must_use]
    pub fn notification_settings_of(&self, user_id: Snowflake) -> NotificationSettings {
        self.notification_settings_by_user
            .iter()
            .find(|(user, _)| *user == user_id)
            .map(|(_, settings)| settings.clone())
            .unwrap_or_default()
    }

    /// Keeps `settings` as the notification choices of `user_id`.
    pub fn set_notification_settings(
        &mut self,
        user_id: Snowflake,
        settings: NotificationSettings,
    ) {
        self.notification_settings_by_user
            .retain(|(user, _)| *user != user_id);
        self.notification_settings_by_user.push((user_id, settings));
        let excess = self
            .notification_settings_by_user
            .len()
            .saturating_sub(MAXIMUM_REMEMBERED_NOTIFICATION_ACCOUNTS);
        self.notification_settings_by_user.drain(..excess);
    }

    /// The emoji `user_id` picked lately, latest first.
    #[must_use]
    pub fn recent_emoji_of(&self, user_id: Snowflake) -> &[String] {
        self.recent_emoji_by_user
            .iter()
            .find(|(user, _)| *user == user_id)
            .map_or(&[], |(_, recent)| recent.as_slice())
    }

    /// `emoji` becomes the latest `user_id` picked.
    pub fn remember_emoji(&mut self, user_id: Snowflake, emoji: &str) {
        let recent = remember_emoji(self.recent_emoji_of(user_id), emoji);
        self.set_recent_emoji(user_id, recent);
    }

    fn set_recent_emoji(&mut self, user_id: Snowflake, recent: Vec<String>) {
        self.recent_emoji_by_user
            .retain(|(user, _)| *user != user_id);
        self.recent_emoji_by_user.push((user_id, recent));
        let excess = self
            .recent_emoji_by_user
            .len()
            .saturating_sub(MAXIMUM_REMEMBERED_RECENT_EMOJI_ACCOUNTS);
        self.recent_emoji_by_user.drain(..excess);
    }

    /// The volume and local mute the reader gave `user_id`.
    #[must_use]
    pub fn voice_person(&self, user_id: Snowflake) -> PersonVoiceSetting {
        self.voice_people
            .iter()
            .find(|(user, _)| *user == user_id)
            .map(|(_, setting)| *setting)
            .unwrap_or_default()
    }

    /// Keeps `setting` for `user_id`; a default one is forgotten.
    pub fn set_voice_person(&mut self, user_id: Snowflake, setting: PersonVoiceSetting) {
        self.voice_people.retain(|(user, _)| *user != user_id);
        if !setting.is_default() {
            self.voice_people.push((user_id, setting));
            keep_latest(&mut self.voice_people, MAXIMUM_REMEMBERED_VOICE_PEOPLE);
        }
    }

    /// Opens or closes the docked member sheet of a server or group, for next time.
    pub fn remember_member_sheet(&mut self, place_id: Snowflake, is_open: bool) {
        self.member_sheet_place_ids.retain(|id| *id != place_id);
        if is_open {
            self.member_sheet_place_ids.push(place_id);
            keep_latest(&mut self.member_sheet_place_ids, MAXIMUM_REMEMBERED_PLACES);
        }
    }
}

fn keep_latest<T>(list: &mut Vec<T>, maximum_length: usize) {
    let excess = list.len().saturating_sub(maximum_length);
    list.drain(..excess);
}

fn parse_place(value: &str) -> Option<RememberedPlace> {
    if value == HOME_PLACE {
        return Some(RememberedPlace::Home {
            conversation_id: None,
        });
    }
    if let Some(conversation) = value
        .strip_prefix(HOME_PLACE)
        .and_then(|rest| rest.strip_prefix(':'))
    {
        return parse_snowflake(conversation).map(|id| RememberedPlace::Home {
            conversation_id: Some(id),
        });
    }
    parse_snowflake(value.strip_prefix(SERVER_PLACE_PREFIX)?)
        .map(|guild_id| RememberedPlace::Server { guild_id })
}

fn place_text(place: RememberedPlace) -> String {
    match place {
        RememberedPlace::Home {
            conversation_id: None,
        } => HOME_PLACE.to_owned(),
        RememberedPlace::Home {
            conversation_id: Some(id),
        } => format!("{HOME_PLACE}:{id}"),
        RememberedPlace::Server { guild_id } => format!("{SERVER_PLACE_PREFIX}{guild_id}"),
    }
}

/// Only plain decimal ids: anything else in the file is ignored.
fn parse_snowflake(text: &str) -> Option<Snowflake> {
    let is_decimal =
        !text.is_empty() && text.len() <= 20 && text.bytes().all(|byte| byte.is_ascii_digit());
    is_decimal.then(|| text.parse().ok()).flatten()
}

fn parse_media_volume(text: &str) -> Option<f32> {
    text.parse::<f32>()
        .ok()
        .filter(|volume| (0.0..=1.0).contains(volume))
}

fn parse_column_width(text: &str) -> Option<f32> {
    text.parse::<f32>()
        .ok()
        .filter(|width| width.is_finite() && (0.0..=MAXIMUM_STORED_COLUMN_WIDTH).contains(width))
}

impl Preferences {
    /// The stored preferences; the defaults when there are none or they cannot be read. Blocks.
    #[must_use]
    pub fn load() -> Self {
        preferences_file_path().map_or_else(Self::default, |path| Self::load_from(&path))
    }

    /// Only a regular file of a size Mellow writes is read: a FIFO would block, and anything
    /// larger was not written by Mellow.
    fn load_from(path: &Path) -> Self {
        let read = std::fs::metadata(path).and_then(|metadata| {
            if !metadata.is_file() {
                return Err(std::io::Error::from(std::io::ErrorKind::InvalidInput));
            }
            let mut text = String::new();
            std::fs::File::open(path)?
                .take(MAXIMUM_FILE_LENGTH + 1)
                .read_to_string(&mut text)?;
            if u64::try_from(text.len()).unwrap_or(u64::MAX) > MAXIMUM_FILE_LENGTH {
                return Err(std::io::Error::from(std::io::ErrorKind::FileTooLarge));
            }
            Ok(text)
        });
        match read {
            Ok(text) => Self::from_file_text(&text),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Self::default(),
            Err(error) => {
                tracing::warn!(error = %error.kind(), "cannot read the preferences; using the defaults");
                Self::default()
            }
        }
    }

    fn from_file_text(text: &str) -> Self {
        let mut preferences = Self::default();
        for (key, value) in text.lines().filter_map(|line| line.split_once('=')) {
            let value = value.trim();
            match key.trim() {
                // The same https rule as a typed address: a tampered file cannot downgrade it.
                INSTANCE_ADDRESS_KEY => {
                    preferences.last_instance_address = InstanceAddress::parse(value).ok();
                }
                PLACE_KEY => preferences.last_place = parse_place(value),
                CHANNEL_KEY => {
                    if let Some((guild, channel)) = value.split_once(':')
                        && let (Some(guild_id), Some(channel_id)) =
                            (parse_snowflake(guild), parse_snowflake(channel))
                    {
                        preferences.remember_channel(guild_id, channel_id);
                    }
                }
                MEMBER_SHEET_KEY => {
                    if let Some(place_id) = parse_snowflake(value) {
                        preferences.remember_member_sheet(place_id, true);
                    }
                }
                CHANNELS_WIDTH_KEY => preferences.channels_column_width = parse_column_width(value),
                MEMBERS_WIDTH_KEY => preferences.members_column_width = parse_column_width(value),
                SEARCH_WIDTH_KEY => preferences.search_column_width = parse_column_width(value),
                MEDIA_VOLUME_KEY => preferences.media_volume = parse_media_volume(value),
                MEDIA_MUTED_KEY => preferences.is_media_muted = value == "true",
                THEME_KEY => preferences.theme = ThemeChoice::parse(value),
                LANGUAGE_KEY => preferences.language = LanguageChoice::parse(value),
                // The web's JSON, validated field by field as the web reads it.
                VOICE_SETTINGS_KEY | VOICE_TOGGLES_KEY | VOICE_PEOPLE_KEY => {
                    let stored = serde_json::from_str(value).unwrap_or(serde_json::Value::Null);
                    match key.trim() {
                        VOICE_SETTINGS_KEY => {
                            preferences.voice_settings = parse_voice_settings(&stored);
                        }
                        VOICE_TOGGLES_KEY => {
                            preferences.voice_toggles = parse_voice_toggles(&stored);
                        }
                        _ => {
                            for (user_id, setting) in parse_person_voice_settings(&stored) {
                                preferences.set_voice_person(user_id, setting);
                            }
                        }
                    }
                }
                // `<user id>:<the web's JSON>`, validated field by field as the web reads it.
                NOTIFICATIONS_KEY => {
                    if let Some((user, json)) = value.split_once(':')
                        && let Some(user_id) = parse_snowflake(user)
                        && let Ok(stored) = serde_json::from_str(json)
                    {
                        preferences.set_notification_settings(
                            user_id,
                            parse_notification_settings(&stored),
                        );
                    }
                }
                // `<user id>:<a JSON list>`, kept only as emoji the picker knows.
                RECENT_EMOJI_KEY => {
                    if let Some((user, json)) = value.split_once(':')
                        && let Some(user_id) = parse_snowflake(user)
                        && let Ok(serde_json::Value::Array(stored)) = serde_json::from_str(json)
                    {
                        let stored = stored
                            .into_iter()
                            .filter_map(|value| match value {
                                serde_json::Value::String(emoji) => Some(emoji),
                                _ => None,
                            })
                            .collect();
                        preferences.set_recent_emoji(user_id, recent_emoji_from(stored));
                    }
                }
                _ => {}
            }
        }
        preferences
    }

    fn to_file_text(&self) -> String {
        use std::fmt::Write as _;
        let instance_address = self
            .last_instance_address
            .as_ref()
            .map_or("", InstanceAddress::as_str);
        let mut text = format!("{INSTANCE_ADDRESS_KEY}={instance_address}\n");
        // Writing to a `String` cannot fail.
        if let Some(place) = self.last_place {
            let _ = writeln!(text, "{PLACE_KEY}={}", place_text(place));
        }
        for (guild_id, channel_id) in &self.last_channel_by_guild {
            let _ = writeln!(text, "{CHANNEL_KEY}={guild_id}:{channel_id}");
        }
        for place_id in &self.member_sheet_place_ids {
            let _ = writeln!(text, "{MEMBER_SHEET_KEY}={place_id}");
        }
        for (key, width) in [
            (CHANNELS_WIDTH_KEY, self.channels_column_width),
            (MEMBERS_WIDTH_KEY, self.members_column_width),
            (SEARCH_WIDTH_KEY, self.search_column_width),
        ] {
            if let Some(width) = width {
                let _ = writeln!(text, "{key}={width}");
            }
        }
        if let Some(volume) = self.media_volume {
            let _ = writeln!(text, "{MEDIA_VOLUME_KEY}={volume}");
        }
        if self.is_media_muted {
            let _ = writeln!(text, "{MEDIA_MUTED_KEY}=true");
        }
        if let Some(theme) = self.theme.as_file_text() {
            let _ = writeln!(text, "{THEME_KEY}={theme}");
        }
        if let Some(language) = self.language.as_file_text() {
            let _ = writeln!(text, "{LANGUAGE_KEY}={language}");
        }
        // Compact JSON escapes line breaks, so each account stays on one line.
        if self.voice_settings != VoiceSettings::DEFAULT {
            let json = voice_settings_to_json(&self.voice_settings);
            let _ = writeln!(text, "{VOICE_SETTINGS_KEY}={json}");
        }
        if self.voice_toggles != StoredVoiceToggles::default() {
            let json = voice_toggles_to_json(self.voice_toggles);
            let _ = writeln!(text, "{VOICE_TOGGLES_KEY}={json}");
        }
        if !self.voice_people.is_empty() {
            let json = person_voice_settings_to_json(&self.voice_people);
            let _ = writeln!(text, "{VOICE_PEOPLE_KEY}={json}");
        }
        for (user_id, settings) in &self.notification_settings_by_user {
            let json = notification_settings_to_json(settings);
            let _ = writeln!(text, "{NOTIFICATIONS_KEY}={user_id}:{json}");
        }
        for (user_id, recent) in &self.recent_emoji_by_user {
            let json = serde_json::Value::from(recent.clone());
            let _ = writeln!(text, "{RECENT_EMOJI_KEY}={user_id}:{json}");
        }
        text
    }

    /// Writes the preferences on GPUI's background executor; of several quick saves, the latest
    /// wins.
    fn save_in_background(&self, cx: &App) {
        let Some(path) = preferences_file_path() else {
            return;
        };
        let text = self.to_file_text();
        let request = LATEST_SAVE_REQUEST.fetch_add(1, Ordering::SeqCst) + 1;
        cx.background_executor()
            .spawn(async move {
                let _write = WRITE_LOCK.lock().unwrap_or_else(PoisonError::into_inner);
                if LATEST_SAVE_REQUEST.load(Ordering::SeqCst) != request {
                    return;
                }
                if let Err(error) = write_atomically(&path, &text) {
                    tracing::warn!(error = %error.kind(), "cannot save the preferences");
                }
            })
            .detach();
    }
}

/// A change asked for before the file was read; applied once it is.
type PendingChange = Box<dyn FnOnce(&mut Preferences)>;

/// The preferences the app runs with. Every change goes through [`change_preferences`], which
/// saves the whole set, so no part of the app overwrites another's settings. Without
/// [`load_preferences_in_background`] (tests), nothing is read or written on the machine.
pub struct PreferencesStore {
    /// `None` while the file is read.
    loaded: Option<Preferences>,
    pending_changes: Vec<PendingChange>,
    is_saved_to_disk: bool,
}

impl Default for PreferencesStore {
    fn default() -> Self {
        Self {
            loaded: Some(Preferences::default()),
            pending_changes: Vec::new(),
            is_saved_to_disk: false,
        }
    }
}

impl std::fmt::Debug for PreferencesStore {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PreferencesStore")
            .field("loaded", &self.loaded)
            .field("pending_change_count", &self.pending_changes.len())
            .finish_non_exhaustive()
    }
}

impl Global for PreferencesStore {}

/// Reads the preferences file off the main thread; until then [`preferences`] is `None` and
/// changes wait.
pub fn load_preferences_in_background(cx: &mut App) {
    cx.set_global(PreferencesStore {
        loaded: None,
        pending_changes: Vec::new(),
        is_saved_to_disk: true,
    });
    let load = cx.background_spawn(async { Preferences::load() });
    cx.spawn(async move |cx| {
        let preferences = load.await;
        cx.update(|cx| take_loaded_preferences(preferences, cx));
    })
    .detach();
}

/// The file is read: changes asked meanwhile go on top of it, and are saved.
fn take_loaded_preferences(mut preferences: Preferences, cx: &mut App) {
    cx.update_global::<PreferencesStore, _>(|store, cx| {
        let has_changes = !store.pending_changes.is_empty();
        for change in store.pending_changes.drain(..) {
            change(&mut preferences);
        }
        if has_changes && store.is_saved_to_disk {
            preferences.save_in_background(cx);
        }
        store.loaded = Some(preferences);
    });
}

/// The preferences, once read.
#[must_use]
pub fn preferences(cx: &App) -> Option<&Preferences> {
    cx.try_global::<PreferencesStore>()
        .map_or(Some(&DEFAULT_PREFERENCES), |store| store.loaded.as_ref())
}

static DEFAULT_PREFERENCES: Preferences = Preferences {
    last_instance_address: None,
    last_place: None,
    last_channel_by_guild: Vec::new(),
    member_sheet_place_ids: Vec::new(),
    channels_column_width: None,
    members_column_width: None,
    search_column_width: None,
    notification_settings_by_user: Vec::new(),
    media_volume: None,
    is_media_muted: false,
    recent_emoji_by_user: Vec::new(),
    theme: ThemeChoice::System,
    language: LanguageChoice::System,
    voice_settings: VoiceSettings::DEFAULT,
    voice_toggles: StoredVoiceToggles {
        is_microphone_muted: false,
        is_deafened: false,
        was_microphone_muted_before_deafening: false,
    },
    voice_people: Vec::new(),
};

/// Changes the preferences and saves them; a change asked for while the file is still read is
/// applied on top of it.
pub fn change_preferences(cx: &mut App, change: impl FnOnce(&mut Preferences) + 'static) {
    if !cx.has_global::<PreferencesStore>() {
        cx.set_global(PreferencesStore::default());
    }
    cx.update_global::<PreferencesStore, _>(|store, cx| match store.loaded.as_mut() {
        Some(preferences) => {
            change(preferences);
            if store.is_saved_to_disk {
                preferences.save_in_background(cx);
            }
        }
        None => store.pending_changes.push(Box::new(change)),
    });
}

/// A crash or a full disk mid-write leaves the previous file, never half of the new one.
fn write_atomically(path: &Path, text: &str) -> std::io::Result<()> {
    let directory = path
        .parent()
        .ok_or_else(|| std::io::Error::from(std::io::ErrorKind::InvalidInput))?;
    std::fs::create_dir_all(directory)?;
    let mut file = tempfile::NamedTempFile::new_in(directory)?;
    file.write_all(text.as_bytes())?;
    file.as_file().sync_all()?;
    file.persist(path).map_err(|error| error.error)?;
    Ok(())
}

/// `~/Library/Application Support/Mellow` on macOS, `%APPDATA%\Mellow` on Windows,
/// `$XDG_CONFIG_HOME/mellow` (or `~/.config/mellow`) elsewhere.
fn preferences_file_path() -> Option<PathBuf> {
    let directory = if cfg!(target_os = "macos") {
        PathBuf::from(std::env::var_os("HOME")?).join("Library/Application Support/Mellow")
    } else if cfg!(windows) {
        PathBuf::from(std::env::var_os("APPDATA")?).join("Mellow")
    } else {
        // The XDG base directory spec ignores relative paths.
        std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .filter(|path| path.is_absolute())
            .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))?
            .join("mellow")
    };
    Some(directory.join(FILE_NAME))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn with_instance(address: &str) -> Preferences {
        Preferences {
            last_instance_address: Some(InstanceAddress::parse(address).unwrap()),
            ..Preferences::default()
        }
    }

    fn id(raw: u64) -> Snowflake {
        Snowflake::from_raw(raw)
    }

    #[test]
    fn preferences_survive_a_round_trip_through_an_atomic_write() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("nested").join(FILE_NAME);
        let mut preferences = with_instance("chat.example.org");
        preferences.last_place = Some(RememberedPlace::Home {
            conversation_id: Some(id(7)),
        });
        preferences.remember_channel(id(1), id(11));
        preferences.remember_channel(id(2), id(21));
        preferences.remember_member_sheet(id(1), true);
        preferences.channels_column_width = Some(300.0);
        preferences.members_column_width = Some(264.0);
        preferences.search_column_width = Some(480.0);
        preferences.media_volume = Some(0.25);
        preferences.is_media_muted = true;
        preferences.theme = ThemeChoice::Light;
        preferences.language = LanguageChoice::French;

        write_atomically(&path, &preferences.to_file_text()).unwrap();
        assert_eq!(Preferences::load_from(&path), preferences);

        write_atomically(&path, &Preferences::default().to_file_text()).unwrap();
        assert_eq!(Preferences::load_from(&path), Preferences::default());
        // Only the file itself: no temporary file is left behind.
        assert_eq!(
            std::fs::read_dir(path.parent().unwrap()).unwrap().count(),
            1
        );
    }

    #[test]
    fn only_a_regular_file_of_a_sane_size_is_read() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join(FILE_NAME);
        let line = "instance-address=https://chat.example.org/\n";
        std::fs::write(&path, line).unwrap();
        assert_eq!(
            Preferences::load_from(&path),
            with_instance("chat.example.org")
        );

        // Truncated reading would keep the first 64 KiB of an oversized file.
        let oversized = format!(
            "{line}{}",
            "#".repeat(usize::try_from(MAXIMUM_FILE_LENGTH).unwrap())
        );
        std::fs::write(&path, oversized).unwrap();
        assert_eq!(Preferences::load_from(&path), Preferences::default());

        // A directory (or a FIFO) is never opened for reading.
        assert_eq!(
            Preferences::load_from(directory.path()),
            Preferences::default()
        );
        assert_eq!(
            Preferences::load_from(&directory.path().join("missing")),
            Preferences::default()
        );
    }

    #[test]
    fn unknown_or_invalid_lines_keep_the_defaults() {
        for text in [
            "instance-address=http://chat.example.org\n",
            "instance-address=ftp://chat.example.org\n",
            "instance-address=https://user:secret@chat.example.org\n",
            "noise\ntheme=sepia\n",
        ] {
            assert_eq!(
                Preferences::from_file_text(text),
                Preferences::default(),
                "{text}"
            );
        }
    }

    #[test]
    fn the_shell_remembers_the_place_channels_member_sheets_and_widths() {
        let text = "place=server:1\nchannel=1:11\nchannel=2:21\nchannel=1:12\n\
            member-sheet=1\nmember-sheet=2\nchannels-width=300\nmembers-width=264.5\nsearch-width=480\n";
        let preferences = Preferences::from_file_text(text);
        assert_eq!(
            preferences.last_place,
            Some(RememberedPlace::Server { guild_id: id(1) })
        );
        assert_eq!(preferences.last_channel_of(id(1)), Some(id(12)));
        assert_eq!(preferences.last_channel_of(id(2)), Some(id(21)));
        assert_eq!(preferences.last_channel_of(id(3)), None);
        assert_eq!(preferences.member_sheet_place_ids, [id(1), id(2)]);
        assert_eq!(preferences.channels_column_width, Some(300.0));
        assert_eq!(preferences.members_column_width, Some(264.5));
        assert_eq!(preferences.search_column_width, Some(480.0));
        assert_eq!(
            Preferences::from_file_text(&preferences.to_file_text()),
            preferences
        );
    }

    #[test]
    fn tampered_navigation_lines_are_ignored() {
        let text = "place=server:abc\nchannel=1:-2\nchannel=1\nmember-sheet=99999999999999999999999\n\
            channels-width=NaN\nmembers-width=-4\nplace=home:\nmedia-volume=1.5\nmedia-volume=NaN\n\
            media-muted=yes\ntheme=sepia\nlanguage=de\n";
        let preferences = Preferences::from_file_text(text);
        assert_eq!(preferences, Preferences::default());
    }

    #[test]
    fn voice_settings_toggles_and_people_survive_a_round_trip_and_tampering_keeps_defaults() {
        use mellow_client_core::shared_logic::voice_settings::{PushToTalkKey, VoiceInputMode};
        let mut preferences = Preferences {
            voice_settings: VoiceSettings {
                input_device_id: Some("pipewire:alsa_input.usb-Blue_Yeti".into()),
                input_mode: VoiceInputMode::PushToTalk,
                push_to_talk_key: Some(PushToTalkKey {
                    code: "KeyV".into(),
                    label: "V".into(),
                }),
                ..VoiceSettings::DEFAULT
            },
            voice_toggles: StoredVoiceToggles {
                is_microphone_muted: true,
                is_deafened: false,
                was_microphone_muted_before_deafening: false,
            },
            ..Preferences::default()
        };
        preferences.set_voice_person(
            id(7),
            PersonVoiceSetting {
                volume_percent: 40,
                is_locally_muted: true,
            },
        );
        preferences.set_voice_person(id(8), PersonVoiceSetting::default());
        assert_eq!(preferences.voice_people.len(), 1);
        assert_eq!(
            Preferences::from_file_text(&preferences.to_file_text()),
            preferences
        );
        let tampered = "voice-settings={not json\nvoice-toggles=[1]\nvoice-people=\"x\"\n";
        assert_eq!(
            Preferences::from_file_text(tampered),
            Preferences::default()
        );
    }

    #[test]
    fn remembered_places_stay_bounded_and_most_recent_last() {
        let mut preferences = Preferences::default();
        for raw in 0..=u64::try_from(MAXIMUM_REMEMBERED_PLACES).unwrap() {
            preferences.remember_member_sheet(id(raw), true);
            preferences.remember_channel(id(raw), id(raw + 1000));
        }
        assert_eq!(
            preferences.member_sheet_place_ids.len(),
            MAXIMUM_REMEMBERED_PLACES
        );
        assert_eq!(preferences.member_sheet_place_ids.first(), Some(&id(1)));
        assert_eq!(
            preferences.last_channel_by_guild.len(),
            MAXIMUM_REMEMBERED_PLACES
        );
        preferences.remember_member_sheet(id(5), false);
        assert!(!preferences.member_sheet_place_ids.contains(&id(5)));
    }

    #[test]
    fn notification_choices_survive_a_round_trip_per_account_and_stay_bounded() {
        let mut preferences = Preferences::default();
        let settings = NotificationSettings {
            are_desktop_notifications_enabled: true,
            is_sound_enabled: false,
            ..NotificationSettings::default()
        };
        preferences.set_notification_settings(id(1), settings.clone());
        let read = Preferences::from_file_text(&preferences.to_file_text());
        assert_eq!(read.notification_settings_of(id(1)), settings);
        assert_eq!(
            read.notification_settings_of(id(2)),
            NotificationSettings::default()
        );
        for raw in 2..=u64::try_from(MAXIMUM_REMEMBERED_NOTIFICATION_ACCOUNTS).unwrap() + 1 {
            preferences.set_notification_settings(id(raw), NotificationSettings::default());
        }
        assert_eq!(
            preferences.notification_settings_by_user.len(),
            MAXIMUM_REMEMBERED_NOTIFICATION_ACCOUNTS
        );
        assert_eq!(
            preferences.notification_settings_of(id(1)),
            NotificationSettings::default(),
            "the oldest account is forgotten"
        );
        let tampered =
            Preferences::from_file_text("notifications=1:{not json\nnotifications=x:{}\n");
        assert_eq!(tampered.notification_settings_by_user, []);
    }

    #[test]
    fn recent_emoji_survive_a_round_trip_per_account_and_only_as_known_emoji() {
        let mut preferences = Preferences::default();
        preferences.remember_emoji(id(1), "👍");
        preferences.remember_emoji(id(1), "❤️");
        preferences.remember_emoji(id(2), "🔥");
        let read = Preferences::from_file_text(&preferences.to_file_text());
        assert_eq!(read.recent_emoji_of(id(1)), ["❤️", "👍"]);
        assert_eq!(read.recent_emoji_of(id(2)), ["🔥"]);
        assert_eq!(read.recent_emoji_of(id(3)), [] as [String; 0]);
        for raw in 3..=u64::try_from(MAXIMUM_REMEMBERED_RECENT_EMOJI_ACCOUNTS).unwrap() + 1 {
            preferences.remember_emoji(id(raw), "👍");
        }
        assert!(
            preferences.recent_emoji_of(id(1)).is_empty(),
            "the oldest account is forgotten"
        );
        let tampered = Preferences::from_file_text(
            "recent-emoji=1:[\"👍\",3,\"<b>\",\"lol\"]\nrecent-emoji=2:{\"a\":1}\nrecent-emoji=x:[]\n",
        );
        assert_eq!(
            tampered.recent_emoji_by_user,
            [(id(1), vec!["👍".to_owned()])]
        );
    }

    #[gpui::test]
    fn changes_asked_while_the_file_is_read_apply_on_top_of_it(cx: &mut gpui::TestAppContext) {
        cx.update(|cx| {
            cx.set_global(PreferencesStore {
                loaded: None,
                pending_changes: Vec::new(),
                is_saved_to_disk: false,
            });
            change_preferences(cx, |preferences| preferences.remember_channel(id(1), id(2)));
            assert_eq!(preferences(cx), None);
            take_loaded_preferences(with_instance("chat.example.org"), cx);
            let loaded = preferences(cx).unwrap();
            assert_eq!(loaded.last_channel_of(id(1)), Some(id(2)));
            assert!(loaded.last_instance_address.is_some());
        });
    }
}
