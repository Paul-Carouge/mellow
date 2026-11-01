//! Voice channels (ADR-0030): who is connected where, signalled over the gateway, with the media
//! relayed by the loop in `media`. Sits beside `gateway`; permissions and limits come from
//! `domain`. Logs never carry SDP, addresses or `Host` values.

mod address;
mod media;
#[cfg(test)]
mod tests;

use std::collections::{HashMap, HashSet};
use std::net::{Ipv4Addr, UdpSocket};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Weak};

use mellow_protocol::{
    GatewayEvent, Snowflake, VoiceDisconnectReason, VoiceJoinRefusal, VoiceState,
};
use tokio::sync::mpsc::{UnboundedReceiver, unbounded_channel};

use crate::config::VoiceConfig;
use crate::domain::guilds::{ChannelAudience, Guilds};
use crate::domain::voice_rules::{VoiceAccess, has_room_for_one_more, validate_voice_offer};
use crate::events::{EventHub, SessionKey};
pub(crate) use address::is_valid_host;
use address::{HostResolutions, announced_address, host_from_header};
use media::{MediaCommand, MediaEvent, MediaJoin};

/// A `voice_join` frame, with what the gateway knows of its sender.
#[derive(Debug)]
pub(crate) struct VoiceJoinRequest {
    pub(crate) user_id: Snowflake,
    pub(crate) session: SessionKey,
    pub(crate) channel_id: Snowflake,
    pub(crate) offer_id: u32,
    pub(crate) sdp_offer: String,
    pub(crate) self_mute: bool,
    pub(crate) self_deaf: bool,
    /// The `Host` header of the gateway connection, as sent.
    pub(crate) host_header: Option<String>,
}

/// A join checked and answered, waiting for room in the channel.
struct PreparedJoin {
    candidate_address: std::net::SocketAddr,
    /// Boxed: a session is about 20 kB, too much to carry across awaits inline.
    rtc: Box<str0m::Rtc>,
    sdp_answer: String,
}

/// One connected person, as the service knows them.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Participant {
    /// The gateway session the voice connection belongs to: it ends with that session.
    session: SessionKey,
    connection_id: u64,
    guild_id: Snowflake,
    channel_id: Snowflake,
    self_mute: bool,
    self_deaf: bool,
    /// Lacks `SPEAK`: their audio is not forwarded.
    suppressed: bool,
}

impl Participant {
    fn voice_state(&self, user_id: Snowflake) -> VoiceState {
        VoiceState {
            user_id,
            guild_id: self.guild_id,
            channel_id: self.channel_id,
            self_mute: self.self_mute,
            self_deaf: self.self_deaf,
            suppressed: self.suppressed,
        }
    }

    /// Whether others hear them: deafening oneself mutes too, as on Discord.
    const fn is_forwarded(&self) -> bool {
        !self.suppressed && !self.self_mute && !self.self_deaf
    }

    fn forwarding_command(&self, user_id: Snowflake) -> MediaCommand {
        MediaCommand::SetForwarding {
            user_id,
            is_forwarded: self.is_forwarded(),
            is_deafened: self.self_deaf,
        }
    }
}

#[derive(Debug)]
struct MediaHandle {
    commands: std::sync::mpsc::Sender<MediaCommand>,
    udp_port: u16,
}

#[derive(Debug)]
struct VoiceShared {
    /// `None` when voice is disabled or its UDP port could not be opened.
    media: Option<MediaHandle>,
    public_address: Option<String>,
    resolutions: HostResolutions,
    /// The HTTP listener is on loopback without TLS or proxy: a development instance, where a
    /// `localhost` candidate is replaced by the machine's network address.
    is_development_listener: bool,
    max_participants: usize,
    guilds: Arc<Guilds>,
    events: Arc<EventHub>,
    /// By user: one voice connection per user. An async lock, held across the audience reads,
    /// so the events of one change go out before those of the next.
    participants: tokio::sync::Mutex<HashMap<Snowflake, Participant>>,
    next_connection_id: AtomicU64,
}

/// Cheap to clone; every clone is the same service.
#[derive(Debug, Clone)]
pub(crate) struct VoiceService {
    shared: Arc<VoiceShared>,
}

