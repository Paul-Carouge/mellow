//! Where the reader is: Home (the conversations) or a server, and the channel or conversation open
//! there (the web's `selectGuild` and `selectChannel` in `client.svelte.ts`). The desktop also
//! remembers the channel last opened in each server and the place itself across launches
//! (`Preferences`), as Discord does. Pure: the shell applies it to the client state it is given.

use std::collections::HashMap;

use mellow_client_core::shared_logic::channel_order::channel_sections;
use mellow_client_core::state::ClientState;
use mellow_protocol::{ChannelKind, Guild, Snowflake};

use crate::preferences::{Preferences, RememberedPlace};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Place {
    /// The conversations; `None` until one is open (the Friends space comes with step 4).
    Home { conversation_id: Option<Snowflake> },
    /// A server and its open text channel; `None` when it has none the reader can open.
    Server {
        guild_id: Snowflake,
        channel_id: Option<Snowflake>,
    },
}

impl Place {
    #[must_use]
    pub fn guild_id(self) -> Option<Snowflake> {
        match self {
            Self::Home { .. } => None,
            Self::Server { guild_id, .. } => Some(guild_id),
        }
    }

    /// The open channel or conversation.
    #[must_use]
    pub fn channel_id(self) -> Option<Snowflake> {
        match self {
            Self::Home { conversation_id } => conversation_id,
            Self::Server { channel_id, .. } => channel_id,
        }
    }

    #[must_use]
    pub fn remembered(self) -> RememberedPlace {
        match self {
            Self::Home { conversation_id } => RememberedPlace::Home { conversation_id },
            Self::Server { guild_id, .. } => RememberedPlace::Server { guild_id },
        }
    }
}

/// Up or down a list (Alt+Up, Alt+Down and their variants).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    Previous,
    Next,
}

