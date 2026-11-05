//! Role icons (ADR-0050): a small picture, animated like an emoji, or a Unicode emoji, shown next
//! to the names of the role's members. Pictures are guild images, served to members like icons.

use mellow_protocol::{AuditAction, Role, Snowflake};

use super::icons::{IconUpload, discard};
use super::{GuildError, Guilds};
use crate::domain::guild_rules::{GuildRuleViolation, MAXIMUM_EMOJI_STORED_BYTES};
use crate::domain::image_processing::{
    IconKind, ImageProcessingError, ProcessedProfileImage, process_icon,
};
use crate::domain::message_rules::{custom_emoji_of_reaction_key, reaction_emoji_key};
use crate::domain::messages::MessageError;
use crate::domain::profile_images::ProfileImages;
use crate::storage::guild_images::{self as storage, IconChange, RoleIconChange};
use crate::storage::guild_roles as role_storage;

/// What a role's icon becomes.
pub(crate) enum RoleIconRequest {
    Picture(IconUpload),
    /// Any text: checked to be one Unicode emoji.
    Emoji(String),
    Remove,
}

/// One Unicode emoji, without variation selectors (as reactions store them); never a custom
/// emoji, which a role icon would show to members of other servers' roles.
fn role_icon_emoji(emoji: &str) -> Result<String, GuildRuleViolation> {
    let emoji = emoji.trim();
    if custom_emoji_of_reaction_key(emoji).is_some() {
        return Err(GuildRuleViolation::RoleIconEmoji);
    }
    reaction_emoji_key(emoji).map_err(|_| GuildRuleViolation::RoleIconEmoji)
}

/// The same size cap as custom emoji, whose pictures role icons resemble.
fn within_role_icon_stored_size(
    processed: ProcessedProfileImage,
) -> Result<ProcessedProfileImage, ImageProcessingError> {
    let stored_bytes = processed.picture.bytes.len() + processed.still.as_ref().map_or(0, Vec::len);
    if stored_bytes > MAXIMUM_EMOJI_STORED_BYTES {
        return Err(ImageProcessingError::TooLarge);
    }
    Ok(processed)
}

impl Guilds {
    /// Needs `MANAGE_ROLES` over the role (`RoleManager`); `@everyone` has no icon. A picture
    /// replaces an emoji and the other way round. Every member sees the guild's roles again
    /// (`GUILD_ROLES_UPDATE`).
    pub(crate) async fn replace_role_icon(
        &self,
        images: &ProfileImages,
        user_id: Snowflake,
        guild_id: Snowflake,
        role_id: Snowflake,
        request: RoleIconRequest,
        now_unix_milliseconds: u64,
    ) -> Result<Role, MessageError> {
        if role_id == guild_id {
            return Err(GuildError::from(GuildRuleViolation::EveryoneRole).into());
        }
        let emoji = match &request {
            RoleIconRequest::Emoji(emoji) => {
                Some(role_icon_emoji(emoji).map_err(GuildError::from)?)
            }
            RoleIconRequest::Picture(_) | RoleIconRequest::Remove => None,
        };
        // Checked before processing too, so nobody else can spend processing slots.
        let (_, role) = self
            .manager_and_manageable_role(user_id, guild_id, role_id)
            .await?;
        let new_image = match request {
            RoleIconRequest::Picture(IconUpload { bytes, crop }) => Some(
                images
                    .store(
                        bytes,
                        move |bytes| {
                            process_icon(bytes, IconKind::Role, crop)
                                .and_then(within_role_icon_stored_size)
                        },
                        now_unix_milliseconds,
                    )
                    .await?,
            ),
            RoleIconRequest::Emoji(_) | RoleIconRequest::Remove => None,
        };
        // Processing takes seconds: the permission or the rank may have changed meanwhile.
        let audit_after_permission_recheck = async {
            self.manager_and_manageable_role(user_id, guild_id, role_id)
                .await?;
            self.audit_entry(
                guild_id,
                user_id,
                AuditAction::RoleUpdate,
                Some(role_id),
                None,
            )
        }
        .await;
        let audit = match audit_after_permission_recheck {
            Ok(audit) => audit,
            Err(error) => {
                discard(images, new_image).await;
                return Err(error.into());
            }
        };
        let change = match (new_image, emoji.as_deref()) {
            (Some(image), _) => RoleIconChange::Picture(image),
            (None, Some(emoji)) => RoleIconChange::Emoji(emoji),
            (None, None) => RoleIconChange::None,
        };
        let outcome = storage::replace_role_icon(
            &self.database,
            guild_id,
            role_id,
            change,
            &audit,
            now_unix_milliseconds,
        )
        .await
        .map_err(GuildError::from);
        let updated = match outcome {
            Ok(IconChange::Changed { updated, replaced }) => {
                if let Some(replaced) = replaced {
                    images.delete_files(replaced.id, replaced.still_id).await;
                }
                updated
            }
            Ok(IconChange::Unchanged) => return Ok(role),
            Ok(IconChange::NotFound) => {
                discard(images, new_image).await;
                return Err(GuildError::NotFound.into());
            }
            Err(error) => {
                discard(images, new_image).await;
                return Err(error.into());
            }
        };
        self.publish_roles(
            guild_id,
            role_storage::list_roles(&self.database, guild_id)
                .await
                .map_err(GuildError::from)?,
        );
        Ok(updated)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn role_icon_emoji_are_one_unicode_emoji() {
        assert_eq!(role_icon_emoji(" \u{2764}\u{FE0F} ").unwrap(), "\u{2764}");
        assert_eq!(
            role_icon_emoji("\u{1F9D1}\u{200D}\u{1F4BB}").unwrap(),
            "\u{1F9D1}\u{200D}\u{1F4BB}"
        );
        for refused in [":123", "ab", "", "\u{1F525}\u{1F525}", "<:x:1>"] {
            assert_eq!(
                role_icon_emoji(refused),
                Err(GuildRuleViolation::RoleIconEmoji),
                "{refused}"
            );
        }
    }
}
