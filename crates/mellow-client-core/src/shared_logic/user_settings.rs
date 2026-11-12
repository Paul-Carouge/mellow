//! User settings' rules (`UserSettings.svelte` and its pages): the sections, the live name check
//! of `name-availability.svelte.ts`, the "About me" and links draft of `profile-details-form.ts`
//! and the two-factor helpers of `two-factor.ts` (ADR-0048, ADR-0049, ADR-0050).

use mellow_protocol::{
    MAXIMUM_ABOUT_ME_LENGTH, MAXIMUM_PROFILE_LINK_LABEL_LENGTH, MAXIMUM_PROFILE_LINK_URL_LENGTH,
    NewProfileLink, ProfileLink, UpdateUserProfileRequest, UserProfile,
};
use url::Url;

/// The user settings' sections, in the navigation's order: the account's own, then the app's.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum UserSettingsSection {
    MyAccount,
    Profile,
    Security,
    /// The desktop's theme and language (the web follows the browser for both).
    Appearance,
    Notifications,
    About,
}

impl UserSettingsSection {
    pub const ACCOUNT_GROUP: [Self; 3] = [Self::MyAccount, Self::Profile, Self::Security];
    pub const APP_GROUP: [Self; 3] = [Self::Appearance, Self::Notifications, Self::About];
}

/// Trimmed, with inner runs of white space made one space, as the instance stores names.
#[must_use]
pub fn tidied_name(name: &str) -> String {
    name.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Where the live "is this name free?" check stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum NameAvailabilityState {
    #[default]
    Idle,
    Checking,
    Available,
    Taken,
    /// The name breaks the naming rules (`VALIDATION_FAILED`).
    Invalid,
    /// The check could not answer; the save still checks.
    Unknown,
}

/// One request to make once typing pauses, tagged so a late answer about an older draft is
/// dropped.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NameCheckRequest {
    pub generation: u64,
    pub name: String,
}

/// What a check learnt from the instance.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NameCheckAnswer {
    Available,
    Taken,
    Refused,
    Failed,
}

/// The live name check (ADR-0048) without its timer: the client waits
/// [`NAME_CHECK_PAUSE_MILLISECONDS`] after the last `update`, then asks for the request's name.
#[derive(Debug, Clone, Default)]
pub struct NameAvailabilityCheck {
    state: NameAvailabilityState,
    generation: u64,
}

/// Long enough that a typed word sends one request, short enough to feel live.
pub const NAME_CHECK_PAUSE_MILLISECONDS: u64 = 400;

impl NameAvailabilityCheck {
    #[must_use]
    pub fn state(&self) -> NameAvailabilityState {
        self.state
    }

    /// The draft changed: what to ask once typing pauses, nothing while it is empty or the
    /// current name. Any earlier request is now stale.
    pub fn update(&mut self, draft: &str, current_name: &str) -> Option<NameCheckRequest> {
        self.generation += 1;
        let name = tidied_name(draft);
        if name.is_empty() || name == current_name {
            self.state = NameAvailabilityState::Idle;
            return None;
        }
        self.state = NameAvailabilityState::Checking;
        Some(NameCheckRequest {
            generation: self.generation,
            name,
        })
    }

    /// Whether `generation` is still the draft's: the client skips a request gone stale.
    #[must_use]
    pub fn is_current(&self, generation: u64) -> bool {
        generation == self.generation
    }

    /// Applies an answer, unless a newer draft replaced the one it was about.
    pub fn finish(&mut self, generation: u64, answer: NameCheckAnswer) {
        if generation != self.generation {
            return;
        }
        self.state = match answer {
            NameCheckAnswer::Available => NameAvailabilityState::Available,
            NameCheckAnswer::Taken => NameAvailabilityState::Taken,
            NameCheckAnswer::Refused => NameAvailabilityState::Invalid,
            NameCheckAnswer::Failed => NameAvailabilityState::Unknown,
        };
    }

    /// A save found the name taken meanwhile: shown under the field, as the live check does.
    pub fn mark_taken(&mut self) {
        self.state = NameAvailabilityState::Taken;
    }