/// The text channels a server opens, in the sidebar's order; voice channels are joined, never
/// opened.
#[must_use]
pub fn openable_channel_ids(guild: &Guild) -> Vec<Snowflake> {
    channel_sections(&guild.channels)
        .into_iter()
        .flat_map(|section| section.channels)
        .filter(|channel| channel.kind == ChannelKind::Text)
        .map(|channel| channel.id)
        .collect()
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Navigation {
    /// `None` until the first `Ready` chose a place.
    place: Option<Place>,
    last_channel_by_guild: HashMap<Snowflake, Snowflake>,
}

impl Navigation {
    #[must_use]
    pub fn place(&self) -> Option<Place> {
        self.place
    }

    #[must_use]
    pub fn selected_guild_id(&self) -> Option<Snowflake> {
        self.place.and_then(Place::guild_id)
    }

    #[must_use]
    pub fn selected_channel_id(&self) -> Option<Snowflake> {
        self.place.and_then(Place::channel_id)
    }

    /// Opens a server (`None`: Home) at `preferred_channel_id` when it is there, else at the
    /// channel last opened in it, else at its first channel.
    pub fn select_guild(
        &mut self,
        state: &ClientState,
        guild_id: Option<Snowflake>,
        preferred_channel_id: Option<Snowflake>,
    ) {
        let place = match guild_id.and_then(|guild_id| state.guild(guild_id)) {
            Some(guild) => {
                let channel_ids = openable_channel_ids(guild);
                let remembered = self.last_channel_by_guild.get(&guild.id).copied();
                let channel_id = [preferred_channel_id, remembered]
                    .into_iter()
                    .flatten()
                    .find(|candidate| channel_ids.contains(candidate))
                    .or_else(|| channel_ids.first().copied());
                Place::Server {
                    guild_id: guild.id,
                    channel_id,
                }
            }
            None => Place::Home {
                conversation_id: preferred_channel_id
                    .filter(|id| state.direct_message_channel(*id).is_some()),
            },
        };
        self.go_to(place);
    }

    /// Opens a channel of the current server, or a conversation at Home.
    pub fn select_channel(&mut self, state: &ClientState, channel_id: Snowflake) {
        match state.guild_of_channel(channel_id) {
            Some(guild) => self.select_guild(state, Some(guild.id), Some(channel_id)),
            None => self.select_guild(state, None, Some(channel_id)),
        }
    }

    fn go_to(&mut self, place: Place) {
        if let Place::Server {
            guild_id,
            channel_id: Some(channel_id),
        } = place
        {
            self.last_channel_by_guild.insert(guild_id, channel_id);
        }
        self.place = Some(place);
    }

    /// After the state changed: the first `Ready` opens the remembered place (or the first server,
    /// as the web does); a server left, a channel deleted or hidden, or a conversation closed
    /// gives way to what remains. Returns whether the place changed.
    pub fn follow_state(&mut self, state: &ClientState, remembered: &Preferences) -> bool {
        if state.current_user().is_none() {
            return false;
        }
        let before = self.place;
        match self.place {
            None => {
                self.last_channel_by_guild
                    .extend(remembered.last_channel_by_guild.iter().copied());
                match remembered.last_place {
                    Some(RememberedPlace::Server { guild_id })
                        if state.guild(guild_id).is_some() =>
                    {
                        self.select_guild(state, Some(guild_id), None);
                    }
                    Some(RememberedPlace::Home { conversation_id }) => {
                        self.select_guild(state, None, conversation_id);
                    }
                    _ => self.select_first_server(state),
                }
            }
            Some(Place::Server {
                guild_id,
                channel_id,
            }) => match state.guild(guild_id) {
                None => self.select_first_server(state),
                Some(guild) => {
                    let channel_ids = openable_channel_ids(guild);
                    let is_open_channel_listed =
                        channel_id.map_or(channel_ids.is_empty(), |id| channel_ids.contains(&id));
                    if !is_open_channel_listed {
                        self.select_guild(state, Some(guild_id), channel_id);
                    }
                }
            },
            Some(Place::Home {
                conversation_id: Some(conversation_id),
            }) if state.direct_message_channel(conversation_id).is_none() => {
                self.go_to(Place::Home {
                    conversation_id: None,
                });
            }
            Some(Place::Home { .. }) => {}
        }
        self.place != before
    }

    fn select_first_server(&mut self, state: &ClientState) {
        let first = state.guilds().first().map(|guild| guild.id);
        self.select_guild(state, first, None);
    }

    /// The channel or conversation before or after the open one, round the end of the list
    /// (Alt+Up, Alt+Down); with `only_unread`, the nearest one with unread messages
    /// (Alt+Shift+Up, Alt+Shift+Down). `None` when there is nowhere else to go.
    #[must_use]
    pub fn adjacent_channel(
        &self,
        state: &ClientState,
        direction: Direction,
        only_unread: bool,
    ) -> Option<Snowflake> {
        let listed: Vec<Snowflake> = match self.place? {
            Place::Server { guild_id, .. } => openable_channel_ids(state.guild(guild_id)?),
            Place::Home { .. } => state
                .direct_message_channels()
                .iter()
                .map(|conversation| conversation.id)
                .collect(),
        };
        let current = self.selected_channel_id();
        let candidates = cyclic_order_after(&listed, current, direction);
        candidates
            .into_iter()
            .find(|id| Some(*id) != current && (!only_unread || state.has_unread(*id)))
    }

    /// The server before or after the open one, Home first, round the end of the rail
    /// (Ctrl+Alt+Up, Ctrl+Alt+Down). `Some(None)` is Home.
    #[must_use]
    pub fn adjacent_server(
        &self,
        state: &ClientState,
        direction: Direction,
    ) -> Option<Option<Snowflake>> {
        let rail: Vec<Option<Snowflake>> = std::iter::once(None)
            .chain(state.guilds().iter().map(|guild| Some(guild.id)))
            .collect();
        let current = self.place.map(Place::guild_id);
        let position = rail.iter().position(|entry| Some(*entry) == current);
        let next = match (position, direction) {
            (None, _) => 0,
            (Some(index), Direction::Next) => (index + 1) % rail.len(),
            (Some(index), Direction::Previous) => (index + rail.len() - 1) % rail.len(),
        };
        rail.get(next)
            .copied()
            .filter(|entry| Some(*entry) != current)
    }
}

/// `listed` starting after `current` in `direction`, wrapping round; from the first (or last)
/// item when `current` is not listed.
fn cyclic_order_after(
    listed: &[Snowflake],
    current: Option<Snowflake>,
    direction: Direction,
) -> Vec<Snowflake> {
    let ordered: Vec<Snowflake> = match direction {
        Direction::Next => listed.to_vec(),
        Direction::Previous => listed.iter().rev().copied().collect(),
    };
    match ordered.iter().position(|id| Some(*id) == current) {
        Some(index) => ordered[index + 1..]
            .iter()
            .chain(&ordered[..=index])
            .copied()
            .collect(),
        None => ordered,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shell_fixtures::{
        CONVERSATION_ANA, CONVERSATION_ZOE, GUILD_BOOKS, GUILD_RUST, RUST_ANNOUNCEMENTS,
        RUST_GENERAL, RUST_HELP, RUST_VOICE, sample_state, sample_state_with_unreads,
    };

    fn ready_navigation(state: &ClientState, remembered: &Preferences) -> Navigation {
        let mut navigation = Navigation::default();
        assert!(navigation.follow_state(state, remembered));
        navigation
    }

    #[test]
    fn nothing_is_chosen_before_ready_then_the_first_server_opens_at_its_first_channel() {
        let mut navigation = Navigation::default();
        assert!(!navigation.follow_state(&ClientState::default(), &Preferences::default()));
        assert_eq!(navigation.place(), None);

        let navigation = ready_navigation(&sample_state(), &Preferences::default());
        assert_eq!(
            navigation.place(),
            Some(Place::Server {
                guild_id: GUILD_RUST,
                channel_id: Some(RUST_ANNOUNCEMENTS)
            })
        );
    }

    #[test]
    fn the_remembered_place_and_channel_open_at_launch() {
        let state = sample_state();
        let mut remembered = Preferences {
            last_place: Some(RememberedPlace::Server {
                guild_id: GUILD_RUST,
            }),
            ..Preferences::default()
        };
        remembered.remember_channel(GUILD_RUST, RUST_HELP);
        let navigation = ready_navigation(&state, &remembered);
        assert_eq!(navigation.selected_channel_id(), Some(RUST_HELP));

        remembered.last_place = Some(RememberedPlace::Home {
            conversation_id: Some(CONVERSATION_ZOE),
        });
        let navigation = ready_navigation(&state, &remembered);
        assert_eq!(
            navigation.place(),
            Some(Place::Home {
                conversation_id: Some(CONVERSATION_ZOE)
            })
        );

        // A server left since then: the first one opens.
        remembered.last_place = Some(RememberedPlace::Server {
            guild_id: Snowflake::from_raw(404),
        });
        let navigation = ready_navigation(&state, &remembered);
        assert_eq!(navigation.selected_guild_id(), Some(GUILD_RUST));
    }

    #[test]
    fn each_server_reopens_at_its_last_channel_and_voice_channels_never_open() {
        let state = sample_state();
        let mut navigation = ready_navigation(&state, &Preferences::default());
        navigation.select_channel(&state, RUST_HELP);
        navigation.select_guild(&state, Some(GUILD_BOOKS), None);
        assert_eq!(navigation.selected_guild_id(), Some(GUILD_BOOKS));
        navigation.select_guild(&state, Some(GUILD_RUST), None);
        assert_eq!(navigation.selected_channel_id(), Some(RUST_HELP));

        navigation.select_guild(&state, Some(GUILD_RUST), Some(RUST_VOICE));
        assert_eq!(navigation.selected_channel_id(), Some(RUST_HELP));
        navigation.select_guild(&state, None, Some(Snowflake::from_raw(404)));
        assert_eq!(
            navigation.place(),
            Some(Place::Home {
                conversation_id: None
            })
        );
    }

    #[test]
    fn a_vanished_server_channel_or_conversation_gives_way() {
        let mut state = sample_state();
        let mut navigation = ready_navigation(&state, &Preferences::default());
        navigation.select_channel(&state, RUST_HELP);
        state.apply_event(
            mellow_protocol::GatewayEvent::ChannelDelete {
                guild_id: GUILD_RUST,
                id: RUST_HELP,
            },
            0,
        );
        assert!(navigation.follow_state(&state, &Preferences::default()));
        assert_eq!(navigation.selected_channel_id(), Some(RUST_ANNOUNCEMENTS));

        state.apply_event(
            mellow_protocol::GatewayEvent::GuildDelete { id: GUILD_RUST },
            0,
        );
        assert!(navigation.follow_state(&state, &Preferences::default()));
        assert_eq!(navigation.selected_guild_id(), Some(GUILD_BOOKS));
        assert!(!navigation.follow_state(&state, &Preferences::default()));

        navigation.select_guild(&state, None, Some(CONVERSATION_ANA));
        state.remove_direct_message_channel(CONVERSATION_ANA);
        assert!(navigation.follow_state(&state, &Preferences::default()));
        assert_eq!(
            navigation.place(),
            Some(Place::Home {
                conversation_id: None
            })
        );
    }

    #[test]
    fn alt_arrows_walk_the_channels_round_and_skip_to_unread_ones() {
        let state = sample_state_with_unreads(&[(RUST_HELP, 0)]);
        let mut navigation = ready_navigation(&state, &Preferences::default());
        // announcements, general, help: voice channels are skipped.
        assert_eq!(
            navigation.adjacent_channel(&state, Direction::Next, false),
            Some(RUST_GENERAL)
        );
        assert_eq!(
            navigation.adjacent_channel(&state, Direction::Previous, false),
            Some(RUST_HELP)
        );
        assert_eq!(
            navigation.adjacent_channel(&state, Direction::Next, true),
            Some(RUST_HELP)
        );
        navigation.select_channel(&state, RUST_HELP);
        assert_eq!(
            navigation.adjacent_channel(&state, Direction::Next, true),
            None
        );

        navigation.select_guild(&state, None, None);
        assert_eq!(
            navigation.adjacent_channel(&state, Direction::Next, false),
            Some(CONVERSATION_ANA)
        );
    }

    #[test]
    fn ctrl_alt_arrows_walk_the_rail_with_home_first() {
        let state = sample_state();
        let mut navigation = ready_navigation(&state, &Preferences::default());
        assert_eq!(
            navigation.adjacent_server(&state, Direction::Next),
            Some(Some(GUILD_BOOKS))
        );
        assert_eq!(
            navigation.adjacent_server(&state, Direction::Previous),
            Some(None)
        );
        navigation.select_guild(&state, Some(GUILD_BOOKS), None);
        assert_eq!(
            navigation.adjacent_server(&state, Direction::Next),
            Some(None)
        );
    }
}
