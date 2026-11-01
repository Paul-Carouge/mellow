//! Soundboard sounds and join sounds (ADR-0032): short canonical WAV files a guild names, played
//! by each client of a voice channel. Their files share the profile images' sealed store.

use mellow_protocol::{AuditAction, GatewayEvent, Permissions, Snowflake, SoundboardSound};

use super::{GuildError, Guilds};
use crate::domain::guild_rules::GuildRuleViolation;
use crate::domain::message_rules::custom_emoji_of_reaction_key;
use crate::domain::messages::MessageError;
use crate::domain::profile_images::ProfileImages;
use crate::domain::sound_rules::{
    MAXIMUM_SOUNDS_PER_GUILD, canonical_wav_duration_milliseconds, normalize_sound_emoji,
    normalize_sound_name,
};
use crate::storage::guild_sounds::{
    self as storage, JoinSoundOutcome, NewGuildSound, SoundDeleteOutcome, SoundWriteOutcome,
};

const SOUND_LIMIT_MESSAGE: &str = "a server holds at most 48 sounds";

impl Guilds {
    /// The guild's sounds, for its members only (`NotFound` otherwise).
    pub(crate) async fn list_sounds(
        &self,
        user_id: Snowflake,
        guild_id: Snowflake,
    ) -> Result<Vec<SoundboardSound>, GuildError> {
        self.membership_and_permissions(guild_id, user_id).await?;
        Ok(storage::list_guild_sounds(&self.database, guild_id).await?)
    }

    /// Needs `MANAGE_EXPRESSIONS`. `file` must be a canonical WAV (ADR-0032). Every member sees
    /// the guild's new list (`GUILD_SOUNDS_UPDATE`).
    #[allow(clippy::too_many_arguments)] // shortcut: mirrors the request; bundle if it grows.
    pub(crate) async fn create_sound(
        &self,
        files: &ProfileImages,
        user_id: Snowflake,
        guild_id: Snowflake,
        name: &str,
        emoji: Option<&str>,
        file: Vec<u8>,
        now_unix_milliseconds: u64,
    ) -> Result<SoundboardSound, MessageError> {
        let name = normalize_sound_name(name).map_err(GuildError::from)?;
        let emoji = normalize_sound_emoji(emoji).map_err(GuildError::from)?;
        let duration_milliseconds =
            canonical_wav_duration_milliseconds(&file).map_err(GuildError::from)?;
        self.require_member_permissions(guild_id, user_id, Permissions::MANAGE_EXPRESSIONS)
            .await?;
        // Cheap refusal before writing a file; the insert's own count stays the one that decides.
        let count = storage::count_guild_sounds(&self.database, guild_id)
            .await
            .map_err(GuildError::from)?;
        if count >= MAXIMUM_SOUNDS_PER_GUILD {
            return Err(GuildError::LimitReached(SOUND_LIMIT_MESSAGE).into());
        }
        let size_bytes = u64::try_from(file.len()).unwrap_or(u64::MAX);
        let id = files.store_file(file, now_unix_milliseconds).await?;
        let custom_emoji_id = emoji.as_deref().and_then(custom_emoji_of_reaction_key);
        let sound = NewGuildSound {
            id,
            name,
            emoji,
            custom_emoji_id,
            duration_milliseconds,
            size_bytes,
        };
        let outcome = async {
            let audit =
                self.audit_entry(guild_id, user_id, AuditAction::SoundCreate, Some(id), None)?;
            let outcome = storage::insert_guild_sound(
                &self.database,
                guild_id,
                &sound,
                MAXIMUM_SOUNDS_PER_GUILD,
                &audit,
                now_unix_milliseconds,
            )
            .await?;
            sounds_written(outcome)
        }
        .await;
        let sounds = match outcome {
            Ok(sounds) => sounds,
            Err(error) => {
                files.delete_files(id, None).await;
                return Err(error.into());
            }
        };
        let created = sounds.iter().find(|sound| sound.id == id).cloned();
        self.publish_sounds(guild_id, sounds);
        created.ok_or_else(|| GuildError::Internal("a created sound was not listed".into()).into())
    }

