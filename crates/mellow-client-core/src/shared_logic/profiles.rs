//! A member's profile and the reader's own custom status: `user-profile-loader.svelte.ts`, the
//! shown links of `ProfileDetails.svelte` and `custom-status.ts` (ADR-0050, DESIGN.md §5.3, §5.7).

use mellow_protocol::{
    ApiErrorCode, MAXIMUM_CUSTOM_STATUS_LENGTH, ProfileLink, Snowflake, UserProfile,
};
use url::Url;

use super::markdown::safe_http_url;
use crate::ClientError;

/// How many characters the custom status may still take, negative when it is too long. Counted as
/// the instance does: Unicode scalar values of the trimmed text, so an emoji is not two.
#[must_use]
pub fn custom_status_characters_left(draft: &str) -> i64 {
    let used = draft.trim().chars().count();
    i64::try_from(MAXIMUM_CUSTOM_STATUS_LENGTH).unwrap_or(i64::MAX)
        - i64::try_from(used).unwrap_or(i64::MAX)
}

/// A profile link as the card shows it: only http(s) addresses, named by their label or host.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShownProfileLink {
    /// Normalised by `safe_http_url`; the only address opened.
    pub url: String,
    pub host: String,
    /// Trimmed; `None` when blank.
    pub label: Option<String>,
    pub icon_id: Option<Snowflake>,
}

/// The links a profile card lists, in the user's order; any other scheme is left out.
#[must_use]
pub fn shown_profile_links(links: &[ProfileLink]) -> Vec<ShownProfileLink> {
    links
        .iter()
        .filter_map(|link| {
            let url = safe_http_url(&link.url)?;
            let host = Url::parse(&url).ok()?.host_str()?.to_owned();
            let label = link
                .label
                .as_deref()
                .map(str::trim)
                .filter(|label| !label.is_empty())
                .map(str::to_owned);
            Some(ShownProfileLink {
                url,
                host,
                label,
                icon_id: link.icon_id,
            })
        })
        .collect()
}

/// Where the profile on screen stands. Its "about me" and links are read when it opens, since no
/// gateway event carries them.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum UserProfileLoad {
    #[default]
    Idle,
    Loading {
        user_id: Snowflake,
    },
    Loaded {
        user_id: Snowflake,
        profile: UserProfile,
    },
    Failed {
        user_id: Snowflake,
    },
}

/// Which request an answer belongs to: an answer for a profile no longer shown is dropped.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UserProfileTicket {
    generation: u64,
    user_id: Snowflake,
}

impl UserProfileTicket {
    #[must_use]
    pub fn user_id(self) -> Snowflake {
        self.user_id
    }
}

/// The web's `createUserProfileLoader`, without the request: the host sends it with the ticket
/// `show` returns and hands the answer to `finish`.
#[derive(Debug, Default)]
pub struct UserProfileLoader {
    current: UserProfileLoad,
    generation: u64,
}

impl UserProfileLoader {
    #[must_use]
    pub fn current(&self) -> &UserProfileLoad {
        &self.current
    }

    /// Starts reading `user_id`'s profile; `None` forgets the one shown.
    pub fn show(&mut self, user_id: Option<Snowflake>) -> Option<UserProfileTicket> {
        self.generation += 1;
        let Some(user_id) = user_id else {
            self.current = UserProfileLoad::Idle;
            return None;
        };
        self.current = UserProfileLoad::Loading { user_id };
        Some(UserProfileTicket {
            generation: self.generation,
            user_id,
        })
    }

