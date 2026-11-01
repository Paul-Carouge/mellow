//! Custom emoji (ADR-0028): small square pictures a guild names, used in messages and reactions.
//! Their pictures are guild images, served to members like icons.

use mellow_protocol::{AuditAction, CustomEmoji, GatewayEvent, Permissions, Snowflake};

use super::icons::discard;
use super::{GuildError, Guilds};
use crate::domain::guild_rules::{
    GuildRuleViolation, MAXIMUM_EMOJI_STORED_BYTES, MAXIMUM_EMOJIS_PER_GUILD_AND_KIND,
    normalize_emoji_name,
};
use crate::domain::image_processing::{
    IconKind, ImageProcessingError, ProcessedProfileImage, may_be_animated, process_icon,
};
use crate::domain::messages::MessageError;
use crate::domain::profile_images::ProfileImages;
use crate::storage::guild_emojis::{self as storage, EmojiDeleteOutcome, EmojiWriteOutcome};

const EMOJI_LIMIT_MESSAGE: &str = "a server holds at most 50 still and 50 animated emoji";

impl Guilds {
    /// The guild's emoji, for its members only (`NotFound` otherwise).
    pub(crate) async fn list_emojis(
        &self,
        user_id: Snowflake,
        guild_id: Snowflake,
    ) -> Result<Vec<CustomEmoji>, GuildError> {
        self.membership_and_permissions(guild_id, user_id).await?;
        Ok(storage::list_guild_emojis(&self.database, guild_id).await?)
    }