    /// Stops a pending check, so a closed form gets no late answer.
    pub fn cancel(&mut self) {
        self.generation += 1;
        self.state = NameAvailabilityState::Idle;
    }
}

/// Whether the name form may be saved: a new, non-empty name not known to be taken or invalid.
#[must_use]
pub fn can_save_display_name(
    draft: &str,
    current_name: &str,
    state: NameAvailabilityState,
) -> bool {
    let name = tidied_name(draft);
    !name.is_empty()
        && name != current_name
        && !matches!(
            state,
            NameAvailabilityState::Taken | NameAvailabilityState::Invalid
        )
}

/// Whether the password form may be sent: every field filled, the new password long enough and
/// typed the same twice.
#[must_use]
pub fn can_change_password(
    current_password: &str,
    new_password: &str,
    confirmation: &str,
    minimum_length: usize,
) -> bool {
    !current_password.is_empty()
        && new_password.chars().count() >= minimum_length
        && !confirmation.is_empty()
        && confirmation == new_password
}

/// One link row of the "About me" and links editor.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ProfileLinkDraft {
    pub url: String,
    pub label: String,
}

/// The editor's draft, as typed.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ProfileDraft {
    pub about_me: String,
    pub links: Vec<ProfileLinkDraft>,
}

/// What is wrong with a link row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProfileLinkProblem {
    NotHttps,
    TooLong,
}

impl ProfileDraft {
    #[must_use]
    pub fn of_profile(profile: &UserProfile) -> Self {
        Self {
            about_me: profile.about_me.clone(),
            links: profile
                .links
                .iter()
                .map(|link| ProfileLinkDraft {
                    url: link.url.clone(),
                    label: link.label.clone().unwrap_or_default(),
                })
                .collect(),
        }
    }

    /// Blank link rows are left out, so an empty row never blocks a save.
    #[must_use]
    pub fn request(&self) -> UpdateUserProfileRequest {
        UpdateUserProfileRequest {
            about_me: self.about_me.trim().to_owned(),
            links: self
                .links
                .iter()
                .filter(|link| !link.url.trim().is_empty())
                .map(|link| {
                    let label = link.label.trim();
                    NewProfileLink {
                        url: link.url.trim().to_owned(),
                        label: (!label.is_empty()).then(|| label.to_owned()),
                    }
                })
                .collect(),
        }
    }

    /// Characters as the instance counts them: Unicode scalar values of the trimmed text.
    #[must_use]
    pub fn about_me_length(&self) -> usize {
        self.about_me.trim().chars().count()
    }

    #[must_use]
    pub fn is_about_me_too_long(&self) -> bool {
        self.about_me_length() > MAXIMUM_ABOUT_ME_LENGTH
    }

    #[must_use]
    pub fn can_save(&self) -> bool {
        !self.is_about_me_too_long() && self.links.iter().all(|link| link_problem(link).is_none())
    }

    /// The draft's links the preview shows, each with the icon its host already has among the
    /// saved links, so a typed link shows the glyph until it is saved and fetched.
    #[must_use]
    pub fn preview_links(&self, saved: &[ProfileLink]) -> Vec<ProfileLink> {
        let icon_of_host = |host: &str| {
            saved.iter().find_map(|link| {
                (host_of(&link.url).as_deref() == Some(host))
                    .then_some(link.icon_id)
                    .flatten()
            })
        };
        self.request()
            .links
            .into_iter()
            .filter_map(|link| {
                let host = host_of(&link.url)?;
                let draft = ProfileLinkDraft {
                    url: link.url.clone(),
                    label: link.label.clone().unwrap_or_default(),
                };
                link_problem(&draft).is_none().then(|| ProfileLink {
                    icon_id: icon_of_host(&host),
                    url: link.url,
                    label: link.label,
                })
            })
            .collect()
    }
}

