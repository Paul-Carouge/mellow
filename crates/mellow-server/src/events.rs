//! In-process event fan-out to gateway sessions (ARCHITECTURE.md §4.8). The domain publishes
//! here after each committed change; the gateway turns events into frames.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex, PoisonError};

use mellow_protocol::{GatewayEvent, PresenceStatus, Snowflake};
use tokio::sync::mpsc;

use crate::domain::session_token::SessionTokenSha256;

/// Events queued per session before it counts as too slow and is dropped (it then resumes or
/// resynchronises through REST, instead of slowing everyone else down).
const SESSION_EVENT_QUEUE_CAPACITY: usize = 512;

/// An event serialized once at publish time and shared by every receiving session, so memory and
/// CPU do not grow with the number of sessions.
#[derive(Debug)]
pub(crate) struct PublishedEvent {
    /// The JSON of the `GatewayEvent`, ready to be placed as the `d` of a dispatch frame.
    pub(crate) payload_json: Arc<str>,
}

pub(crate) type SharedEvent = Arc<PublishedEvent>;

fn published(event: &GatewayEvent) -> Option<SharedEvent> {
    match serde_json::to_string(event) {
        Ok(json) => Some(Arc::new(PublishedEvent {
            payload_json: json.into(),
        })),
        Err(error) => {
            tracing::error!(%error, "cannot serialize a gateway event");
            None
        }
    }
}

/// Identifies one gateway session inside the hub.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct SessionKey(u64);

#[derive(Debug)]
struct SessionEntry {
    user_id: Snowflake,
    token_sha256: SessionTokenSha256,
    guild_ids: HashSet<Snowflake>,
    /// `None` once the session was cut off (too slow, or its token revoked); the entry stays until
    /// `unregister_session` so presence is computed from live sessions only once.
    sender: Option<mpsc::Sender<SharedEvent>>,
}

#[derive(Debug, Default)]
struct HubState {
    next_session_key: u64,
    sessions: HashMap<SessionKey, SessionEntry>,
    guild_sessions: HashMap<Snowflake, HashSet<SessionKey>>,
    user_sessions: HashMap<Snowflake, HashSet<SessionKey>>,
}

impl HubState {
    /// Dropping the sender ends the session task, which tells its client to resynchronise.
    fn cut_off(&mut self, key: SessionKey) {
        if let Some(entry) = self.sessions.get_mut(&key) {
            entry.sender = None;
            // `guild_ids` is kept: the "offline" announcement at unregistration still needs it.
            for guild_id in &entry.guild_ids {
                if let Some(keys) = self.guild_sessions.get_mut(guild_id) {
                    keys.remove(&key);
                    if keys.is_empty() {
                        self.guild_sessions.remove(guild_id);
                    }
                }
            }
        }
    }

    /// Queues `event` for `keys`; sessions whose queue is full or closed are cut off.
    fn send_to(&mut self, keys: impl IntoIterator<Item = SessionKey>, event: &SharedEvent) {
        let slow_sessions: Vec<SessionKey> = keys
            .into_iter()
            .filter(|key| {
                self.sessions.get(key).is_some_and(|entry| {
                    entry
                        .sender
                        .as_ref()
                        .is_some_and(|sender| sender.try_send(event.clone()).is_err())
                })
            })
            .collect();
        for key in slow_sessions {
            self.cut_off(key);
        }
    }

    fn keys_of_guild(&self, guild_id: Snowflake) -> Vec<SessionKey> {
        self.guild_sessions
            .get(&guild_id)
            .map(|keys| keys.iter().copied().collect())
            .unwrap_or_default()
    }

    fn keys_of_user(&self, user_id: Snowflake) -> Vec<SessionKey> {
        self.user_sessions
            .get(&user_id)
            .map(|keys| keys.iter().copied().collect())
            .unwrap_or_default()
    }

    fn publish_to_guilds(
        &mut self,
        guild_ids: impl IntoIterator<Item = Snowflake>,
        event: &GatewayEvent,
    ) {
        let Some(event) = published(event) else {
            return;
        };
        for guild_id in guild_ids {
            let keys = self.keys_of_guild(guild_id);
            self.send_to(keys, &event);
        }
    }
}

/// Registering more sessions than allowed for one user.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct TooManySessions;

#[derive(Debug, Default)]
pub(crate) struct EventHub {
    state: Mutex<HubState>,
}

impl EventHub {
    fn state(&self) -> std::sync::MutexGuard<'_, HubState> {
        // The maps stay consistent even if a holder panicked mid-update: every update is a
        // single insert or remove.
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Registers a session (subscribed to no guild yet), if the user has fewer than
    /// `maximum_sessions_per_user` sessions; the check and the insert are atomic.
    pub(crate) fn register_session(
        &self,
        user_id: Snowflake,
        token_sha256: SessionTokenSha256,
        maximum_sessions_per_user: usize,
    ) -> Result<(SessionKey, mpsc::Receiver<SharedEvent>), TooManySessions> {
        let mut state = self.state();
        if state.user_sessions.get(&user_id).map_or(0, HashSet::len) >= maximum_sessions_per_user {
            return Err(TooManySessions);
        }
        let (sender, receiver) = mpsc::channel(SESSION_EVENT_QUEUE_CAPACITY);
        let key = SessionKey(state.next_session_key);
        state.next_session_key += 1;
        state.sessions.insert(
            key,
            SessionEntry {
                user_id,
                token_sha256,
                guild_ids: HashSet::new(),
                sender: Some(sender),
            },
        );
        state.user_sessions.entry(user_id).or_default().insert(key);
        Ok((key, receiver))
    }