    /// Needs `MANAGE_EXPRESSIONS`. Replaces the sound's name and emoji.
    pub(crate) async fn update_sound(
        &self,
        user_id: Snowflake,
        guild_id: Snowflake,
        sound_id: Snowflake,
        name: &str,
        emoji: Option<&str>,
    ) -> Result<SoundboardSound, GuildError> {
        let name = normalize_sound_name(name)?;
        let emoji = normalize_sound_emoji(emoji)?;
        self.require_member_permissions(guild_id, user_id, Permissions::MANAGE_EXPRESSIONS)
            .await?;
        let audit = self.audit_entry(
            guild_id,
            user_id,
            AuditAction::SoundUpdate,
            Some(sound_id),
            None,
        )?;
        let outcome = storage::update_guild_sound(
            &self.database,
            guild_id,
            sound_id,
            &name,
            emoji.as_deref(),
            emoji.as_deref().and_then(custom_emoji_of_reaction_key),
            &audit,
        )
        .await?;
        if let SoundWriteOutcome::Unchanged(sound) = outcome {
            return Ok(sound);
        }
        let sounds = sounds_written(outcome)?;
        let updated = sounds.iter().find(|sound| sound.id == sound_id).cloned();
        self.publish_sounds(guild_id, sounds);
        updated.ok_or(GuildError::NotFound)
    }

    /// Needs `MANAGE_EXPRESSIONS`. Members who had it as join sound have none any more.
    pub(crate) async fn delete_sound(
        &self,
        files: &ProfileImages,
        user_id: Snowflake,
        guild_id: Snowflake,
        sound_id: Snowflake,
    ) -> Result<(), GuildError> {
        self.require_member_permissions(guild_id, user_id, Permissions::MANAGE_EXPRESSIONS)
            .await?;
        let audit = self.audit_entry(
            guild_id,
            user_id,
            AuditAction::SoundDelete,
            Some(sound_id),
            None,
        )?;
        let outcome =
            storage::delete_guild_sound(&self.database, guild_id, sound_id, &audit).await?;
        let SoundDeleteOutcome::Deleted {
            sounds,
            members_without_join_sound,
        } = outcome
        else {
            return Err(GuildError::NotFound);
        };
        files.delete_files(sound_id, None).await;
        self.publish_sounds(guild_id, sounds);
        for member_id in members_without_join_sound {
            self.events.publish_to_user(
                member_id,
                &GatewayEvent::MyJoinSoundUpdate {
                    guild_id,
                    sound_id: None,
                },
            );
        }
        Ok(())
    }

    /// Chooses one of the guild's sounds as the member's join sound there, or clears it
    /// (`None`). Choosing needs `USE_SOUNDBOARD`; clearing needs nothing. The member's sessions
    /// learn it from `MY_JOIN_SOUND_UPDATE`.
    pub(crate) async fn set_join_sound(
        &self,
        user_id: Snowflake,
        guild_id: Snowflake,
        sound_id: Option<Snowflake>,
    ) -> Result<(), GuildError> {
        let (_, permissions) = self.membership_and_permissions(guild_id, user_id).await?;
        if sound_id.is_some() && !permissions.contains(Permissions::USE_SOUNDBOARD) {
            return Err(GuildError::MissingPermissions);
        }
        match storage::set_member_join_sound(&self.database, guild_id, user_id, sound_id).await? {
            JoinSoundOutcome::NotFound => Err(GuildError::NotFound),
            JoinSoundOutcome::Unchanged => Ok(()),
            JoinSoundOutcome::Changed => {
                self.events.publish_to_user(
                    user_id,
                    &GatewayEvent::MyJoinSoundUpdate { guild_id, sound_id },
                );
                Ok(())
            }
        }
    }