impl VoiceService {
    /// Opens the UDP port and starts the media thread, unless voice is disabled. A port that
    /// cannot be opened is logged once; text chat goes on and joins get `voice_unavailable`.
    /// `is_development_listener`: the HTTP listener is on loopback without TLS or reverse proxy.
    pub(crate) fn start(
        config: &VoiceConfig,
        is_development_listener: bool,
        guilds: Arc<Guilds>,
        events: Arc<EventHub>,
    ) -> Self {
        let started = if config.enabled {
            start_media_loop(config.udp_port)
        } else {
            None
        };
        let (media, media_events) = started.unzip();
        let service = Self::with_media(
            config,
            is_development_listener,
            media,
            guilds,
            events.clone(),
        );
        if let Some(media_events) = media_events {
            tokio::spawn(run_voice_events(
                Arc::downgrade(&service.shared),
                media_events,
                events,
            ));
        }
        service
    }

    /// No voice at all: every join is refused with `voice_unavailable`.
    #[cfg(test)]
    pub(crate) fn disabled(guilds: Arc<Guilds>, events: Arc<EventHub>) -> Self {
        let config = VoiceConfig {
            enabled: false,
            ..VoiceConfig::default()
        };
        Self::with_media(&config, false, None, guilds, events)
    }

    fn with_media(
        config: &VoiceConfig,
        is_development_listener: bool,
        media: Option<MediaHandle>,
        guilds: Arc<Guilds>,
        events: Arc<EventHub>,
    ) -> Self {
        Self {
            shared: Arc::new(VoiceShared {
                media,
                public_address: config.public_address.clone(),
                resolutions: HostResolutions::default(),
                is_development_listener,
                max_participants: usize::try_from(config.max_participants).unwrap_or(usize::MAX),
                guilds,
                events,
                participants: tokio::sync::Mutex::default(),
                next_connection_id: AtomicU64::new(1),
            }),
        }
    }

    /// Whether voice is enabled and its UDP port open (`Ready.voice_enabled`).
    pub(crate) fn is_available(&self) -> bool {
        self.shared.media.is_some()
    }

    pub(crate) fn refuse_join(
        &self,
        session: SessionKey,
        channel_id: Snowflake,
        offer_id: u32,
        reason: VoiceJoinRefusal,
    ) {
        self.shared.events.publish_to_session(
            session,
            &GatewayEvent::VoiceJoinRefused {
                channel_id,
                offer_id,
                reason,
            },
        );
    }

    /// Everything a join needs before it touches the participants: the offer's shape, the
    /// permission (`domain`), the address to announce, and the answered session.
    async fn prepare_join(
        &self,
        request: &VoiceJoinRequest,
    ) -> Result<PreparedJoin, VoiceJoinRefusal> {
        let shared = &self.shared;
        let media = shared
            .media
            .as_ref()
            .ok_or(VoiceJoinRefusal::VoiceUnavailable)?;
        validate_voice_offer(&request.sdp_offer).map_err(|_| VoiceJoinRefusal::InvalidOffer)?;
        // Checked early so a refused join costs no DNS lookup or session; checked again under
        // the lock by `admit`.
        match shared
            .guilds
            .voice_access(request.user_id, request.channel_id)
            .await
        {
            Ok(VoiceAccess::Allowed { .. }) => {}
            Ok(VoiceAccess::ChannelGone | VoiceAccess::NotMember | VoiceAccess::Denied) => {
                return Err(VoiceJoinRefusal::MissingPermissions);
            }
            Err(error) => {
                tracing::warn!(%error, "voice join: cannot read the channel");
                return Err(VoiceJoinRefusal::VoiceUnavailable);
            }
        }
        // An address the administrator wrote is announced as is. One taken from the Host the
        // client used may be `localhost`: replaced by the machine's network address only on a
        // development instance, never on an exposed one (it would reveal its network address).
        let candidate_address = match &shared.public_address {
            Some(public_address) => {
                shared
                    .resolutions
                    .resolve(public_address, media.udp_port)
                    .await
            }
            None => match request.host_header.as_deref().and_then(host_from_header) {
                Some(host) => shared
                    .resolutions
                    .resolve(host, media.udp_port)
                    .await
                    .map(|address| announced_address(address, shared.is_development_listener)),
                None => None,
            },
        };
        let Some(candidate_address) = candidate_address else {
            tracing::warn!("voice join: no IPv4 address to announce; set [voice] public_address");
            return Err(VoiceJoinRefusal::VoiceUnavailable);
        };
        let sdp_offer = request.sdp_offer.clone();
        let answered =
            tokio::task::spawn_blocking(move || media::answer_offer(&sdp_offer, candidate_address))
                .await;
        let Ok(Some((rtc, sdp_answer))) = answered else {
            return Err(VoiceJoinRefusal::InvalidOffer);
        };
        Ok(PreparedJoin {
            candidate_address,
            rtc,
            sdp_answer,
        })
    }