/// What is wrong with a link row; a blank row is fine (it is not sent).
#[must_use]
pub fn link_problem(link: &ProfileLinkDraft) -> Option<ProfileLinkProblem> {
    let url = link.url.trim();
    if url.is_empty() {
        return None;
    }
    if url.chars().count() > MAXIMUM_PROFILE_LINK_URL_LENGTH
        || link.label.trim().chars().count() > MAXIMUM_PROFILE_LINK_LABEL_LENGTH
    {
        return Some(ProfileLinkProblem::TooLong);
    }
    let is_plain_https = Url::parse(url).is_ok_and(|parsed| {
        parsed.scheme() == "https"
            && parsed.host_str().is_some_and(|host| !host.is_empty())
            && parsed.username().is_empty()
            && parsed.password().is_none()
    });
    (!is_plain_https).then_some(ProfileLinkProblem::NotHttps)
}

fn host_of(url: &str) -> Option<String> {
    let parsed = Url::parse(url).ok()?;
    let host = parsed.host_str()?;
    Some(host.strip_suffix('.').unwrap_or(host).to_lowercase())
}

/// Wrong codes older than this no longer warn on the Security page (thirty days).
const RECENT_WRONG_CODE_MILLISECONDS: u64 = 30 * 24 * 60 * 60 * 1000;

/// How apps name the account: the instance's host name keeps two Mellow accounts apart.
fn issuer(host_name: &str) -> String {
    format!("Mellow ({host_name})")
}

/// `encodeURIComponent`: everything but letters, digits and `-_.!~*'()` percent-encoded.
fn encode_uri_component(text: &str) -> String {
    use std::fmt::Write as _;
    let mut encoded = String::with_capacity(text.len());
    for byte in text.bytes() {
        if byte.is_ascii_alphanumeric() || b"-_.!~*'()".contains(&byte) {
            encoded.push(char::from(byte));
        } else {
            // Writing to a `String` cannot fail.
            let _ = write!(encoded, "%{byte:02X}");
        }
    }
    encoded
}

/// `URLSearchParams` encoding (letters, digits and `*-._` kept), with spaces as `%20`, since
/// some apps keep a `+` as is.
fn encode_query_value(text: &str) -> String {
    url::form_urlencoded::byte_serialize(text.as_bytes())
        .collect::<String>()
        .replace('+', "%20")
}

/// The `otpauth://` URI authenticator apps read from the QR code (Key Uri Format). Takes the host
/// name without the port: apps split the label at its first colon, encoded or not.
#[must_use]
pub fn otpauth_uri(secret: &str, username: &str, host_name: &str) -> String {
    let issuer = issuer(host_name);
    format!(
        "otpauth://totp/{}:{}?secret={}&issuer={}&algorithm=SHA1&digits=6&period=30",
        encode_uri_component(&issuer),
        encode_uri_component(username),
        encode_query_value(secret),
        encode_query_value(&issuer),
    )
}

/// The secret in groups of four, as people type it into an app by hand.
#[must_use]
pub fn grouped_secret(secret: &str) -> String {
    secret
        .chars()
        .collect::<Vec<_>>()
        .chunks(4)
        .map(|group| group.iter().collect::<String>())
        .collect::<Vec<_>>()
        .join(" ")
}

/// The text file a person keeps their recovery codes in.
#[must_use]
pub fn recovery_codes_file_text(heading: &str, codes: &[String]) -> String {
    format!("{heading}\n\n{}\n", codes.join("\n"))
}

/// Whether a wrong code was typed for the account lately: someone else may know the password.
#[must_use]
pub fn is_wrong_code_recent(
    last_failed_at_unix_milliseconds: Option<u64>,
    now_unix_milliseconds: u64,
) -> bool {
    last_failed_at_unix_milliseconds.is_some_and(|failed_at| {
        now_unix_milliseconds.saturating_sub(failed_at) < RECENT_WRONG_CODE_MILLISECONDS
    })
}

/// Six digits, as the code field accepts them (spaces from a paste are dropped).
#[must_use]
pub fn is_six_digit_code(typed: &str) -> bool {
    let digits: Vec<char> = typed
        .chars()
        .filter(|character| *character != ' ')
        .collect();
    digits.len() == 6 && digits.iter().all(char::is_ascii_digit)
}

