//! Guilds, their channels, roles, members, custom emoji (ADR-0028) and sounds (ADR-0032).

use mellow_protocol::{
    Channel, CustomEmoji, Guild, GuildSummary, Member, Role, Snowflake, SoundboardSound,
    compute_guild_permissions,
};

use super::{ClientState, StateChange};

/// Only to adapt the interface: the server decides (ADR-0014).
fn recompute_my_permissions(guild: &mut Guild, current_user_id: Snowflake) {
    let member_roles = guild
        .roles
        .iter()
        .filter(|role| role.id == guild.id || guild.my_role_ids.contains(&role.id));
    guild.my_permissions = compute_guild_permissions(current_user_id, guild.owner_id, member_roles);
}

fn sort_members_by_username(members: &mut [Member]) {
    members.sort_by(|left, right| left.user.username.cmp(&right.user.username));
}

impl ClientState {
    fn guild_mut(&mut self, guild_id: Snowflake) -> Option<&mut Guild> {
        self.guilds.iter_mut().find(|guild| guild.id == guild_id)
    }

    /// A custom emoji of any of the user's guilds: the pictures this user may load, so the ones a
    /// message or reaction can show. Others read as their name.
    /// shortcut: a linear search over every guild's list; index it if profiles show it hot.
    #[must_use]
    pub fn custom_emoji(&self, emoji_id: Snowflake) -> Option<&CustomEmoji> {
        self.guilds
            .iter()
            .flat_map(|guild| &guild.emojis)
            .find(|emoji| emoji.id == emoji_id)
    }

    /// The guild's custom emoji, oldest first.
    #[must_use]
    pub fn emojis_of(&self, guild_id: Snowflake) -> &[CustomEmoji] {
        self.guild(guild_id)
            .map_or(&[], |guild| guild.emojis.as_slice())
    }

    /// The guild's sounds, oldest first.
    #[must_use]
    pub fn sounds_of(&self, guild_id: Snowflake) -> &[SoundboardSound] {
        self.guild(guild_id)
            .map_or(&[], |guild| guild.sounds.as_slice())
    }

    pub fn set_members(&mut self, guild_id: Snowflake, mut members: Vec<Member>) -> StateChange {
        sort_members_by_username(&mut members);
        self.members_by_guild.insert(guild_id, members);
        StateChange::Members { guild_id }
    }

    /// Only a loaded member list changes.
    pub(super) fn add_member(&mut self, guild_id: Snowflake, member: Member) -> StateChange {
        if let Some(members) = self.members_by_guild.get_mut(&guild_id)
            && !members
                .iter()
                .any(|existing| existing.user.id == member.user.id)
        {
            members.push(member);
            sort_members_by_username(members);
        }
        StateChange::Members { guild_id }
    }

    pub(super) fn remove_member(&mut self, guild_id: Snowflake, user_id: Snowflake) -> StateChange {
        if let Some(members) = self.members_by_guild.get_mut(&guild_id) {
            members.retain(|member| member.user.id != user_id);
        }
        StateChange::Members { guild_id }
    }

    pub(super) fn update_guild_summary(&mut self, summary: GuildSummary) -> StateChange {
        match self.guild_mut(summary.id) {
            Some(guild) => {
                guild.name = summary.name;
                guild.icon = summary.icon;
                StateChange::Guilds
            }
            None => StateChange::Nothing,
        }
    }

    pub(super) fn remove_channel(&mut self, guild_id: Snowflake, id: Snowflake) -> StateChange {
        if let Some(guild) = self.guild_mut(guild_id) {
            guild.channels.retain(|channel| channel.id != id);
            // Children of a deleted category become uncategorised, as on the server.
            for channel in &mut guild.channels {
                if channel.parent_id == Some(id) {
                    channel.parent_id = None;
                }
            }
        }
        self.forget_channel(id);
        StateChange::Guilds
    }

    /// A fresh snapshot also follows permission changes (ADR-0015): channels it no longer lists
    /// were hidden from us, so everything kept about them goes too.
    pub(super) fn replace_guild(&mut self, guild: Guild) -> StateChange {
        let Some(existing) = self.guild_mut(guild.id) else {
            self.guilds.push(guild);
            return StateChange::Guilds;
        };
        let hidden_channel_ids: Vec<Snowflake> = existing
            .channels
            .iter()
            .map(|channel| channel.id)
            .filter(|id| !guild.channels.iter().any(|channel| channel.id == *id))
            .collect();
        *existing = guild;
        for channel_id in hidden_channel_ids {
            self.forget_channel(channel_id);
        }
        StateChange::Guilds
    }