    /// Joins, moves or reconnects. Answers the session with `VOICE_SESSION_DESCRIPTION` or
    /// `VOICE_JOIN_REFUSED`, then tells the channel's viewers.
    pub(crate) async fn join(&self, request: VoiceJoinRequest) {
        match self.prepare_join(&request).await {
            Ok(prepared) => self.admit(request, prepared).await,
            Err(reason) => self.refuse_join(
                request.session,
                request.channel_id,
                request.offer_id,
                reason,
            ),
        }
    }

    /// Adds the prepared participant under the participants lock. Access is read again there:
    /// a permission revoked while the offer was being answered is re-checked only for those
    /// already inserted, so a join racing it must see it here. A session that ended meanwhile
    /// (its `end_session` already ran) gets nothing.
    async fn admit(&self, request: VoiceJoinRequest, prepared: PreparedJoin) {
        let shared = &self.shared;
        let refuse = |reason| {
            self.refuse_join(
                request.session,
                request.channel_id,
                request.offer_id,
                reason,
            );
        };
        let PreparedJoin {
            candidate_address,
            rtc,
            sdp_answer,
            ..
        } = prepared;
        let Some(media) = &shared.media else {
            return refuse(VoiceJoinRefusal::VoiceUnavailable);
        };
        let mut participants = shared.participants.lock().await;
        if !shared.events.is_session_live(request.session) {
            return;
        }
        let (guild_id, user_limit, may_speak) = match shared
            .guilds
            .voice_access(request.user_id, request.channel_id)
            .await
        {
            Ok(VoiceAccess::Allowed {
                guild_id,
                user_limit,
                may_speak,
            }) => (guild_id, user_limit, may_speak),
            Ok(VoiceAccess::ChannelGone | VoiceAccess::NotMember | VoiceAccess::Denied) => {
                return refuse(VoiceJoinRefusal::MissingPermissions);
            }
            Err(error) => {
                tracing::warn!(%error, "voice join: cannot read the channel");
                return refuse(VoiceJoinRefusal::VoiceUnavailable);
            }
        };
        let previous = participants.get(&request.user_id).cloned();
        let others_in_channel = participants
            .iter()
            .filter(|(user_id, participant)| {
                **user_id != request.user_id && participant.channel_id == request.channel_id
            })
            .count();
        if !has_room_for_one_more(others_in_channel, user_limit) {
            return refuse(VoiceJoinRefusal::ChannelFull);
        }
        let others_on_instance = participants.len() - usize::from(previous.is_some());
        if others_on_instance >= shared.max_participants {
            return refuse(VoiceJoinRefusal::InstanceFull);
        }
        let participant = Participant {
            session: request.session,
            connection_id: shared.next_connection_id.fetch_add(1, Ordering::Relaxed),
            guild_id,
            channel_id: request.channel_id,
            self_mute: request.self_mute,
            self_deaf: request.self_deaf,
            suppressed: !may_speak,
        };
        let join = MediaJoin {
            user_id: request.user_id,
            connection_id: participant.connection_id,
            channel_id: request.channel_id,
            rtc,
            candidate_address,
            is_forwarded: participant.is_forwarded(),
            is_deafened: participant.self_deaf,
        };
        if media
            .commands
            .send(MediaCommand::Join(Box::new(join)))
            .is_err()
        {
            tracing::error!("voice join: the media thread is gone");
            return refuse(VoiceJoinRefusal::VoiceUnavailable);
        }
        participants.insert(request.user_id, participant.clone());
        shared.events.publish_to_session(
            request.session,
            &GatewayEvent::VoiceSessionDescription {
                channel_id: request.channel_id,
                offer_id: request.offer_id,
                sdp_answer,
            },
        );
        shared
            .announce_join(request.user_id, previous, &participant)
            .await;
    }

