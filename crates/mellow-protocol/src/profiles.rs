//! A member's richer profile (ADR-0050): an "about me" text and links to their other places.

use serde::{Deserialize, Serialize};

use crate::Snowflake;

/// Characters of an "about me" text, counted as Unicode scalar values (Discord's limit).
pub const MAXIMUM_ABOUT_ME_LENGTH: usize = 190;
pub const MAXIMUM_PROFILE_LINKS: usize = 5;
/// Characters of a profile link's address.
pub const MAXIMUM_PROFILE_LINK_URL_LENGTH: usize = 300;
/// Characters of a profile link's label.
pub const MAXIMUM_PROFILE_LINK_LABEL_LENGTH: usize = 40;

/// Body of `GET /api/v1/users/{user_id}/profile` (and `/users/@me/profile`): readable by the user
/// and by whoever shares a server, a conversation or a relationship with them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UserProfile {
    pub user_id: Snowflake,
    /// Inline Markdown only (no headings, lists, quotes or code blocks); empty when none.
    #[serde(default)]
    pub about_me: String,
    /// In the order the user chose.
    #[serde(default)]
    pub links: Vec<ProfileLink>,
}

/// One of a profile's links: an `https` address, as the user wrote it once checked.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProfileLink {
    pub url: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    /// The site's icon, fetched and stored by the instance (never by browsers), served at
    /// `GET /api/v1/link-icons/{id}`. Absent until fetched, and when the site has none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon_id: Option<Snowflake>,
}

/// One link of `UpdateUserProfileRequest`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NewProfileLink {
    pub url: String,
    /// Blank or absent shows the address's host instead.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
}

/// Body of `PUT /api/v1/users/@me/profile`: replaces both fields. Answered with the stored
/// `UserProfile`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UpdateUserProfileRequest {
    #[serde(default)]
    pub about_me: String,
    #[serde(default)]
    pub links: Vec<NewProfileLink>,
}

/// Body of `PUT /api/v1/guilds/{guild_id}/roles/{role_id}/icon/emoji`: the role's icon becomes
/// this Unicode emoji, in place of any picture (ADR-0050).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SetRoleEmojiRequest {
    pub unicode_emoji: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn links_leave_out_what_they_lack_and_requests_refuse_unknown_fields() {
        let link = ProfileLink {
            url: "https://example.org/".into(),
            label: None,
            icon_id: None,
        };
        assert_eq!(
            serde_json::to_value(&link).unwrap(),
            serde_json::json!({ "url": "https://example.org/" })
        );
        let request = r#"{"about_me":"hi","links":[],"pronouns":"she/her"}"#;
        assert!(serde_json::from_str::<UpdateUserProfileRequest>(request).is_err());
        let profile: UserProfile = serde_json::from_str(r#"{"user_id":"7"}"#).unwrap();
        assert!(profile.about_me.is_empty() && profile.links.is_empty());
    }
}