    pub(super) fn remove_guild(&mut self, id: Snowflake) -> StateChange {
        let channel_ids: Vec<Snowflake> = self
            .guild(id)
            .map(|guild| guild.channels.iter().map(|channel| channel.id).collect())
            .unwrap_or_default();
        for channel_id in channel_ids {
            self.forget_channel(channel_id);
        }
        self.guilds.retain(|guild| guild.id != id);
        self.members_by_guild.remove(&id);
        StateChange::Guilds
    }

    /// A deleted role leaves its members here too: the server sends no `GUILD_MEMBER_UPDATE`.
    pub(super) fn replace_roles(&mut self, guild_id: Snowflake, roles: Vec<Role>) -> StateChange {
        let current_user_id = self.current_user_id();
        let Some(guild) = self.guilds.iter_mut().find(|guild| guild.id == guild_id) else {
            return StateChange::Nothing;
        };
        guild.roles = roles;
        let is_known_role =
            |role_id: &Snowflake| guild.roles.iter().any(|role| role.id == *role_id);
        guild.my_role_ids.retain(is_known_role);
        if let Some(members) = self.members_by_guild.get_mut(&guild_id) {
            for member in members {
                member.role_ids.retain(is_known_role);
            }
        }
        if let Some(current_user_id) = current_user_id {
            recompute_my_permissions(guild, current_user_id);
        }
        StateChange::Guilds
    }

    pub(super) fn update_member_roles(
        &mut self,
        guild_id: Snowflake,
        user_id: Snowflake,
        role_ids: Vec<Snowflake>,
    ) -> StateChange {
        if let Some(member) = self
            .members_by_guild
            .get_mut(&guild_id)
            .and_then(|members| members.iter_mut().find(|member| member.user.id == user_id))
        {
            member.role_ids.clone_from(&role_ids);
        }
        if self.current_user_id() == Some(user_id)
            && let Some(guild) = self.guild_mut(guild_id)
        {
            guild.my_role_ids = role_ids;
            recompute_my_permissions(guild, user_id);
            return StateChange::Guilds;
        }
        StateChange::Members { guild_id }
    }

    pub(super) fn upsert_channel(&mut self, channel: Channel) {
        let Some(guild) = self.guild_mut(channel.guild_id) else {
            return;
        };
        match guild
            .channels
            .iter_mut()
            .find(|existing| existing.id == channel.id)
        {
            Some(existing) => *existing = channel,
            None => guild.channels.push(channel),
        }
        guild
            .channels
            .sort_by_key(|existing| (existing.position, existing.id));
    }

    /// `GUILD_EMOJIS_UPDATE`: the whole list.
    pub(super) fn replace_emojis(
        &mut self,
        guild_id: Snowflake,
        emojis: Vec<CustomEmoji>,
    ) -> StateChange {
        match self.guild_mut(guild_id) {
            Some(guild) => {
                guild.emojis = emojis;
                StateChange::GuildEmojis { guild_id }
            }
            None => StateChange::Nothing,
        }
    }

    /// The answer to creating an emoji; the gateway's list may have brought it first.
    pub fn add_created_emoji(&mut self, guild_id: Snowflake, created: CustomEmoji) -> StateChange {
        let Some(guild) = self.guild_mut(guild_id) else {
            return StateChange::Nothing;
        };
        if guild.emojis.iter().any(|emoji| emoji.id == created.id) {
            return StateChange::Nothing;
        }
        guild.emojis.push(created);
        StateChange::GuildEmojis { guild_id }
    }

    /// The answer to deleting an emoji. A rename shows with `GUILD_EMOJIS_UPDATE` only: an answer
    /// arriving after a later rename's event would put the older name back.
    pub fn remove_deleted_emoji(
        &mut self,
        guild_id: Snowflake,
        emoji_id: Snowflake,
    ) -> StateChange {
        let Some(guild) = self.guild_mut(guild_id) else {
            return StateChange::Nothing;
        };
        guild.emojis.retain(|emoji| emoji.id != emoji_id);
        StateChange::GuildEmojis { guild_id }
    }

    /// `GUILD_SOUNDS_UPDATE`: the whole list.
    pub(super) fn replace_sounds(
        &mut self,
        guild_id: Snowflake,
        sounds: Vec<SoundboardSound>,
    ) -> StateChange {
        match self.guild_mut(guild_id) {
            Some(guild) => {
                guild.sounds = sounds;
                StateChange::GuildSounds { guild_id }
            }
            None => StateChange::Nothing,
        }
    }

    /// The answer to creating a sound; the gateway's list may have brought it first.
    pub fn add_created_sound(
        &mut self,
        guild_id: Snowflake,
        created: SoundboardSound,
    ) -> StateChange {
        let Some(guild) = self.guild_mut(guild_id) else {
            return StateChange::Nothing;
        };
        if guild.sounds.iter().any(|sound| sound.id == created.id) {
            return StateChange::Nothing;
        }
        guild.sounds.push(created);
        StateChange::GuildSounds { guild_id }
    }