    /// Leaves the voice channel `session` is connected to, if any.
    pub(crate) async fn leave(&self, user_id: Snowflake, session: SessionKey) {
        let mut participants = self.shared.participants.lock().await;
        if participants
            .get(&user_id)
            .is_some_and(|participant| participant.session == session)
        {
            self.shared
                .disconnect(&mut participants, user_id, None)
                .await;
        }
    }

    /// The user's own mute and deafen, from the session connected to voice.
    pub(crate) async fn set_self_state(
        &self,
        user_id: Snowflake,
        session: SessionKey,
        self_mute: bool,
        self_deaf: bool,
    ) {
        let mut participants = self.shared.participants.lock().await;
        let Some(participant) = participants
            .get_mut(&user_id)
            .filter(|participant| participant.session == session)
        else {
            return;
        };
        if (participant.self_mute, participant.self_deaf) == (self_mute, self_deaf) {
            return;
        }
        participant.self_mute = self_mute;
        participant.self_deaf = self_deaf;
        let participant = participant.clone();
        self.shared
            .send_media_command(participant.forwarding_command(user_id));
        self.shared
            .publish_state_updated(user_id, &participant)
            .await;
    }

    /// The gateway session ended for good (not a resume): its voice connection ends with it.
    pub(crate) async fn end_session(&self, session: SessionKey) {
        let mut participants = self.shared.participants.lock().await;
        let user_id = participants
            .iter()
            .find(|(_, participant)| participant.session == session)
            .map(|(user_id, _)| *user_id);
        if let Some(user_id) = user_id {
            self.shared
                .disconnect(&mut participants, user_id, None)
                .await;
        }
    }

    /// Who is connected to the voice channels in `channel_ids` (those the user can view).
    pub(crate) async fn voice_states_in(
        &self,
        channel_ids: &HashSet<Snowflake>,
    ) -> Vec<VoiceState> {
        self.shared
            .participants
            .lock()
            .await
            .iter()
            .filter(|(_, participant)| channel_ids.contains(&participant.channel_id))
            .map(|(user_id, participant)| participant.voice_state(*user_id))
            .collect()
    }
}

impl VoiceShared {
    /// Tells the others what a join changed: the session it replaced, a move, a new state.
    async fn announce_join(
        &self,
        user_id: Snowflake,
        previous: Option<Participant>,
        participant: &Participant,
    ) {
        let Some(previous) = previous else {
            self.publish_state_updated(user_id, participant).await;
            return;
        };
        if previous.session != participant.session {
            self.events.publish_to_session(
                previous.session,
                &GatewayEvent::VoiceDisconnected {
                    channel_id: previous.channel_id,
                    reason: VoiceDisconnectReason::MovedElsewhere,
                },
            );
        }
        if previous.channel_id != participant.channel_id {
            self.publish_move(user_id, &previous, participant).await;
        } else if previous.voice_state(user_id) != participant.voice_state(user_id) {
            self.publish_state_updated(user_id, participant).await;
        }
        // Joining the same channel again with the same state reconnects the media (an ICE
        // restart): nothing is announced, so the client does not react to its own reconnection.
    }

    fn send_media_command(&self, command: MediaCommand) {
        if let Some(media) = &self.media
            && media.commands.send(command).is_err()
        {
            tracing::error!("voice: the media thread is gone");
        }
    }

    /// Removes the participant and stops their media; tells their session why when the server
    /// ended it (`reason`), and the channel's viewers that they left.
    async fn disconnect(
        &self,
        participants: &mut HashMap<Snowflake, Participant>,
        user_id: Snowflake,
        reason: Option<VoiceDisconnectReason>,
    ) {
        let Some(participant) = participants.remove(&user_id) else {
            return;
        };
        self.send_media_command(MediaCommand::Leave { user_id });
        if let Some(reason) = reason {
            self.events.publish_to_session(
                participant.session,
                &GatewayEvent::VoiceDisconnected {
                    channel_id: participant.channel_id,
                    reason,
                },
            );
        }
        self.publish_state_deleted(user_id, &participant).await;
    }

    async fn publish_to_viewers(
        &self,
        guild_id: Snowflake,
        channel_id: Snowflake,
        event: &GatewayEvent,
    ) {
        match self.guilds.audience_of_guild_channel(channel_id).await {
            Ok(Some(audience)) => self.guilds.publish_to_audience(guild_id, &audience, event),
            // Deleted: its viewers dropped it, and everything in it, on CHANNEL_DELETE.
            Ok(None) => {}
            Err(error) => tracing::warn!(%error, "voice: cannot read who views a channel"),
        }
    }