#[cfg(test)]
mod tests {
    use mellow_protocol::Snowflake;

    use super::*;

    fn link(url: &str, label: &str) -> ProfileLinkDraft {
        ProfileLinkDraft {
            url: url.into(),
            label: label.into(),
        }
    }

    #[test]
    fn tidies_names_as_the_instance_stores_them() {
        assert_eq!(tidied_name("  Jean \t Paul "), "Jean Paul");
    }

    #[test]
    fn asks_once_per_draft_never_for_the_current_or_an_empty_name() {
        let mut check = NameAvailabilityCheck::default();
        let first = check.update("  Pa", "Aurore");
        let second = check.update("  Paul   C ", "Aurore");
        assert_eq!(check.state(), NameAvailabilityState::Checking);
        let (Some(first), Some(second)) = (first, second) else {
            panic!("both drafts are checked");
        };
        assert_eq!(second.name, "Paul C");
        assert!(!check.is_current(first.generation));
        check.finish(second.generation, NameCheckAnswer::Available);
        assert_eq!(check.state(), NameAvailabilityState::Available);
        assert_eq!(check.update(" Aurore ", "Aurore"), None);
        assert_eq!(check.update("", "Aurore"), None);
        assert_eq!(check.state(), NameAvailabilityState::Idle);
    }

    #[test]
    fn keeps_the_newest_drafts_answer_and_tells_a_refused_name_from_a_lost_answer() {
        let mut check = NameAvailabilityCheck::default();
        let old = check
            .update("Old", "Aurore")
            .map(|request| request.generation);
        let new = check
            .update("New", "Aurore")
            .map(|request| request.generation);
        let (Some(old), Some(new)) = (old, new) else {
            panic!("both drafts are checked");
        };
        check.finish(new, NameCheckAnswer::Available);
        check.finish(old, NameCheckAnswer::Taken);
        assert_eq!(check.state(), NameAvailabilityState::Available);

        let refused = check
            .update("a", "Aurore")
            .map(|request| request.generation);
        check.finish(refused.unwrap_or_default(), NameCheckAnswer::Refused);
        assert_eq!(check.state(), NameAvailabilityState::Invalid);
        let lost = check
            .update("Paul", "Aurore")
            .map(|request| request.generation);
        check.finish(lost.unwrap_or_default(), NameCheckAnswer::Failed);
        assert_eq!(check.state(), NameAvailabilityState::Unknown);
        let cancelled = check
            .update("Paula", "Aurore")
            .map(|request| request.generation);
        check.cancel();
        check.finish(cancelled.unwrap_or_default(), NameCheckAnswer::Available);
        assert_eq!(check.state(), NameAvailabilityState::Idle);
    }

    #[test]
    fn saves_a_new_name_unless_it_is_known_taken_or_invalid() {
        use NameAvailabilityState::{Available, Invalid, Taken, Unknown};
        assert!(can_save_display_name(" Paul ", "Aurore", Available));
        assert!(can_save_display_name("Paul", "Aurore", Unknown));
        assert!(!can_save_display_name("Paul", "Aurore", Taken));
        assert!(!can_save_display_name("Paul", "Aurore", Invalid));
        assert!(!can_save_display_name(" Aurore", "Aurore", Available));
        assert!(!can_save_display_name("  ", "Aurore", Available));
    }

    #[test]
    fn sends_a_password_change_only_when_complete_and_confirmed() {
        assert!(can_change_password("old", "longenough", "longenough", 8));
        assert!(!can_change_password("", "longenough", "longenough", 8));
        assert!(!can_change_password("old", "short", "short", 8));
        assert!(!can_change_password("old", "longenough", "longenougH", 8));
        assert!(!can_change_password("old", "longenough", "", 8));
        // Counted in characters, as the web's `[...text].length`.
        assert!(can_change_password("old", "éééééééé", "éééééééé", 8));
    }