    /// The sound's file id (its own), if `user_id` is a member of its guild.
    pub(crate) async fn readable_sound_file_id(
        &self,
        user_id: Snowflake,
        sound_id: Snowflake,
    ) -> Result<Snowflake, GuildError> {
        let stored = storage::find_guild_sound(&self.database, sound_id)
            .await?
            .ok_or(GuildError::NotFound)?;
        self.membership_and_permissions(stored.guild_id, user_id)
            .await?;
        Ok(stored.sound.id)
    }

    /// The member's join sound in the guild, if they chose one.
    pub(crate) async fn join_sound_of_member(
        &self,
        guild_id: Snowflake,
        user_id: Snowflake,
    ) -> Result<Option<Snowflake>, GuildError> {
        Ok(storage::member_join_sound(&self.database, guild_id, user_id).await?)
    }

    /// The guild a sound belongs to, or `None` once it is gone.
    pub(crate) async fn guild_of_sound(
        &self,
        sound_id: Snowflake,
    ) -> Result<Option<Snowflake>, GuildError> {
        Ok(storage::find_guild_sound(&self.database, sound_id)
            .await?
            .map(|stored| stored.guild_id))
    }

    fn publish_sounds(&self, guild_id: Snowflake, sounds: Vec<SoundboardSound>) {
        self.events.publish_to_guild(
            guild_id,
            &GatewayEvent::GuildSoundsUpdate { guild_id, sounds },
        );
    }
}

/// The guild's sounds after a write, or why it was refused.
fn sounds_written(outcome: SoundWriteOutcome) -> Result<Vec<SoundboardSound>, GuildError> {
    match outcome {
        SoundWriteOutcome::Written(sounds) => Ok(sounds),
        SoundWriteOutcome::Unchanged(_) => Err(GuildError::Internal(
            "only an edit answers unchanged, and it handles it first".into(),
        )),
        SoundWriteOutcome::NotFound => Err(GuildError::NotFound),
        SoundWriteOutcome::EmojiNotFound => Err(GuildRuleViolation::SoundEmoji.into()),
        SoundWriteOutcome::LimitReached => Err(GuildError::LimitReached(SOUND_LIMIT_MESSAGE)),
    }
}

#[cfg(test)]
mod tests {
    use mellow_protocol::{
        AuditChange, AuditChangeKey, AuditLogQuery, CreateGuildRequest, CreateInviteRequest, Guild,
        UpdateRoleRequest,
    };

    use super::*;
    use crate::domain::account_rules::RegistrationMode;
    use crate::domain::accounts::tests::{NOW, migrated_database, register_request};
    use crate::domain::image_processing::tests::png;
    use crate::domain::sound_rules::tests::canonical_wav;
    use crate::http::AppState;
    use crate::http::tests::test_state;

    /// A guild of `owner` that `member` joined, another guild of the owner's, and `outsider`
    /// who belongs to none.
    struct Club {
        state: AppState,
        owner: Snowflake,
        member: Snowflake,
        outsider: Snowflake,
        guild: Guild,
        other_guild: Guild,
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
        let create = |name: &str| {
            state.guilds.create_guild(
                owner,
                false,
                CreateGuildRequest {
                    name: name.into(),
                    initial_categories: None,
                },
                NOW,
            )
        };
        let guild = create("Club").await.unwrap();
        let other_guild = create("Other").await.unwrap();
        for joined in [&guild, &other_guild] {
            let invite = state
                .guilds
                .create_invite(owner, joined.id, CreateInviteRequest::default(), NOW)
                .await
                .unwrap();
            state
                .guilds
                .join_with_invite(member, &invite.code, NOW)
                .await
                .unwrap();
        }
        Club {
            state,
            owner,
            member,
            outsider,
            guild,
            other_guild,
        }
    }