    /// A move is one `VOICE_STATE_UPDATE` with the new channel; viewers of the old channel who
    /// cannot view the new one would never see it, so they get a `VOICE_STATE_DELETE` instead.
    async fn publish_move(
        &self,
        user_id: Snowflake,
        previous: &Participant,
        current: &Participant,
    ) {
        self.publish_state_updated(user_id, current).await;
        let audiences = (
            self.guilds
                .audience_of_guild_channel(previous.channel_id)
                .await,
            self.guilds
                .audience_of_guild_channel(current.channel_id)
                .await,
        );
        // Compared as sets of people, so a move to another guild's channel is covered too.
        let viewers_of = |audience: ChannelAudience, guild_id: Snowflake| match audience {
            ChannelAudience::EveryMember => self.events.online_user_ids_of_guild(guild_id),
            ChannelAudience::Members(viewers) => viewers,
        };
        let viewers_left_behind: HashSet<Snowflake> = match audiences {
            (Ok(None), _) => return,
            (Ok(Some(old)), Ok(new)) => {
                let old_viewers = viewers_of(old, previous.guild_id);
                let new_viewers = new
                    .map(|new| viewers_of(new, current.guild_id))
                    .unwrap_or_default();
                old_viewers.difference(&new_viewers).copied().collect()
            }
            (Err(error), _) | (_, Err(error)) => {
                tracing::warn!(%error, "voice: cannot read who views a channel");
                return;
            }
        };
        self.events.publish_to_guild_members(
            previous.guild_id,
            &viewers_left_behind,
            &GatewayEvent::VoiceStateDelete {
                guild_id: previous.guild_id,
                channel_id: previous.channel_id,
                user_id,
            },
        );
    }

    async fn publish_state_updated(&self, user_id: Snowflake, participant: &Participant) {
        let event = GatewayEvent::VoiceStateUpdate {
            voice_state: participant.voice_state(user_id),
        };
        self.publish_to_viewers(participant.guild_id, participant.channel_id, &event)
            .await;
    }

    async fn publish_state_deleted(&self, user_id: Snowflake, participant: &Participant) {
        let event = GatewayEvent::VoiceStateDelete {
            guild_id: participant.guild_id,
            channel_id: participant.channel_id,
            user_id,
        };
        self.publish_to_viewers(participant.guild_id, participant.channel_id, &event)
            .await;
    }

    async fn apply_media_event(&self, event: MediaEvent) {
        let mut participants = self.participants.lock().await;
        match event {
            MediaEvent::Ended {
                user_id,
                connection_id,
            } => {
                if participants
                    .get(&user_id)
                    .is_some_and(|participant| participant.connection_id == connection_id)
                {
                    self.disconnect(
                        &mut participants,
                        user_id,
                        Some(VoiceDisconnectReason::MediaTimeout),
                    )
                    .await;
                }
            }
            MediaEvent::SlotAssigned {
                listener_id,
                connection_id,
                mid,
                speaker_id,
            } => {
                let Some(listener) = participants
                    .get(&listener_id)
                    .filter(|listener| listener.connection_id == connection_id)
                else {
                    return;
                };
                self.events.publish_to_session(
                    listener.session,
                    &GatewayEvent::VoiceSlotAssigned {
                        channel_id: listener.channel_id,
                        mid: mid.to_string(),
                        user_id: speaker_id,
                    },
                );
            }
            MediaEvent::Speaking {
                user_id,
                connection_id,
                is_speaking,
            } => {
                let Some(participant) = participants.get(&user_id).filter(|participant| {
                    participant.connection_id == connection_id
                        && (participant.is_forwarded() || !is_speaking)
                }) else {
                    return;
                };
                let event = GatewayEvent::Speaking {
                    guild_id: participant.guild_id,
                    channel_id: participant.channel_id,
                    user_id,
                    speaking: is_speaking,
                };
                self.publish_to_viewers(participant.guild_id, participant.channel_id, &event)
                    .await;
            }
        }
    }