    /// Subscribes the session to its user's guilds. When this is the user's first session,
    /// announces them online to those guilds.
    pub(crate) fn activate_session(&self, key: SessionKey, guild_ids: &[Snowflake]) {
        let mut state = self.state();
        let Some(entry) = state.sessions.get_mut(&key) else {
            return;
        };
        entry.guild_ids.extend(guild_ids.iter().copied());
        let user_id = entry.user_id;
        for guild_id in guild_ids {
            state
                .guild_sessions
                .entry(*guild_id)
                .or_default()
                .insert(key);
        }
        if state.user_sessions.get(&user_id).map_or(0, HashSet::len) == 1 {
            let presence = GatewayEvent::PresenceUpdate {
                user_id,
                status: PresenceStatus::Online,
            };
            state.publish_to_guilds(guild_ids.iter().copied(), &presence);
        }
    }

    /// Removes the session. When it was the user's last one, announces them offline to the
    /// guilds it was subscribed to, under the same lock, so a later "online" cannot be overtaken.
    pub(crate) fn unregister_session(&self, key: SessionKey) {
        let mut state = self.state();
        let Some(entry) = state.sessions.remove(&key) else {
            return;
        };
        for guild_id in &entry.guild_ids {
            if let Some(keys) = state.guild_sessions.get_mut(guild_id) {
                keys.remove(&key);
                if keys.is_empty() {
                    state.guild_sessions.remove(guild_id);
                }
            }
        }
        let was_last_session = state
            .user_sessions
            .get_mut(&entry.user_id)
            .is_some_and(|keys| {
                keys.remove(&key);
                keys.is_empty()
            });
        if was_last_session {
            state.user_sessions.remove(&entry.user_id);
            let presence = GatewayEvent::PresenceUpdate {
                user_id: entry.user_id,
                status: PresenceStatus::Offline,
            };
            state.publish_to_guilds(entry.guild_ids, &presence);
        }
    }

    /// Ends every live session opened with this token (logout), at once rather than at the next
    /// heartbeat.
    pub(crate) fn disconnect_token(&self, token_sha256: &SessionTokenSha256) {
        let mut state = self.state();
        let keys: Vec<SessionKey> = state
            .sessions
            .iter()
            .filter(|(_, entry)| entry.token_sha256 == *token_sha256)
            .map(|(key, _)| *key)
            .collect();
        for key in keys {
            state.cut_off(key);
        }
    }

    pub(crate) fn is_online(&self, user_id: Snowflake) -> bool {
        self.state().user_sessions.contains_key(&user_id)
    }

    /// Starts delivering `guild_id`'s events to every session of `user_id` (after joining it).
    pub(crate) fn subscribe_user_to_guild(&self, user_id: Snowflake, guild_id: Snowflake) {
        let mut state = self.state();
        for key in state.keys_of_user(user_id) {
            let is_live = state.sessions.get_mut(&key).is_some_and(|entry| {
                entry.guild_ids.insert(guild_id);
                entry.sender.is_some()
            });
            if is_live {
                state
                    .guild_sessions
                    .entry(guild_id)
                    .or_default()
                    .insert(key);
            }
        }
    }

    /// Stops delivering `guild_id`'s events to `user_id` (after leaving it).
    pub(crate) fn unsubscribe_user_from_guild(&self, user_id: Snowflake, guild_id: Snowflake) {
        let mut state = self.state();
        let keys = state.keys_of_user(user_id);
        for key in &keys {
            if let Some(entry) = state.sessions.get_mut(key) {
                entry.guild_ids.remove(&guild_id);
            }
        }
        if let Some(guild_keys) = state.guild_sessions.get_mut(&guild_id) {
            for key in keys {
                guild_keys.remove(&key);
            }
            if guild_keys.is_empty() {
                state.guild_sessions.remove(&guild_id);
            }
        }
    }

    pub(crate) fn publish_to_guild(&self, guild_id: Snowflake, event: &GatewayEvent) {
        self.state().publish_to_guilds([guild_id], event);
    }