    impl Club {
        async fn create_in(
            &self,
            guild_id: Snowflake,
            user_id: Snowflake,
            name: &str,
            emoji: Option<&str>,
            file: Vec<u8>,
        ) -> Result<SoundboardSound, MessageError> {
            self.state
                .guilds
                .create_sound(
                    &self.state.profile_images,
                    user_id,
                    guild_id,
                    name,
                    emoji,
                    file,
                    NOW,
                )
                .await
        }

        async fn create(
            &self,
            user_id: Snowflake,
            name: &str,
        ) -> Result<SoundboardSound, MessageError> {
            self.create_in(self.guild.id, user_id, name, None, canonical_wav(4_800))
                .await
        }

        async fn update(
            &self,
            sound_id: Snowflake,
            name: &str,
            emoji: Option<&str>,
        ) -> Result<SoundboardSound, GuildError> {
            self.state
                .guilds
                .update_sound(self.owner, self.guild.id, sound_id, name, emoji)
                .await
        }

        async fn my_join_sound(&self, user_id: Snowflake) -> Option<Snowflake> {
            self.state
                .guilds
                .get_guild(user_id, self.guild.id)
                .await
                .unwrap()
                .my_join_sound_id
        }
    }

    fn is_violation<T: std::fmt::Debug>(
        result: &Result<T, MessageError>,
        expected: &GuildRuleViolation,
    ) -> bool {
        matches!(result, Err(MessageError::Guild(GuildError::InvalidInput(violation))) if violation == expected)
    }

    #[tokio::test]
    async fn only_managers_add_valid_sounds_that_members_alone_list_and_download() {
        let (_directory, database) = migrated_database().await;
        let club = club(test_state(database.clone(), RegistrationMode::Open)).await;
        let by_member = club.create(club.member, "Horn").await;
        let by_outsider = club.create(club.outsider, "Horn").await;
        let not_a_wav = club
            .create_in(club.guild.id, club.owner, "Horn", None, png(8, 8))
            .await;
        let bad_name = club.create(club.owner, " x ").await;
        let bad_emoji = club
            .create_in(
                club.guild.id,
                club.owner,
                "Horn",
                Some("horn"),
                canonical_wav(1),
            )
            .await;
        let created = club
            .create_in(
                club.guild.id,
                club.owner,
                " Air horn ",
                Some("📯"),
                canonical_wav(4_800),
            )
            .await
            .unwrap();
        let listed = club
            .state
            .guilds
            .list_sounds(club.member, club.guild.id)
            .await;
        let listed_to_outsider = club
            .state
            .guilds
            .list_sounds(club.outsider, club.guild.id)
            .await;
        let guild = club
            .state
            .guilds
            .get_guild(club.member, club.guild.id)
            .await
            .unwrap();
        let file_id = club
            .state
            .guilds
            .readable_sound_file_id(club.member, created.id)
            .await;
        let file = club
            .state
            .profile_images
            .read_file(created.id)
            .await
            .map(|(bytes, _)| bytes);
        let file_to_outsider = club
            .state
            .guilds
            .readable_sound_file_id(club.outsider, created.id)
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
        assert!(is_violation(&not_a_wav, &GuildRuleViolation::SoundFile));
        assert!(is_violation(&bad_name, &GuildRuleViolation::SoundName));
        assert!(is_violation(&bad_emoji, &GuildRuleViolation::SoundEmoji));
        assert_eq!(
            created,
            SoundboardSound {
                id: created.id,
                name: "Air horn".into(),
                emoji: Some("📯".into()),
                duration_milliseconds: 100,
            }
        );
        assert_eq!(listed.unwrap(), std::slice::from_ref(&created));
        assert_eq!(guild.sounds, std::slice::from_ref(&created));
        assert!(matches!(listed_to_outsider, Err(GuildError::NotFound)));
        assert_eq!(file_id.unwrap(), created.id);
        assert_eq!(file.unwrap(), canonical_wav(4_800));
        assert!(matches!(file_to_outsider, Err(GuildError::NotFound)));
    }