    #[test]
    fn counts_characters_as_the_instance_does_emoji_included() {
        let draft = |about_me: String| ProfileDraft {
            about_me,
            links: Vec::new(),
        };
        assert_eq!(draft("é🔥".into()).about_me_length(), 2);
        assert!(draft("🔥".repeat(190)).can_save());
        assert!(!draft("🔥".repeat(191)).can_save());
    }

    #[test]
    fn accepts_only_https_links_and_leaves_blank_rows_out_of_the_request() {
        assert_eq!(link_problem(&link("https://example.org/a", "")), None);
        assert_eq!(link_problem(&link("  ", "")), None);
        assert_eq!(
            link_problem(&link("http://example.org/", "")),
            Some(ProfileLinkProblem::NotHttps)
        );
        assert_eq!(
            link_problem(&link("javascript:alert(1)", "")),
            Some(ProfileLinkProblem::NotHttps)
        );
        assert_eq!(
            link_problem(&link("example.org", "")),
            Some(ProfileLinkProblem::NotHttps)
        );
        assert_eq!(
            link_problem(&link("https://user@example.org/", "")),
            Some(ProfileLinkProblem::NotHttps)
        );
        assert_eq!(
            link_problem(&link(
                &format!("https://example.org/{}", "a".repeat(300)),
                ""
            )),
            Some(ProfileLinkProblem::TooLong)
        );
        let draft = ProfileDraft {
            about_me: "  hi  ".into(),
            links: vec![
                link(" https://example.org/ ", " Blog "),
                link("", "lost"),
                link("https://example.com/", "  "),
            ],
        };
        assert_eq!(
            draft.request(),
            UpdateUserProfileRequest {
                about_me: "hi".into(),
                links: vec![
                    NewProfileLink {
                        url: "https://example.org/".into(),
                        label: Some("Blog".into()),
                    },
                    NewProfileLink {
                        url: "https://example.com/".into(),
                        label: None,
                    },
                ],
            }
        );
    }

    #[test]
    fn previews_typed_links_with_the_icon_their_host_already_has() {
        let draft = ProfileDraft {
            about_me: String::new(),
            links: vec![link("https://Example.org/new", ""), link("http://bad/", "")],
        };
        let saved = [ProfileLink {
            url: "https://example.org/old".into(),
            label: None,
            icon_id: Some(Snowflake::from_raw(950)),
        }];
        assert_eq!(
            draft.preview_links(&saved),
            [ProfileLink {
                url: "https://Example.org/new".into(),
                label: None,
                icon_id: Some(Snowflake::from_raw(950)),
            }]
        );
    }

    #[test]
    fn names_the_instance_in_the_label_and_the_issuer_percent_encoded() {
        assert_eq!(
            otpauth_uri("JBSWY3DPEHPK3PXP", "ana.b", "chat.example.org"),
            "otpauth://totp/Mellow%20(chat.example.org):ana.b?secret=JBSWY3DPEHPK3PXP&issuer=Mellow%20%28chat.example.org%29&algorithm=SHA1&digits=6&period=30"
        );
    }

    #[test]
    fn warns_for_thirty_days_after_a_wrong_code() {
        let day = 24 * 60 * 60 * 1000;
        assert!(!is_wrong_code_recent(None, 100 * day));
        assert!(is_wrong_code_recent(Some(100 * day), 100 * day));
        assert!(is_wrong_code_recent(Some(100 * day), 130 * day - 1));
        assert!(!is_wrong_code_recent(Some(100 * day), 130 * day));
    }

    #[test]
    fn splits_the_key_in_groups_of_four() {
        assert_eq!(
            grouped_secret("JBSWY3DPEHPK3PXPJB"),
            "JBSW Y3DP EHPK 3PXP JB"
        );
        assert_eq!(grouped_secret(""), "");
    }

    #[test]
    fn tells_six_digits_and_writes_one_recovery_code_per_line() {
        assert!(is_six_digit_code("123 456"));
        assert!(!is_six_digit_code("12345"));
        assert!(!is_six_digit_code("12345a"));
        assert_eq!(
            recovery_codes_file_text("Codes", &["a".into(), "b".into()]),
            "Codes\n\na\nb\n"
        );
    }
}