    pub(crate) fn publish_to_user(&self, user_id: Snowflake, event: &GatewayEvent) {
        let Some(event) = published(event) else {
            return;
        };
        let mut state = self.state();
        let keys = state.keys_of_user(user_id);
        state.send_to(keys, &event);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const GUILD: Snowflake = Snowflake::from_raw(10);
    const ALICE: Snowflake = Snowflake::from_raw(1);
    const BOB: Snowflake = Snowflake::from_raw(2);
    const TOKEN_A: SessionTokenSha256 = [1; 32];
    const TOKEN_B: SessionTokenSha256 = [2; 32];
    const NO_CAP: usize = 100;

    fn typing() -> GatewayEvent {
        GatewayEvent::TypingStart {
            guild_id: GUILD,
            channel_id: Snowflake::from_raw(11),
            user_id: ALICE,
        }
    }

    fn drain(events: &mut mpsc::Receiver<SharedEvent>) -> Vec<String> {
        let mut received = Vec::new();
        while let Ok(event) = events.try_recv() {
            received.push(event.payload_json.to_string());
        }
        received
    }

    #[test]
    fn guild_events_reach_subscribed_sessions_only() {
        let hub = EventHub::default();
        let (alice_key, mut alice_events) = hub.register_session(ALICE, TOKEN_A, NO_CAP).unwrap();
        hub.activate_session(alice_key, &[GUILD]);
        let (bob_key, mut bob_events) = hub.register_session(BOB, TOKEN_B, NO_CAP).unwrap();
        hub.activate_session(bob_key, &[]);
        drain(&mut alice_events);

        hub.publish_to_guild(GUILD, &typing());
        assert_eq!(drain(&mut alice_events).len(), 1);
        assert_eq!(
            drain(&mut bob_events),
            Vec::<String>::new(),
            "bob is not in the guild"
        );

        hub.subscribe_user_to_guild(BOB, GUILD);
        hub.publish_to_guild(GUILD, &typing());
        assert_eq!(drain(&mut bob_events).len(), 1);
        hub.unsubscribe_user_from_guild(BOB, GUILD);
        hub.publish_to_guild(GUILD, &typing());
        assert_eq!(drain(&mut bob_events), Vec::<String>::new());
    }

    #[test]
    fn presence_goes_online_with_the_first_session_and_offline_with_the_last() {
        let hub = EventHub::default();
        let (watcher, mut watcher_events) = hub.register_session(BOB, TOKEN_B, NO_CAP).unwrap();
        hub.activate_session(watcher, &[GUILD]);
        drain(&mut watcher_events);

        let (first, _first_events) = hub.register_session(ALICE, TOKEN_A, NO_CAP).unwrap();
        hub.activate_session(first, &[GUILD]);
        let (second, _second_events) = hub.register_session(ALICE, TOKEN_A, NO_CAP).unwrap();
        hub.activate_session(second, &[GUILD]);
        let online = drain(&mut watcher_events);
        assert_eq!(
            online.len(),
            1,
            "only the first session announces online: {online:?}"
        );
        assert!(online[0].contains("online"));

        hub.unregister_session(first);
        assert_eq!(drain(&mut watcher_events), Vec::<String>::new());
        assert!(hub.is_online(ALICE));
        hub.unregister_session(second);
        let offline = drain(&mut watcher_events);
        assert!(
            offline.len() == 1 && offline[0].contains("offline"),
            "{offline:?}"
        );
        assert!(!hub.is_online(ALICE));
    }

    #[test]
    fn a_slow_session_is_cut_off_and_still_reports_offline_when_removed() {
        let hub = EventHub::default();
        let (watcher, mut watcher_events) = hub.register_session(BOB, TOKEN_B, NO_CAP).unwrap();
        hub.activate_session(watcher, &[GUILD]);
        let (slow, mut slow_events) = hub.register_session(ALICE, TOKEN_A, NO_CAP).unwrap();
        hub.activate_session(slow, &[GUILD]);
        for _ in 0..=SESSION_EVENT_QUEUE_CAPACITY {
            hub.publish_to_guild(GUILD, &typing());
            drain(&mut watcher_events);
        }
        let mut received = 0;
        while slow_events.try_recv().is_ok() {
            received += 1;
        }
        assert!(received <= SESSION_EVENT_QUEUE_CAPACITY);
        assert!(matches!(
            slow_events.try_recv(),
            Err(mpsc::error::TryRecvError::Disconnected)
        ));

        hub.unregister_session(slow);
        let offline = drain(&mut watcher_events);
        assert!(
            offline.iter().any(|event| event.contains("offline")),
            "{offline:?}"
        );
    }

    #[test]
    fn the_session_cap_is_atomic_and_logout_cuts_off_only_that_token() {
        let hub = EventHub::default();
        let (_, mut with_a) = hub.register_session(ALICE, TOKEN_A, 2).unwrap();
        let (_, mut with_b) = hub.register_session(ALICE, TOKEN_B, 2).unwrap();
        assert_eq!(
            hub.register_session(ALICE, TOKEN_A, 2).err(),
            Some(TooManySessions)
        );

        hub.disconnect_token(&TOKEN_A);
        assert!(matches!(
            with_a.try_recv(),
            Err(mpsc::error::TryRecvError::Disconnected)
        ));
        assert!(matches!(
            with_b.try_recv(),
            Err(mpsc::error::TryRecvError::Empty)
        ));
    }
}