    /// Checks again everyone connected in `guild_ids`, after a change of who may view, connect or
    /// speak there: the ones who lost access are disconnected, the others gain or lose `SPEAK`.
    /// Then everyone still connected there is announced again to their channel's viewers, so a
    /// member who just joined the guild or started to see a channel learns who is in it
    /// (`VOICE_STATE_UPDATE` is idempotent for those who knew).
    async fn reevaluate(&self, guild_ids: &HashSet<Snowflake>) {
        let mut participants = self.participants.lock().await;
        let affected: Vec<(Snowflake, Snowflake)> = participants
            .iter()
            .filter(|(_, participant)| guild_ids.contains(&participant.guild_id))
            .map(|(user_id, participant)| (*user_id, participant.channel_id))
            .collect();
        for (user_id, channel_id) in &affected {
            let access = match self.guilds.voice_access(*user_id, *channel_id).await {
                Ok(access) => access,
                Err(error) => {
                    tracing::warn!(%error, "voice: cannot check a participant's access again");
                    continue;
                }
            };
            let reason = match access {
                VoiceAccess::ChannelGone => VoiceDisconnectReason::ChannelDeleted,
                VoiceAccess::NotMember => VoiceDisconnectReason::RemovedFromGuild,
                VoiceAccess::Denied => VoiceDisconnectReason::PermissionsChanged,
                VoiceAccess::Allowed { may_speak, .. } => {
                    if let Some(participant) = participants
                        .get_mut(user_id)
                        .filter(|participant| participant.suppressed == may_speak)
                    {
                        participant.suppressed = !may_speak;
                        let command = participant.forwarding_command(*user_id);
                        self.send_media_command(command);
                    }
                    continue;
                }
            };
            self.disconnect(&mut participants, *user_id, Some(reason))
                .await;
        }
        for (user_id, _) in affected {
            if let Some(participant) = participants.get(&user_id) {
                self.publish_state_updated(user_id, participant).await;
            }
        }
    }
}

/// Binds the voice port on every IPv4 address and starts the media thread.
fn start_media_loop(udp_port: u16) -> Option<(MediaHandle, UnboundedReceiver<MediaEvent>)> {
    // Installed once per process; later calls keep the first provider.
    str0m::crypto::from_feature_flags().install_process_default();
    let socket = match UdpSocket::bind((Ipv4Addr::UNSPECIFIED, udp_port)) {
        Ok(socket) => socket,
        Err(error) => {
            tracing::error!(
                udp_port,
                %error,
                "voice: cannot open the UDP port; voice channels are unavailable until it is free"
            );
            return None;
        }
    };
    let bound_port = match socket.local_addr() {
        Ok(address) => address.port(),
        Err(error) => {
            tracing::error!(%error, "voice: cannot read the UDP port; voice channels are unavailable");
            return None;
        }
    };
    let (commands, command_receiver) = std::sync::mpsc::channel();
    // Unbounded: speaking edges and endings are a few per participant and second at most, and
    // the media thread must never wait on the async side.
    let (event_sender, media_events) = unbounded_channel();
    let spawned = std::thread::Builder::new()
        .name("mellow-voice-media".into())
        .spawn(move || media::run_media_loop(socket, &command_receiver, &event_sender));
    if let Err(error) = spawned {
        tracing::error!(%error, "voice: cannot start the media thread; voice channels are unavailable");
        return None;
    }
    tracing::info!(udp_port = bound_port, "voice media listening");
    Some((
        MediaHandle {
            commands,
            udp_port: bound_port,
        },
        media_events,
    ))
}

/// Applies the media thread's events and re-checks participants after access changes, until the
/// service is dropped (the media thread then stops and closes `media_events`).
async fn run_voice_events(
    shared: Weak<VoiceShared>,
    mut media_events: UnboundedReceiver<MediaEvent>,
    events: Arc<EventHub>,
) {
    loop {
        // Each step runs as its own task: a panic in one is logged and the loop goes on, so
        // permission re-checks never stop silently while the server runs.
        let step = tokio::select! {
            event = media_events.recv() => {
                let Some(shared) = shared.upgrade() else {
                    return;
                };
                let Some(event) = event else {
                    tracing::error!("voice: the media thread stopped; voice channels are unavailable");
                    return;
                };
                tokio::spawn(async move { shared.apply_media_event(event).await })
            }
            guild_ids = events.next_guild_access_changes() => {
                let Some(shared) = shared.upgrade() else {
                    return;
                };
                tokio::spawn(async move { shared.reevaluate(&guild_ids).await })
            }
        };
        if let Err(error) = step.await {
            tracing::error!(%error, "voice: handling a voice event failed");
        }
    }
}