    /// Needs `MANAGE_EXPRESSIONS`. The picture's centred square is kept, animated if the picture is.
    /// Every member sees the guild's new list (`GUILD_EMOJIS_UPDATE`).
    pub(crate) async fn create_emoji(
        &self,
        images: &ProfileImages,
        user_id: Snowflake,
        guild_id: Snowflake,
        name: &str,
        bytes: Vec<u8>,
        now_unix_milliseconds: u64,
    ) -> Result<CustomEmoji, MessageError> {
        let name = normalize_emoji_name(name).map_err(GuildError::from)?;
        // Checked before processing too, so nobody else can spend processing slots.
        self.require_member_permissions(guild_id, user_id, Permissions::MANAGE_EXPRESSIONS)
            .await?;
        // Cheap refusal before processing; the insert's own count stays the one that decides.
        let (still_count, animated_count) =
            storage::count_guild_emojis_by_kind(&self.database, guild_id)
                .await
                .map_err(GuildError::from)?;
        let is_quota_full_for_this_upload = still_count >= MAXIMUM_EMOJIS_PER_GUILD_AND_KIND
            && (animated_count >= MAXIMUM_EMOJIS_PER_GUILD_AND_KIND || !may_be_animated(&bytes));
        if is_quota_full_for_this_upload {
            return Err(GuildError::LimitReached(EMOJI_LIMIT_MESSAGE).into());
        }
        let image = images
            .store(
                bytes,
                |bytes| {
                    process_icon(bytes, IconKind::Emoji, None).and_then(within_emoji_stored_size)
                },
                now_unix_milliseconds,
            )
            .await?;
        let new_image = Some(image);
        // Processing takes seconds: the permission may have been taken away meanwhile.
        let audit_after_permission_recheck = async {
            self.require_member_permissions(guild_id, user_id, Permissions::MANAGE_EXPRESSIONS)
                .await?;
            self.audit_entry(
                guild_id,
                user_id,
                AuditAction::EmojiCreate,
                Some(image.id),
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
        let outcome = storage::insert_guild_emoji(
            &self.database,
            guild_id,
            &name,
            image,
            MAXIMUM_EMOJIS_PER_GUILD_AND_KIND,
            &audit,
            now_unix_milliseconds,
        )
        .await
        .map_err(GuildError::from);
        let emojis = match outcome.and_then(emojis_written) {
            Ok(emojis) => emojis,
            Err(error) => {
                discard(images, new_image).await;
                return Err(error.into());
            }
        };
        let created = emojis.iter().find(|emoji| emoji.id == image.id).cloned();
        self.publish_emojis(guild_id, emojis);
        created.ok_or_else(|| GuildError::Internal("a created emoji was not listed".into()).into())
    }

    /// Needs `MANAGE_EXPRESSIONS`. Messages keep the text they were written with; reactions are keyed
    /// by id, so they follow the new name (ADR-0028).
    pub(crate) async fn rename_emoji(
        &self,
        user_id: Snowflake,
        guild_id: Snowflake,
        emoji_id: Snowflake,
        name: &str,
    ) -> Result<CustomEmoji, GuildError> {
        let name = normalize_emoji_name(name)?;
        self.require_member_permissions(guild_id, user_id, Permissions::MANAGE_EXPRESSIONS)
            .await?;
        let audit = self.audit_entry(
            guild_id,
            user_id,
            AuditAction::EmojiUpdate,
            Some(emoji_id),
            None,
        )?;
        let outcome =
            storage::rename_guild_emoji(&self.database, guild_id, emoji_id, &name, &audit).await?;
        if let EmojiWriteOutcome::Unchanged(emoji) = outcome {
            return Ok(emoji);
        }
        let emojis = emojis_written(outcome)?;
        let renamed = emojis.iter().find(|emoji| emoji.id == emoji_id).cloned();
        self.publish_emojis(guild_id, emojis);
        renamed.ok_or(GuildError::NotFound)
    }

    /// Needs `MANAGE_EXPRESSIONS`. Messages keep their `<:name:id>` text and reactions with the emoji
    /// stay (ADR-0028); its picture is no longer served.
    pub(crate) async fn delete_emoji(
        &self,
        images: &ProfileImages,
        user_id: Snowflake,
        guild_id: Snowflake,
        emoji_id: Snowflake,
    ) -> Result<(), MessageError> {
        self.require_member_permissions(guild_id, user_id, Permissions::MANAGE_EXPRESSIONS)
            .await?;
        let audit = self.audit_entry(
            guild_id,
            user_id,
            AuditAction::EmojiDelete,
            Some(emoji_id),
            None,
        )?;
        let outcome = storage::delete_guild_emoji(&self.database, guild_id, emoji_id, &audit)
            .await
            .map_err(GuildError::from)?;
        let EmojiDeleteOutcome::Deleted { emojis, image } = outcome else {
            return Err(GuildError::NotFound.into());
        };
        images.delete_files(image.id, image.still_id).await;
        self.publish_emojis(guild_id, emojis);
        Ok(())
    }

    fn publish_emojis(&self, guild_id: Snowflake, emojis: Vec<CustomEmoji>) {
        self.events.publish_to_guild(
            guild_id,
            &GatewayEvent::GuildEmojisUpdate { guild_id, emojis },
        );
    }
}

/// Refuses an emoji whose stored files (the picture and its still frame) pass
/// `MAXIMUM_EMOJI_STORED_BYTES`: every member downloads them, often many at once.
fn within_emoji_stored_size(
    processed: ProcessedProfileImage,
) -> Result<ProcessedProfileImage, ImageProcessingError> {
    let stored_bytes = processed.picture.bytes.len() + processed.still.as_ref().map_or(0, Vec::len);
    if stored_bytes > MAXIMUM_EMOJI_STORED_BYTES {
        return Err(ImageProcessingError::TooLarge);
    }
    Ok(processed)
}

/// The guild's emoji after a write, or why it was refused.
fn emojis_written(outcome: EmojiWriteOutcome) -> Result<Vec<CustomEmoji>, GuildError> {
    match outcome {
        EmojiWriteOutcome::Written(emojis) => Ok(emojis),
        EmojiWriteOutcome::Unchanged(_) => Err(GuildError::Internal(
            "only a rename answers unchanged, and it handles it first".into(),
        )),
        EmojiWriteOutcome::NotFound => Err(GuildError::NotFound),
        EmojiWriteOutcome::NameTaken => Err(GuildRuleViolation::EmojiNameTaken.into()),
        EmojiWriteOutcome::LimitReached => Err(GuildError::LimitReached(EMOJI_LIMIT_MESSAGE)),
    }
}
#[cfg(test)]
mod tests {
    use mellow_protocol::{
        AuditLogQuery, CreateGuildRequest, CreateInviteRequest, CreateRoleRequest, Guild,
        MessageHistoryQuery, OpenDirectMessageRequest, SendMessageRequest,
    };

    use super::*;
    use crate::domain::account_rules::RegistrationMode;
    use crate::domain::accounts::tests::{NOW, migrated_database, register_request};
    use crate::domain::image_processing::tests::{animated_gif, png};
    use crate::domain::message_rules::MessageRuleViolation;
    use crate::http::AppState;
    use crate::http::tests::test_state;
    use crate::storage::profile_images::NewProfileImage;

    /// A guild of `owner` that `member` joined; `outsider` belongs to none.
    struct Club {
        state: AppState,
        owner: Snowflake,
        member: Snowflake,
        outsider: Snowflake,
        guild: Guild,
    }

    async fn club(state: AppState) -> Club {
        let register = |name: &str| {
            state
                .accounts
                .register(register_request(name, &format!("{name}@example.org")), NOW)
        };
        let owner = register("owner").await.unwrap().user.id;
        let member = register("member").await.unwrap().user.id;
        let outsider = register("outsider").await.unwrap().user.id;
        let guild = guild_of(&state, owner, "Club").await;
        let invite = state
            .guilds
            .create_invite(owner, guild.id, CreateInviteRequest::default(), NOW)
            .await
            .unwrap();
        state
            .guilds
            .join_with_invite(member, &invite.code, NOW)
            .await
            .unwrap();
        Club {
            state,
            owner,
            member,
            outsider,
            guild,
        }
    }

    async fn guild_of(state: &AppState, owner: Snowflake, name: &str) -> Guild {
        let request = CreateGuildRequest {
            name: name.into(),
            initial_categories: None,
        };
        state
            .guilds
            .create_guild(owner, false, request, NOW)
            .await
            .unwrap()
    }

    impl Club {
        async fn create(
            &self,
            user_id: Snowflake,
            name: &str,
        ) -> Result<CustomEmoji, MessageError> {
            self.create_from(user_id, name, png(64, 40)).await
        }

        async fn create_from(
            &self,
            user_id: Snowflake,
            name: &str,
            picture: Vec<u8>,
        ) -> Result<CustomEmoji, MessageError> {
            self.state
                .guilds
                .create_emoji(
                    &self.state.profile_images,
                    user_id,
                    self.guild.id,
                    name,
                    picture,
                    NOW,
                )
                .await
        }

        fn general(&self) -> Snowflake {
            self.guild.channels[1].id
        }

        async fn message_in(&self, channel_id: Snowflake) -> Snowflake {
            let request = SendMessageRequest {
                content: "hi".into(),
                reply_to_id: None,
                nonce: None,
                attachment_ids: Vec::new(),
            };
            self.state
                .messages
                .send(self.owner, channel_id, request, NOW)
                .await
                .unwrap()
                .id
        }

        async fn react(
            &self,
            channel_id: Snowflake,
            message_id: Snowflake,
            key: &str,
        ) -> Result<(), MessageError> {
            self.state
                .messages
                .add_reaction(self.owner, channel_id, message_id, key)
                .await
        }

        /// The key and count of each reaction on a message of `general`.
        async fn reactions(&self, message_id: Snowflake) -> Vec<(String, u32)> {
            let messages = self
                .state
                .messages
                .list_messages(self.owner, self.general(), MessageHistoryQuery::default())
                .await
                .unwrap();
            messages
                .into_iter()
                .filter(|message| message.id == message_id)
                .flat_map(|message| message.reactions)
                .map(|reaction| (reaction.emoji, reaction.count))
                .collect()
        }
    }

    fn is_rule_violation(
        result: &Result<CustomEmoji, MessageError>,
        expected: &GuildRuleViolation,
    ) -> bool {
        matches!(result, Err(MessageError::Guild(GuildError::InvalidInput(violation))) if violation == expected)
    }

    #[tokio::test]
    async fn only_members_with_manage_emojis_add_emoji_with_valid_names_unique_whatever_the_case() {
        let (_directory, database) = migrated_database().await;
        let club = club(test_state(database.clone(), RegistrationMode::Open)).await;
        let by_member = club.create(club.member, "party").await;
        let by_outsider = club.create(club.outsider, "party").await;
        let created = club.create(club.owner, " party ").await.unwrap();
        let same_name = club.create(club.owner, "PARTY").await;
        let bad_name = club.create(club.owner, "a").await;
        let not_a_picture = club
            .create_from(club.owner, "text", b"hello".to_vec())
            .await;
        // A role holding MANAGE_EXPRESSIONS is enough; it is a guild permission.
        let role = club
            .state
            .guilds
            .create_role(
                club.owner,
                club.guild.id,
                CreateRoleRequest {
                    name: "Emoji keepers".into(),
                    color: 0,
                    permissions: Permissions::MANAGE_EXPRESSIONS,
                },
                NOW,
            )
            .await
            .unwrap();
        club.state
            .guilds
            .add_member_role(club.owner, club.guild.id, club.member, role.id)
            .await
            .unwrap();
        let animated = club
            .create_from(club.member, "parrot", animated_gif(60, 40, 3))
            .await
            .unwrap();
        let listed = club
            .state
            .guilds
            .list_emojis(club.member, club.guild.id)
            .await
            .unwrap();
        let listed_to_outsider = club
            .state
            .guilds
            .list_emojis(club.outsider, club.guild.id)
            .await;
        let guild = club
            .state
            .guilds
            .get_guild(club.member, club.guild.id)
            .await
            .unwrap();
        let picture = club
            .state
            .guilds
            .readable_icon_file_id(club.member, animated.id, true)
            .await;
        let picture_to_outsider = club
            .state
            .guilds
            .readable_icon_file_id(club.outsider, animated.id, false)
            .await;
        database.close().await;

        assert!(matches!(
            by_member,
            Err(MessageError::Guild(GuildError::MissingPermissions))
        ));
        assert!(matches!(
            by_outsider,
            Err(MessageError::Guild(GuildError::NotFound))
        ));
        assert_eq!(created.name, "party");
        assert!(!created.is_animated);
        assert!(is_rule_violation(
            &same_name,
            &GuildRuleViolation::EmojiNameTaken
        ));
        assert!(is_rule_violation(&bad_name, &GuildRuleViolation::EmojiName));
        assert!(matches!(
            not_a_picture,
            Err(MessageError::InvalidInput(MessageRuleViolation::Picture))
        ));
        assert!(animated.is_animated);
        assert_eq!(listed, [created, animated.clone()]);
        assert_eq!(guild.emojis, listed);
        assert!(
            matches!(picture, Ok(still) if still != animated.id),
            "a still frame"
        );
        assert!(matches!(picture_to_outsider, Err(GuildError::NotFound)));
        assert!(matches!(listed_to_outsider, Err(GuildError::NotFound)));
    }

    #[tokio::test]
    async fn a_guild_holds_50_still_and_50_animated_emoji() {
        let (_directory, database) = migrated_database().await;
        let club = club(test_state(database.clone(), RegistrationMode::Open)).await;
        let mut next_id = 1_000;
        let mut insert = |is_animated: bool, name: String| {
            next_id += 2;
            let image = NewProfileImage {
                id: Snowflake::from_raw(next_id),
                still_id: is_animated.then(|| Snowflake::from_raw(next_id + 1)),
                size_bytes: 1,
            };
            let audit = club
                .state
                .guilds
                .audit_entry(
                    club.guild.id,
                    club.owner,
                    AuditAction::EmojiCreate,
                    Some(image.id),
                    None,
                )
                .unwrap();
            let database = database.clone();
            let guild_id = club.guild.id;
            async move {
                storage::insert_guild_emoji(
                    &database,
                    guild_id,
                    &name,
                    image,
                    MAXIMUM_EMOJIS_PER_GUILD_AND_KIND,
                    &audit,
                    NOW,
                )
                .await
                .unwrap()
            }
        };
        for index in 0..MAXIMUM_EMOJIS_PER_GUILD_AND_KIND {
            let outcome = insert(false, format!("still_{index}")).await;
            assert!(matches!(outcome, EmojiWriteOutcome::Written(_)));
        }
        let one_still_too_many = insert(false, "still_more".into()).await;
        let first_animated = insert(true, "animated".into()).await;
        // Refused before processing: a PNG can only be still.
        let still_upload = club.create(club.owner, "late").await;
        let animated_upload = club
            .create_from(club.owner, "parrot", animated_gif(60, 40, 3))
            .await;
        database.close().await;

        assert_eq!(one_still_too_many, EmojiWriteOutcome::LimitReached);
        assert!(
            matches!(first_animated, EmojiWriteOutcome::Written(emojis) if emojis.len() == 51),
            "animated emoji have their own quota"
        );
        assert!(matches!(
            still_upload,
            Err(MessageError::Guild(GuildError::LimitReached(_)))
        ));
        assert!(animated_upload.is_ok_and(|emoji| emoji.is_animated));
    }

    #[test]
    fn an_emoji_stored_over_512_kb_still_frame_included_is_refused() {
        use crate::domain::image_processing::ProcessedImage;

        let processed = |picture_bytes: usize, still_bytes: Option<usize>| ProcessedProfileImage {
            picture: ProcessedImage {
                bytes: vec![0; picture_bytes],
                content_type: "image/webp",
                is_animated: still_bytes.is_some(),
            },
            still: still_bytes.map(|bytes| vec![0; bytes]),
        };
        let half = MAXIMUM_EMOJI_STORED_BYTES / 2;
        assert!(within_emoji_stored_size(processed(MAXIMUM_EMOJI_STORED_BYTES, None)).is_ok());
        assert!(within_emoji_stored_size(processed(half, Some(half))).is_ok());
        assert_eq!(
            within_emoji_stored_size(processed(MAXIMUM_EMOJI_STORED_BYTES + 1, None)),
            Err(ImageProcessingError::TooLarge)
        );
        assert_eq!(
            within_emoji_stored_size(processed(half, Some(half + 1))),
            Err(ImageProcessingError::TooLarge)
        );
    }

    #[tokio::test]
    async fn renaming_needs_manage_emojis_keeps_the_picture_and_refuses_taken_names() {
        let (_directory, database) = migrated_database().await;
        let club = club(test_state(database.clone(), RegistrationMode::Open)).await;
        let party = club.create(club.owner, "party").await.unwrap();
        let cake = club.create(club.owner, "cake").await.unwrap();
        let other_guild = guild_of(&club.state, club.owner, "Other").await;
        let guilds = &club.state.guilds;
        let by_member = guilds
            .rename_emoji(club.member, club.guild.id, party.id, "fiesta")
            .await;
        let taken = guilds
            .rename_emoji(club.owner, club.guild.id, party.id, "Cake")
            .await;
        let invalid = guilds
            .rename_emoji(club.owner, club.guild.id, party.id, "no way")
            .await;
        let through_other_guild = guilds
            .rename_emoji(club.owner, other_guild.id, party.id, "fiesta")
            .await;
        let unchanged = guilds
            .rename_emoji(club.owner, club.guild.id, party.id, "party")
            .await
            .unwrap();
        let recased = guilds
            .rename_emoji(club.owner, club.guild.id, party.id, "Party")
            .await
            .unwrap();
        let renamed = guilds
            .rename_emoji(club.owner, club.guild.id, party.id, "fiesta")
            .await
            .unwrap();
        let listed = guilds
            .list_emojis(club.member, club.guild.id)
            .await
            .unwrap();
        let rename_entries = guilds
            .list_audit_log(club.owner, club.guild.id, AuditLogQuery::default())
            .await
            .unwrap()
            .into_iter()
            .filter(|entry| entry.action == AuditAction::EmojiUpdate)
            .count();
        database.close().await;

        assert!(matches!(by_member, Err(GuildError::MissingPermissions)));
        assert!(matches!(
            taken,
            Err(GuildError::InvalidInput(GuildRuleViolation::EmojiNameTaken))
        ));
        assert!(matches!(
            invalid,
            Err(GuildError::InvalidInput(GuildRuleViolation::EmojiName))
        ));
        assert!(matches!(through_other_guild, Err(GuildError::NotFound)));
        assert_eq!(unchanged, party);
        assert_eq!(recased.name, "Party");
        assert_eq!(
            renamed,
            CustomEmoji {
                name: "fiesta".into(),
                ..party
            }
        );
        assert_eq!(listed, [renamed, cake]);
        assert_eq!(rename_entries, 2, "an unchanged name records nothing");
    }

    #[tokio::test]
    async fn deleting_stops_serving_the_picture_announces_the_list_and_audits_every_change() {
        let (_directory, database) = migrated_database().await;
        let club = club(test_state(database.clone(), RegistrationMode::Open)).await;
        let party = club.create(club.owner, "party").await.unwrap();
        let (key, mut events) = club
            .state
            .events
            .register_session(club.member, [7; 32], 4)
            .unwrap();
        club.state.events.activate_session(key, &[club.guild.id]);
        while events.try_recv().is_ok() {}
        let guilds = &club.state.guilds;
        let images = &club.state.profile_images;
        let by_member = guilds
            .delete_emoji(images, club.member, club.guild.id, party.id)
            .await;
        guilds
            .delete_emoji(images, club.owner, club.guild.id, party.id)
            .await
            .unwrap();
        let again = guilds
            .delete_emoji(images, club.owner, club.guild.id, party.id)
            .await;
        let listed = guilds
            .list_emojis(club.member, club.guild.id)
            .await
            .unwrap();
        let picture = guilds
            .readable_icon_file_id(club.owner, party.id, false)
            .await;
        let mut announced = Vec::new();
        while let Ok(event) = events.try_recv() {
            announced.push(serde_json::from_str::<GatewayEvent>(&event.payload_json).unwrap());
        }
        let actions: Vec<_> = guilds
            .list_audit_log(club.owner, club.guild.id, AuditLogQuery::default())
            .await
            .unwrap()
            .into_iter()
            .filter(|entry| entry.target_id == Some(party.id))
            .map(|entry| {
                (
                    entry.action,
                    entry.changes[0].old_value.clone(),
                    entry.changes[0].new_value.clone(),
                )
            })
            .collect();
        database.close().await;

        assert!(matches!(
            by_member,
            Err(MessageError::Guild(GuildError::MissingPermissions))
        ));
        assert!(matches!(
            again,
            Err(MessageError::Guild(GuildError::NotFound))
        ));
        assert_eq!(listed, []);
        assert!(matches!(picture, Err(GuildError::NotFound)));
        assert_eq!(
            announced,
            [GatewayEvent::GuildEmojisUpdate {
                guild_id: club.guild.id,
                emojis: Vec::new()
            }]
        );
        assert_eq!(
            actions,
            [
                (AuditAction::EmojiDelete, "party".to_owned(), String::new()),
                (AuditAction::EmojiCreate, String::new(), "party".to_owned()),
            ]
        );
    }

    #[tokio::test]
    #[allow(clippy::too_many_lines)] // One scenario: start, rename, join, delete, take back.
    async fn custom_emoji_start_reactions_only_in_their_guild_and_are_keyed_by_id() {
        use mellow_protocol::{OverwriteKind, SetPermissionOverwriteRequest};

        let (_directory, database) = migrated_database().await;
        let club = club(test_state(database.clone(), RegistrationMode::Open)).await;
        let party = club.create(club.owner, "party").await.unwrap();
        let party_key = format!(":{}", party.id);
        let general = club.general();
        let message = club.message_in(general).await;
        let other_message = club.message_in(general).await;
        let other_guild = guild_of(&club.state, club.owner, "Other").await;
        let elsewhere = other_guild.channels[1].id;
        let message_elsewhere = club.message_in(elsewhere).await;
        let conversation = club
            .state
            .messages
            .open_direct_message(
                club.owner,
                OpenDirectMessageRequest {
                    recipient_id: club.member,
                },
                NOW,
            )
            .await
            .unwrap();
        let message_in_conversation = club.message_in(conversation.id).await;
        // Members may join reactions here, not start them.
        club.state
            .guilds
            .set_permission_overwrite(
                club.owner,
                general,
                club.guild.id,
                SetPermissionOverwriteRequest {
                    kind: OverwriteKind::Role,
                    allow: Permissions::empty(),
                    deny: Permissions::ADD_REACTIONS,
                },
            )
            .await
            .unwrap();

        club.react(general, message, &party_key).await.unwrap();
        let in_other_guild = club.react(elsewhere, message_elsewhere, &party_key).await;
        let in_conversation = club
            .react(conversation.id, message_in_conversation, &party_key)
            .await;
        let unknown = club.react(general, message, ":42").await;
        let member_starting = club
            .state
            .messages
            .add_reaction(club.member, general, other_message, &party_key)
            .await;
        club.state
            .guilds
            .rename_emoji(club.owner, club.guild.id, party.id, "fiesta")
            .await
            .unwrap();
        club.state
            .messages
            .add_reaction(club.member, general, message, &party_key)
            .await
            .unwrap();
        let after_rename = club.reactions(message).await;
        club.state
            .guilds
            .delete_emoji(
                &club.state.profile_images,
                club.owner,
                club.guild.id,
                party.id,
            )
            .await
            .unwrap();
        let after_deletion = club.reactions(message).await;
        let started_after_deletion = club.react(general, other_message, &party_key).await;
        club.state
            .messages
            .remove_own_reaction(club.member, general, message, &party_key)
            .await
            .unwrap();
        club.state
            .messages
            .add_reaction(club.member, general, message, &party_key)
            .await
            .unwrap();
        let after_taking_back = club.reactions(message).await;
        database.close().await;

        let refused = |result: &Result<(), MessageError>| {
            matches!(
                result,
                Err(MessageError::InvalidInput(
                    MessageRuleViolation::CustomReactionEmoji
                ))
            )
        };
        assert!(refused(&in_other_guild), "another guild's channel");
        assert!(refused(&in_conversation), "a conversation");
        assert!(refused(&unknown));
        assert!(matches!(
            member_starting,
            Err(MessageError::Guild(GuildError::MissingPermissions))
        ));
        assert_eq!(
            after_rename,
            [(party_key.clone(), 2)],
            "one reaction across the rename, joined without ADD_REACTIONS"
        );
        assert_eq!(after_deletion, [(party_key.clone(), 2)], "reactions stay");
        assert!(
            refused(&started_after_deletion),
            "a deleted emoji starts no new reaction"
        );
        assert_eq!(
            after_taking_back,
            [(party_key, 2)],
            "but it can be taken back and joined again"
        );
    }
}