    /// Applies the answer to `ticket`'s request, unless another profile was shown since.
    pub fn finish(
        &mut self,
        ticket: UserProfileTicket,
        answer: Result<UserProfile, ClientError>,
    ) -> bool {
        if ticket.generation != self.generation {
            return false;
        }
        let user_id = ticket.user_id;
        self.current = match answer {
            Ok(profile) => UserProfileLoad::Loaded { user_id, profile },
            // Someone the reader shares nothing with any more: their profile simply has no details.
            Err(ClientError::Api {
                code: ApiErrorCode::NotFound,
                ..
            }) => UserProfileLoad::Loaded {
                user_id,
                profile: UserProfile {
                    user_id,
                    about_me: String::new(),
                    links: Vec::new(),
                },
            },
            Err(_) => UserProfileLoad::Failed { user_id },
        };
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(raw: u64) -> Snowflake {
        Snowflake::from_raw(raw)
    }

    fn profile(user_id: Snowflake) -> UserProfile {
        UserProfile {
            user_id,
            about_me: format!("about {user_id}"),
            links: Vec::new(),
        }
    }

    fn api_error(status: u16, code: ApiErrorCode) -> ClientError {
        ClientError::Api {
            status,
            code,
            message: String::new(),
        }
    }

    #[test]
    fn custom_status_counts_the_trimmed_text_as_the_instance_does() {
        let maximum = i64::try_from(MAXIMUM_CUSTOM_STATUS_LENGTH).unwrap();
        assert_eq!(custom_status_characters_left(""), maximum);
        assert_eq!(
            custom_status_characters_left("  En réunion  "),
            maximum - 10
        );
    }

    #[test]
    fn custom_status_counts_an_emoji_outside_the_basic_plane_as_one_character() {
        let maximum = i64::try_from(MAXIMUM_CUSTOM_STATUS_LENGTH).unwrap();
        assert_eq!(custom_status_characters_left("🎧"), maximum - 1);
    }

    #[test]
    fn custom_status_goes_negative_past_the_limit() {
        assert_eq!(
            custom_status_characters_left(&"é".repeat(MAXIMUM_CUSTOM_STATUS_LENGTH)),
            0
        );
        assert_eq!(
            custom_status_characters_left(&"é".repeat(MAXIMUM_CUSTOM_STATUS_LENGTH + 3)),
            -3
        );
    }

    #[test]
    fn shown_links_keep_http_addresses_named_by_label_or_host() {
        let links = [
            ProfileLink {
                url: "https://Example.org/blog".to_owned(),
                label: Some("  Blog ".to_owned()),
                icon_id: Some(id(950)),
            },
            ProfileLink {
                url: "javascript:alert(1)".to_owned(),
                label: None,
                icon_id: None,
            },
            ProfileLink {
                url: "https://paypal.com@evil.example/".to_owned(),
                label: None,
                icon_id: None,
            },
            ProfileLink {
                url: "https://example.com/".to_owned(),
                label: Some("   ".to_owned()),
                icon_id: None,
            },
        ];
        assert_eq!(
            shown_profile_links(&links),
            vec![
                ShownProfileLink {
                    url: "https://example.org/blog".to_owned(),
                    host: "example.org".to_owned(),
                    label: Some("Blog".to_owned()),
                    icon_id: Some(id(950)),
                },
                ShownProfileLink {
                    url: "https://example.com/".to_owned(),
                    host: "example.com".to_owned(),
                    label: None,
                    icon_id: None,
                },
            ]
        );
    }

    #[test]
    fn the_loader_keeps_only_the_answer_for_the_profile_shown_last() {
        let mut loader = UserProfileLoader::default();
        let first = loader.show(Some(id(1))).unwrap();
        let second = loader.show(Some(id(2))).unwrap();
        assert!(loader.finish(second, Ok(profile(id(2)))));
        assert!(!loader.finish(first, Ok(profile(id(1)))));
        assert_eq!(
            loader.current(),
            &UserProfileLoad::Loaded {
                user_id: id(2),
                profile: profile(id(2)),
            }
        );
    }

    #[test]
    fn the_loader_shows_no_details_for_someone_it_may_not_read_and_a_failure_otherwise() {
        let mut hidden = UserProfileLoader::default();
        let ticket = hidden.show(Some(id(1))).unwrap();
        hidden.finish(ticket, Err(api_error(404, ApiErrorCode::NotFound)));
        assert_eq!(
            hidden.current(),
            &UserProfileLoad::Loaded {
                user_id: id(1),
                profile: UserProfile {
                    user_id: id(1),
                    about_me: String::new(),
                    links: Vec::new(),
                },
            }
        );

        let mut broken = UserProfileLoader::default();
        let ticket = broken.show(Some(id(1))).unwrap();
        broken.finish(ticket, Err(ClientError::NotSignedIn));
        assert_eq!(
            broken.current(),
            &UserProfileLoad::Failed { user_id: id(1) }
        );
        assert!(broken.show(None).is_none());
        assert_eq!(broken.current(), &UserProfileLoad::Idle);
    }
}