    #[tokio::test]
    #[allow(clippy::too_many_lines)] // One story: create, edit, delete, then the audit log.
    async fn edits_and_deletions_are_audited_and_custom_emoji_come_from_the_same_server() {
        let (_directory, database) = migrated_database().await;
        let club = club(test_state(database.clone(), RegistrationMode::Open)).await;
        let emoji_here = |guild_id| {
            club.state.guilds.create_emoji(
                &club.state.profile_images,
                club.owner,
                guild_id,
                "party",
                png(64, 64),
                NOW,
            )
        };
        let own_emoji = emoji_here(club.guild.id).await.unwrap();
        let foreign_emoji = emoji_here(club.other_guild.id).await.unwrap();
        let foreign_key = format!(":{}", foreign_emoji.id);
        let own_key = format!(":{}", own_emoji.id);
        let with_foreign = club
            .create_in(
                club.guild.id,
                club.owner,
                "Horn",
                Some(&foreign_key),
                canonical_wav(1),
            )
            .await;
        let sound = club.create(club.owner, "Horn").await.unwrap();
        let unchanged = club.update(sound.id, "Horn", None).await.unwrap();
        let foreign_update = club.update(sound.id, "Horn", Some(&foreign_key)).await;
        let updated = club
            .update(sound.id, "Big horn", Some(&own_key))
            .await
            .unwrap();
        let by_member = club
            .state
            .guilds
            .update_sound(club.member, club.guild.id, sound.id, "Mine", None)
            .await;
        let in_other_guild = club
            .state
            .guilds
            .update_sound(club.owner, club.other_guild.id, sound.id, "Moved", None)
            .await;
        let deleted = club
            .state
            .guilds
            .delete_sound(
                &club.state.profile_images,
                club.owner,
                club.guild.id,
                sound.id,
            )
            .await;
        let deleted_again = club
            .state
            .guilds
            .delete_sound(
                &club.state.profile_images,
                club.owner,
                club.guild.id,
                sound.id,
            )
            .await;
        let file_after = club.state.profile_images.read_file(sound.id).await;
        let audit: Vec<(AuditAction, Vec<AuditChange>)> = club
            .state
            .guilds
            .list_audit_log(club.owner, club.guild.id, AuditLogQuery::default())
            .await
            .unwrap()
            .into_iter()
            .filter(|entry| entry.target_id == Some(sound.id))
            .map(|entry| (entry.action, entry.changes))
            .collect();
        database.close().await;

        assert!(is_violation(&with_foreign, &GuildRuleViolation::SoundEmoji));
        assert_eq!(unchanged, sound);
        assert!(matches!(
            foreign_update,
            Err(GuildError::InvalidInput(GuildRuleViolation::SoundEmoji))
        ));
        assert_eq!(
            (updated.name.as_str(), updated.emoji.as_deref()),
            ("Big horn", Some(own_key.as_str()))
        );
        assert!(matches!(by_member, Err(GuildError::MissingPermissions)));
        assert!(matches!(in_other_guild, Err(GuildError::NotFound)));
        assert!(deleted.is_ok());
        assert!(matches!(deleted_again, Err(GuildError::NotFound)));
        assert!(file_after.is_err(), "the file goes with the sound");
        let change = |key, old: &str, new: &str| AuditChange {
            key,
            old_value: old.into(),
            new_value: new.into(),
        };
        assert_eq!(
            audit,
            [
                (
                    AuditAction::SoundDelete,
                    vec![change(AuditChangeKey::Name, "Big horn", "")]
                ),
                (
                    AuditAction::SoundUpdate,
                    vec![
                        change(AuditChangeKey::Name, "Horn", "Big horn"),
                        change(AuditChangeKey::Emoji, "", &own_key)
                    ]
                ),
                (
                    AuditAction::SoundCreate,
                    vec![change(AuditChangeKey::Name, "", "Horn")]
                ),
            ]
        );
    }