    /// The answer to editing a sound, from a dialog that closes on success.
    pub fn replace_updated_sound(
        &mut self,
        guild_id: Snowflake,
        updated: SoundboardSound,
    ) -> StateChange {
        let Some(sound) = self
            .guild_mut(guild_id)
            .and_then(|guild| guild.sounds.iter_mut().find(|sound| sound.id == updated.id))
        else {
            return StateChange::Nothing;
        };
        *sound = updated;
        StateChange::GuildSounds { guild_id }
    }

    /// The answer to deleting a sound. The instance clears join sounds pointing at it; its event
    /// may come later.
    pub fn remove_deleted_sound(
        &mut self,
        guild_id: Snowflake,
        sound_id: Snowflake,
    ) -> StateChange {
        let Some(guild) = self.guild_mut(guild_id) else {
            return StateChange::Nothing;
        };
        guild.sounds.retain(|sound| sound.id != sound_id);
        if guild.my_join_sound_id == Some(sound_id) {
            guild.my_join_sound_id = None;
        }
        StateChange::GuildSounds { guild_id }
    }

    /// `MY_JOIN_SOUND_UPDATE`, or the answer to choosing one (`None` removes it).
    pub fn set_my_join_sound(
        &mut self,
        guild_id: Snowflake,
        sound_id: Option<Snowflake>,
    ) -> StateChange {
        match self.guild_mut(guild_id) {
            Some(guild) => {
                guild.my_join_sound_id = sound_id;
                StateChange::GuildSounds { guild_id }
            }
            None => StateChange::Nothing,
        }
    }
}

#[cfg(test)]
mod tests {
    use mellow_protocol::GatewayEvent;

    use super::super::tests::{GUILD, NOW, ready_state};
    use super::*;

    const CLUB_OTHER: Snowflake = Snowflake::from_raw(999);

    fn emoji(id: u64, name: &str) -> CustomEmoji {
        CustomEmoji {
            id: Snowflake::from_raw(id),
            name: name.into(),
            is_animated: false,
        }
    }

    fn sound(id: u64) -> SoundboardSound {
        SoundboardSound {
            id: Snowflake::from_raw(id),
            name: format!("sound{id}"),
            emoji: None,
            duration_milliseconds: 800,
        }
    }

    #[test]
    fn knows_every_guilds_emoji_and_follows_their_lists() {
        let mut state = ready_state();
        let wave = emoji(700, "wave");
        let change = state.apply_event(
            GatewayEvent::GuildEmojisUpdate {
                guild_id: GUILD,
                emojis: vec![wave.clone()],
            },
            NOW,
        );
        assert_eq!(change, StateChange::GuildEmojis { guild_id: GUILD });
        assert_eq!(state.custom_emoji(wave.id), Some(&wave));
        assert_eq!(state.emojis_of(CLUB_OTHER), []);
    }

    #[test]
    fn adds_a_created_emoji_once_even_after_the_gateways_list_and_drops_a_deleted_one() {
        let mut state = ready_state();
        let tada = emoji(702, "tada");
        state.add_created_emoji(GUILD, tada.clone());
        assert_eq!(
            state.add_created_emoji(GUILD, tada.clone()),
            StateChange::Nothing
        );
        assert_eq!(state.emojis_of(GUILD), std::slice::from_ref(&tada));
        state.remove_deleted_emoji(GUILD, tada.id);
        assert_eq!(state.custom_emoji(tada.id), None);
    }

    #[test]
    fn a_deleted_sound_also_clears_the_join_sound_that_played_it() {
        let mut state = ready_state();
        state.add_created_sound(GUILD, sound(900));
        state.add_created_sound(GUILD, sound(900));
        state.set_my_join_sound(GUILD, Some(Snowflake::from_raw(900)));
        let mut renamed = sound(900);
        renamed.name = "horn".into();
        state.replace_updated_sound(GUILD, renamed.clone());
        assert_eq!(state.sounds_of(GUILD), [renamed]);
        state.remove_deleted_sound(GUILD, Snowflake::from_raw(900));
        assert_eq!(state.sounds_of(GUILD), []);
        assert_eq!(state.guild(GUILD).unwrap().my_join_sound_id, None);
        state.apply_event(
            GatewayEvent::MyJoinSoundUpdate {
                guild_id: GUILD,
                sound_id: Some(Snowflake::from_raw(901)),
            },
            NOW,
        );
        assert_eq!(
            state.guild(GUILD).unwrap().my_join_sound_id,
            Some(Snowflake::from_raw(901))
        );
    }
}