    #[tokio::test]
    async fn a_guild_holds_48_sounds() {
        let (_directory, database) = migrated_database().await;
        let club = club(test_state(database.clone(), RegistrationMode::Open)).await;
        for index in 0..u64::from(MAXIMUM_SOUNDS_PER_GUILD) {
            let audit = club
                .state
                .guilds
                .audit_entry(
                    club.guild.id,
                    club.owner,
                    AuditAction::SoundCreate,
                    None,
                    None,
                )
                .unwrap();
            let sound = NewGuildSound {
                id: Snowflake::from_raw(1_000 + index),
                name: format!("Sound {index}"),
                emoji: None,
                custom_emoji_id: None,
                duration_milliseconds: 1,
                size_bytes: 46,
            };
            let outcome = storage::insert_guild_sound(
                &database,
                club.guild.id,
                &sound,
                MAXIMUM_SOUNDS_PER_GUILD,
                &audit,
                NOW,
            )
            .await
            .unwrap();
            assert!(matches!(outcome, SoundWriteOutcome::Written(_)));
        }
        let one_too_many = club.create(club.owner, "Late").await;
        let in_other_guild = club
            .create_in(
                club.other_guild.id,
                club.owner,
                "Fine",
                None,
                canonical_wav(1),
            )
            .await;
        database.close().await;

        assert!(matches!(
            one_too_many,
            Err(MessageError::Guild(GuildError::LimitReached(_)))
        ));
        assert!(in_other_guild.is_ok(), "the limit is per server");
    }

    #[tokio::test]
    async fn members_choose_a_join_sound_of_their_server_which_goes_with_the_sound() {
        let (_directory, database) = migrated_database().await;
        let club = club(test_state(database.clone(), RegistrationMode::Open)).await;
        let sound = club.create(club.owner, "Horn").await.unwrap();
        let foreign = club
            .create_in(
                club.other_guild.id,
                club.owner,
                "Bell",
                None,
                canonical_wav(1),
            )
            .await
            .unwrap();
        let set = |user_id, sound_id| {
            club.state
                .guilds
                .set_join_sound(user_id, club.guild.id, sound_id)
        };
        let foreign_choice = set(club.member, Some(foreign.id)).await;
        let by_outsider = set(club.outsider, Some(sound.id)).await;
        set(club.member, Some(sound.id)).await.unwrap();
        let chosen = club.my_join_sound(club.member).await;
        let owner_has_none = club.my_join_sound(club.owner).await;
        club.state
            .guilds
            .delete_sound(
                &club.state.profile_images,
                club.owner,
                club.guild.id,
                sound.id,
            )
            .await
            .unwrap();
        let after_deletion = club.my_join_sound(club.member).await;
        let other = club.create(club.owner, "Gong").await.unwrap();
        // Without USE_SOUNDBOARD a member may clear their choice, not make one.
        club.state
            .guilds
            .update_role(
                club.owner,
                club.guild.id,
                club.guild.id,
                UpdateRoleRequest {
                    name: None,
                    color: None,
                    permissions: Some(Permissions::DEFAULT_EVERYONE - Permissions::USE_SOUNDBOARD),
                },
            )
            .await
            .unwrap();
        let without_permission = set(club.member, Some(other.id)).await;
        let cleared = set(club.member, None).await;
        database.close().await;

        assert!(matches!(foreign_choice, Err(GuildError::NotFound)));
        assert!(matches!(by_outsider, Err(GuildError::NotFound)));
        assert_eq!(chosen, Some(sound.id));
        assert_eq!(owner_has_none, None);
        assert_eq!(after_deletion, None);
        assert!(matches!(
            without_permission,
            Err(GuildError::MissingPermissions)
        ));
        assert!(cleared.is_ok());
    }
}
