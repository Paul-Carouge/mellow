//! Keeps the web client's hand-written wire types (`web/src/lib/protocol.ts`, ADR-0019) in step
//! with this crate. It serialises a sample of every type the web reads or sends, every variant of
//! every enum included, into `web/src/lib/protocol-wire-samples.generated.ts`, where `npm run
//! check` type-checks each one against `protocol.ts` with `satisfies`: a field renamed, added,
//! retyped or made nullable here, or a variant added, fails there. The web's unit tests compare
//! the shared constants and permission bits.
//!
//! This test fails when the committed file is stale. Regenerate it with:
//! `UPDATE_WIRE_SAMPLES=1 cargo test -p mellow-protocol --test typescript_wire_samples`

use std::path::PathBuf;

use mellow_protocol::{
    ApiErrorCode, Attachment, AuditAction, AuditChange, AuditChangeKey, AuditLogEntry,
    AuthorSearchFacet, Ban, Channel, ChannelKind, ChannelPlacement, ChannelSearchFacet,
    ChannelUnread, ChosenStatus, ClientFrame, CurrentUser, CustomEmoji, DirectMessageChannel,
    ForwardOrigin, GatewayEvent, GifSettings, Guild, GuildInvite, GuildSummary, Hello,
    HighlightRange, InitialCategory, InitialTextChannel, Invite, LinkPreview, LinkPreviewImage,
    MAXIMUM_COUNTED_UNREAD_MENTIONS, MAXIMUM_GIF_FAVOURITES,
    MAXIMUM_GROUP_CONVERSATION_NAME_LENGTH, MAXIMUM_GROUP_CONVERSATION_PARTICIPANTS,
    MAXIMUM_POLL_ANSWER_LENGTH, MAXIMUM_POLL_ANSWERS, MAXIMUM_POLL_QUESTION_LENGTH,
    MAXIMUM_REACHABLE_SEARCH_RESULTS, MAXIMUM_SCREEN_SOUND_RECEIVE_SLOTS,
    MAXIMUM_SEARCH_QUERY_LENGTH, MAXIMUM_SEARCH_QUERY_PARTS, MAXIMUM_SEARCH_RESULTS_PER_PAGE,
    MAXIMUM_VIDEO_RECEIVE_SLOTS, MAXIMUM_VOICE_CHANNEL_PARTICIPANTS, MAXIMUM_VOICE_RECEIVE_SLOTS,
    MINIMUM_MILLISECONDS_BETWEEN_SOUNDS, MINIMUM_POLL_ANSWERS, Member, Message, MessageReaction,
    MessageSearchHit, MessageSearchQuery, MessageSearchResults, MessageSearchSort, OverwriteKind,
    PermissionOverwrite, Permissions, Poll, PollAnswer, PollVote, Presence, PresenceStatus,
    ProfileImage, PublicUser, ReactionSummary, ReadState, Ready, Relationship, RelationshipKind,
    Role, ServerFrame, SessionCreated, SimulcastLayer, Snowflake, SoundboardSound, VideoTileSize,
    VoiceDisconnectReason, VoiceJoinRefusal, VoiceState, VoiceVideoRefusal, WatchedCamera,
};
use serde::Serialize;
use serde_json::{Value, json};

const GENERATED_FILE_PATH: &str = "../../web/src/lib/protocol-wire-samples.generated.ts";
const UPDATE_VARIABLE: &str = "UPDATE_WIRE_SAMPLES";

/// One sample per listed variant. The `match` stops compiling when the enum gains a variant this
/// list lacks, and each sample must be the variant its own line names.
macro_rules! every_variant {
    ($enum:ident { $($pattern:pat => $sample:expr),+ $(,)? }) => {{
        let samples: Vec<$enum> = vec![$($sample),+];
        for (index, sample) in samples.iter().enumerate() {
            // A variant may have several samples: its repeated patterns are unreachable.
            #[allow(unreachable_patterns, clippy::unnested_or_patterns)]
            match sample {
                $($pattern)|+ => {}
            }
            let is_named_variant = [$(matches!(sample, $pattern)),+];
            assert!(
                is_named_variant[index],
                "sample {index} of {} is not the variant its line names",
                stringify!($enum),
            );
        }
        samples
    }};
    ($enum:ident: $($variant:ident),+ $(,)?) => {
        every_variant!($enum { $($enum::$variant => $enum::$variant),+ })
    };
}

/// How `protocol.ts` declares a type, which decides what the generated file checks.
enum TypeScriptShape {
    /// An interface: every sample must satisfy it, without extra fields.
    Interface,
    /// A union of string literals: also, no member may lack a sample.
    StringUnion,
    /// A union of objects told apart by this field: also, no tag may lack a sample.
    TaggedUnion(&'static str),
}

struct SampledType {
    type_name: &'static str,
    shape: TypeScriptShape,
    samples: Vec<Value>,
}

fn sampled<T: Serialize>(
    type_name: &'static str,
    shape: TypeScriptShape,
    samples: &[T],
) -> serde_json::Result<SampledType> {
    Ok(SampledType {
        type_name,
        shape,
        samples: samples
            .iter()
            .map(serde_json::to_value)
            .collect::<serde_json::Result<_>>()?,
    })
}

const fn id(raw: u64) -> Snowflake {
    Snowflake::from_raw(raw)
}

fn picture() -> ProfileImage {
    ProfileImage {
        id: id(900),
        is_animated: true,
    }
}

/// Optional fields filled, so their names and types are checked.
fn public_user() -> PublicUser {
    PublicUser {
        id: id(10),
        username: "ada".into(),
        display_name: "Ada".into(),
        avatar: Some(picture()),
        banner: Some(picture()),
    }
}

/// Optional fields left out, so their absence or `null` is checked.
fn sparse_public_user() -> PublicUser {
    PublicUser {
        avatar: None,
        banner: None,
        ..public_user()
    }
}

fn current_user() -> CurrentUser {
    CurrentUser {
        id: id(10),
        username: "ada".into(),
        display_name: "Ada".into(),
        email: "ada@example.org".into(),
        is_instance_administrator: true,
        avatar: Some(picture()),
        banner: Some(picture()),
        status: ChosenStatus::DoNotDisturb,
        custom_status: Some("Writing notes".into()),
    }
}

fn sparse_current_user() -> CurrentUser {
    CurrentUser {
        avatar: None,
        banner: None,
        custom_status: None,
        ..current_user()
    }
}

fn presence() -> Presence {
    Presence {
        user_id: id(10),
        status: PresenceStatus::Idle,
        custom_status: Some("Away".into()),
    }
}

fn sparse_presence() -> Presence {
    Presence {
        custom_status: None,
        ..presence()
    }
}

fn custom_emoji() -> CustomEmoji {
    CustomEmoji {
        id: id(500),
        name: "party".into(),
        is_animated: true,
    }
}

fn sound() -> SoundboardSound {
    SoundboardSound {
        id: id(600),
        name: "Tada".into(),
        emoji: Some(":500".into()),
        duration_milliseconds: 1_200,
    }
}

fn sparse_sound() -> SoundboardSound {
    SoundboardSound {
        emoji: None,
        ..sound()
    }
}

fn overwrite() -> PermissionOverwrite {
    PermissionOverwrite {
        id: id(20),
        kind: OverwriteKind::Role,
        allow: Permissions::SEND_MESSAGES,
        deny: Permissions::ATTACH_FILES,
    }
}

fn channel() -> Channel {
    Channel {
        id: id(30),
        guild_id: id(1),
        kind: ChannelKind::Voice,
        name: "Lounge".into(),
        topic: "Chat".into(),
        parent_id: Some(id(31)),
        position: 2,
        icon: Some(picture()),
        permission_overwrites: vec![overwrite()],
        user_limit: Some(8),
    }
}

fn sparse_channel() -> Channel {
    Channel {
        kind: ChannelKind::Text,
        parent_id: None,
        icon: None,
        permission_overwrites: Vec::new(),
        user_limit: None,
        ..channel()
    }
}

fn role() -> Role {
    Role {
        id: id(20),
        name: "Moderators".into(),
        color: 0x00ff_8800,
        position: 1,
        permissions: Permissions::MANAGE_MESSAGES | Permissions::KICK_MEMBERS,
        is_displayed_separately: true,
        is_mentionable: true,
    }
}

fn guild() -> Guild {
    Guild {
        id: id(1),
        name: "Mellow".into(),
        owner_id: id(10),
        icon: Some(picture()),
        my_permissions: Permissions::all(),
        my_role_ids: vec![id(20)],
        roles: vec![role()],
        channels: vec![channel(), sparse_channel()],
        emojis: vec![custom_emoji()],
        sounds: vec![sound(), sparse_sound()],
        my_join_sound_id: Some(id(600)),
    }
}

fn sparse_guild() -> Guild {
    Guild {
        icon: None,
        my_role_ids: Vec::new(),
        roles: Vec::new(),
        channels: Vec::new(),
        emojis: Vec::new(),
        sounds: Vec::new(),
        my_join_sound_id: None,
        ..guild()
    }
}

fn member() -> Member {
    Member {
        user: public_user(),
        role_ids: vec![id(20)],
        joined_at_unix_milliseconds: 1_700_000_000_000,
    }
}

fn attachment() -> Attachment {
    Attachment {
        id: id(700),
        file_name: "photo.png".into(),
        content_type: "image/png".into(),
        size_bytes: 52_000,
        width: Some(640),
        height: Some(480),
    }
}

fn link_preview() -> LinkPreview {
    LinkPreview {
        id: id(900),
        url: "https://example.org/article".into(),
        title: Some("An article".into()),
        description: Some("What it says".into()),
        site_name: Some("Example".into()),
        theme_color: Some(0x0033_aa77),
        image: Some(LinkPreviewImage {
            width: 400,
            height: 210,
        }),
    }
}

fn sparse_link_preview() -> LinkPreview {
    LinkPreview {
        id: id(901),
        url: "https://example.org/".into(),
        title: Some("Example".into()),
        description: None,
        site_name: None,
        theme_color: None,
        image: None,
    }
}

fn sparse_attachment() -> Attachment {
    Attachment {
        content_type: "application/octet-stream".into(),
        width: None,
        height: None,
        ..attachment()
    }
}

fn reaction_summary() -> ReactionSummary {
    ReactionSummary {
        emoji: "👍".into(),
        count: 3,
        me: true,
    }
}

fn message() -> Message {
    Message {
        id: id(800),
        channel_id: id(30),
        author: public_user(),
        content: "Hello @ada and <@&20>".into(),
        reply_to_id: Some(id(799)),
        edited_at_unix_milliseconds: Some(1_700_000_000_500),
        nonce: Some("nonce-1".into()),
        attachments: vec![attachment(), sparse_attachment()],
        reactions: vec![reaction_summary()],
        pinned: true,
        mention_ids: vec![id(10)],
        mention_role_ids: vec![id(20)],
        link_previews: vec![link_preview(), sparse_link_preview()],
        poll: Some(Poll {
            question: "Lunch?".into(),
            answers: vec![
                PollAnswer {
                    id: 1,
                    text: "Pizza".into(),
                    vote_count: 3,
                    me_voted: true,
                },
                PollAnswer {
                    id: 2,
                    text: "Soup".into(),
                    vote_count: 0,
                    me_voted: false,
                },
            ],
            allows_multiple_answers: true,
            expires_at_unix_milliseconds: 1_700_000_100_000,
            ended_at_unix_milliseconds: Some(1_700_000_050_000),
        }),
        forwarded_from: Some(ForwardOrigin {
            channel_id: id(31),
            message_id: id(700),
        }),
    }
}

fn sparse_message() -> Message {
    Message {
        author: sparse_public_user(),
        reply_to_id: None,
        edited_at_unix_milliseconds: None,
        nonce: None,
        attachments: Vec::new(),
        reactions: Vec::new(),
        pinned: false,
        mention_ids: Vec::new(),
        mention_role_ids: Vec::new(),
        link_previews: Vec::new(),
        poll: None,
        forwarded_from: None,
        ..message()
    }
}

fn message_reaction() -> MessageReaction {
    MessageReaction {
        guild_id: Some(id(1)),
        channel_id: id(30),
        message_id: id(800),
        user_id: id(10),
        emoji: ":500".into(),
    }
}

fn sparse_message_reaction() -> MessageReaction {
    MessageReaction {
        guild_id: None,
        ..message_reaction()
    }
}

fn read_state() -> ReadState {
    ReadState {
        channel_id: id(30),
        last_read_message_id: id(800),
    }
}

fn channel_unread() -> ChannelUnread {
    ChannelUnread {
        channel_id: id(30),
        last_message_id: id(801),
        mention_count: 2,
    }
}

fn direct_message_channel() -> DirectMessageChannel {
    DirectMessageChannel {
        id: id(40),
        recipients: vec![public_user(), sparse_public_user()],
        last_message_id: Some(id(800)),
        owner_id: Some(id(10)),
        name: "Friends".into(),
    }
}

fn sparse_direct_message_channel() -> DirectMessageChannel {
    DirectMessageChannel {
        recipients: vec![sparse_public_user()],
        last_message_id: None,
        owner_id: None,
        name: String::new(),
        ..direct_message_channel()
    }
}

fn relationship() -> Relationship {
    Relationship {
        user: public_user(),
        kind: RelationshipKind::IncomingRequest,
        since_unix_milliseconds: 1_700_000_000_000,
    }
}

fn voice_state() -> VoiceState {
    VoiceState {
        user_id: id(10),
        guild_id: id(1),
        channel_id: id(30),
        self_mute: true,
        self_deaf: false,
        suppressed: false,
        join_sound_id: Some(id(600)),
        self_video: true,
        self_stream: false,
    }
}

fn sparse_voice_state() -> VoiceState {
    VoiceState {
        join_sound_id: None,
        ..voice_state()
    }
}

fn ready() -> Ready {
    Ready {
        session_id: "session-1".into(),
        user: current_user(),
        guilds: vec![guild(), sparse_guild()],
        read_states: vec![read_state()],
        online_user_ids: vec![id(10)],
        direct_message_channels: vec![direct_message_channel()],
        channel_unreads: vec![channel_unread()],
        presences: vec![presence(), sparse_presence()],
        relationships: vec![relationship()],
        may_create_guilds: true,
        voice_states: vec![voice_state(), sparse_voice_state()],
        voice_enabled: true,
    }
}

fn sparse_ready() -> Ready {
    Ready {
        user: sparse_current_user(),
        guilds: Vec::new(),
        read_states: Vec::new(),
        online_user_ids: Vec::new(),
        direct_message_channels: Vec::new(),
        channel_unreads: Vec::new(),
        presences: Vec::new(),
        relationships: Vec::new(),
        may_create_guilds: false,
        voice_states: Vec::new(),
        voice_enabled: false,
        ..ready()
    }
}

fn invite() -> Invite {
    Invite {
        code: "aBcD1234".into(),
        guild_id: id(1),
        uses: 2,
        max_uses: Some(10),
        expires_at_unix_milliseconds: Some(1_700_086_400_000),
    }
}

fn guild_invite() -> GuildInvite {
    GuildInvite {
        code: "aBcD1234".into(),
        creator: public_user(),
        uses: 2,
        max_uses: Some(10),
        expires_at_unix_milliseconds: Some(1_700_086_400_000),
        created_at_unix_milliseconds: 1_700_000_000_000,
    }
}

fn audit_log_entry() -> AuditLogEntry {
    AuditLogEntry {
        id: id(1_000),
        actor_id: Some(id(10)),
        action: AuditAction::RoleUpdate,
        target_id: Some(id(20)),
        context_id: Some(id(1)),
        reason: "Tidy up".into(),
        changes: vec![AuditChange {
            key: AuditChangeKey::Color,
            old_value: "0".into(),
            new_value: "16746496".into(),
        }],
    }
}

fn sparse_audit_log_entry() -> AuditLogEntry {
    AuditLogEntry {
        actor_id: None,
        target_id: None,
        context_id: None,
        reason: String::new(),
        changes: Vec::new(),
        ..audit_log_entry()
    }
}

fn message_search_results() -> MessageSearchResults {
    MessageSearchResults {
        hits: vec![MessageSearchHit {
            message: message(),
            highlights: vec![HighlightRange { start: 0, end: 5 }],
        }],
        total_count: 1,
        total_count_is_lower_bound: false,
        channel_facets: vec![ChannelSearchFacet {
            channel_id: id(30),
            count: 1,
        }],
        author_facets: vec![AuthorSearchFacet {
            author: public_user(),
            count: 1,
        }],
    }
}

#[allow(clippy::too_many_lines)] // One flat list: a sample per variant.
fn gateway_events() -> Vec<GatewayEvent> {
    every_variant!(GatewayEvent {
        GatewayEvent::MessageCreate(_) => GatewayEvent::MessageCreate(message()),
        GatewayEvent::MessageCreate(_) => GatewayEvent::MessageCreate(sparse_message()),
        GatewayEvent::MessageUpdate(_) => GatewayEvent::MessageUpdate(message()),
        GatewayEvent::MessageDelete { .. } => GatewayEvent::MessageDelete {
            guild_id: Some(id(1)),
            channel_id: id(30),
            id: id(800),
        },
        GatewayEvent::MessageDelete { .. } => GatewayEvent::MessageDelete {
            guild_id: None,
            channel_id: id(40),
            id: id(800),
        },
        GatewayEvent::MessagePollVoteAdd(_) => GatewayEvent::MessagePollVoteAdd(PollVote {
            guild_id: Some(id(1)),
            channel_id: id(30),
            message_id: id(800),
            answer_id: 2,
            user_id: id(10),
        }),
        GatewayEvent::MessagePollVoteRemove(_) => GatewayEvent::MessagePollVoteRemove(PollVote {
            guild_id: None,
            channel_id: id(40),
            message_id: id(800),
            answer_id: 1,
            user_id: id(10),
        }),
        GatewayEvent::MessageLinkPreviewsUpdate { .. } => GatewayEvent::MessageLinkPreviewsUpdate {
            guild_id: Some(id(1)),
            channel_id: id(30),
            message_id: id(800),
            link_previews: vec![link_preview()],
        },
        GatewayEvent::MessageLinkPreviewsUpdate { .. } => GatewayEvent::MessageLinkPreviewsUpdate {
            guild_id: None,
            channel_id: id(40),
            message_id: id(800),
            link_previews: Vec::new(),
        },
        GatewayEvent::ChannelCreate(_) => GatewayEvent::ChannelCreate(channel()),
        GatewayEvent::ChannelUpdate(_) => GatewayEvent::ChannelUpdate(sparse_channel()),
        GatewayEvent::ChannelDelete { .. } => GatewayEvent::ChannelDelete {
            guild_id: id(1),
            id: id(30),
        },
        GatewayEvent::GuildCreate(_) => GatewayEvent::GuildCreate(Box::new(guild())),
        GatewayEvent::GuildUpdate(_) => GatewayEvent::GuildUpdate(GuildSummary {
            id: id(1),
            name: "Mellow".into(),
            icon: Some(picture()),
        }),
        GatewayEvent::GuildUpdate(_) => GatewayEvent::GuildUpdate(GuildSummary {
            id: id(1),
            name: "Mellow".into(),
            icon: None,
        }),
        GatewayEvent::GuildDelete { .. } => GatewayEvent::GuildDelete { id: id(1) },
        GatewayEvent::GuildMemberAdd { .. } => GatewayEvent::GuildMemberAdd {
            guild_id: id(1),
            member: member(),
        },
        GatewayEvent::GuildMemberRemove { .. } => GatewayEvent::GuildMemberRemove {
            guild_id: id(1),
            user_id: id(10),
        },
        GatewayEvent::GuildMemberUpdate { .. } => GatewayEvent::GuildMemberUpdate {
            guild_id: id(1),
            user_id: id(10),
            role_ids: vec![id(20)],
        },
        GatewayEvent::GuildRolesUpdate { .. } => GatewayEvent::GuildRolesUpdate {
            guild_id: id(1),
            roles: vec![role()],
        },
        GatewayEvent::GuildEmojisUpdate { .. } => GatewayEvent::GuildEmojisUpdate {
            guild_id: id(1),
            emojis: vec![custom_emoji()],
        },
        GatewayEvent::GuildSoundsUpdate { .. } => GatewayEvent::GuildSoundsUpdate {
            guild_id: id(1),
            sounds: vec![sound(), sparse_sound()],
        },
        GatewayEvent::MyJoinSoundUpdate { .. } => GatewayEvent::MyJoinSoundUpdate {
            guild_id: id(1),
            sound_id: Some(id(600)),
        },
        GatewayEvent::MyJoinSoundUpdate { .. } => GatewayEvent::MyJoinSoundUpdate {
            guild_id: id(1),
            sound_id: None,
        },
        GatewayEvent::MessageReactionAdd(_) =>
            GatewayEvent::MessageReactionAdd(message_reaction()),
        GatewayEvent::MessageReactionRemove(_) =>
            GatewayEvent::MessageReactionRemove(sparse_message_reaction()),
        GatewayEvent::MessageReactionRemoveAll { .. } => GatewayEvent::MessageReactionRemoveAll {
            guild_id: Some(id(1)),
            channel_id: id(30),
            message_id: id(800),
        },
        GatewayEvent::MessageReactionRemoveAll { .. } => GatewayEvent::MessageReactionRemoveAll {
            guild_id: None,
            channel_id: id(40),
            message_id: id(800),
        },
        GatewayEvent::MessageReactionRemoveEmoji { .. } =>
            GatewayEvent::MessageReactionRemoveEmoji {
                guild_id: Some(id(1)),
                channel_id: id(30),
                message_id: id(800),
                emoji: "👍".into(),
            },
        GatewayEvent::MessageReactionRemoveEmoji { .. } =>
            GatewayEvent::MessageReactionRemoveEmoji {
                guild_id: None,
                channel_id: id(40),
                message_id: id(800),
                emoji: "👍".into(),
            },
        GatewayEvent::TypingStart { .. } => GatewayEvent::TypingStart {
            guild_id: Some(id(1)),
            channel_id: id(30),
            user_id: id(10),
        },
        GatewayEvent::TypingStart { .. } => GatewayEvent::TypingStart {
            guild_id: None,
            channel_id: id(40),
            user_id: id(10),
        },
        GatewayEvent::PresenceUpdate(_) => GatewayEvent::PresenceUpdate(presence()),
        GatewayEvent::DirectMessageChannelCreate(_) =>
            GatewayEvent::DirectMessageChannelCreate(direct_message_channel()),
        GatewayEvent::DirectMessageChannelUpdate(_) =>
            GatewayEvent::DirectMessageChannelUpdate(sparse_direct_message_channel()),
        GatewayEvent::DirectMessageChannelDelete { .. } =>
            GatewayEvent::DirectMessageChannelDelete { id: id(40) },
        GatewayEvent::RelationshipAdd(_) => GatewayEvent::RelationshipAdd(relationship()),
        GatewayEvent::RelationshipRemove { .. } =>
            GatewayEvent::RelationshipRemove { user_id: id(10) },
        GatewayEvent::UserUpdate(_) => GatewayEvent::UserUpdate(public_user()),
        GatewayEvent::VoiceStateUpdate { .. } => GatewayEvent::VoiceStateUpdate {
            voice_state: voice_state(),
        },
        GatewayEvent::VoiceStateDelete { .. } => GatewayEvent::VoiceStateDelete {
            guild_id: id(1),
            channel_id: id(30),
            user_id: id(10),
        },
        GatewayEvent::VoiceSessionDescription { .. } => GatewayEvent::VoiceSessionDescription {
            channel_id: id(30),
            offer_id: 7,
            sdp_answer: "v=0".into(),
        },
        GatewayEvent::VoiceJoinRefused { .. } => GatewayEvent::VoiceJoinRefused {
            channel_id: id(30),
            offer_id: 7,
            reason: VoiceJoinRefusal::ChannelFull,
        },
        GatewayEvent::VoiceSlotAssigned { .. } => GatewayEvent::VoiceSlotAssigned {
            channel_id: id(30),
            mid: "1".into(),
            user_id: id(10),
        },
        GatewayEvent::VoiceVideoRefused { .. } => GatewayEvent::VoiceVideoRefused {
            channel_id: id(30),
            reason: VoiceVideoRefusal::TooManyCameras,
        },
        GatewayEvent::VoiceVideoLayers { .. } => GatewayEvent::VoiceVideoLayers {
            channel_id: id(30),
            active: vec![SimulcastLayer::Quarter, SimulcastLayer::High],
        },
        GatewayEvent::VoiceVideoPaused { .. } => GatewayEvent::VoiceVideoPaused {
            channel_id: id(30),
            user_id: id(1),
            is_paused: true,
        },
        GatewayEvent::VoiceDisconnected { .. } => GatewayEvent::VoiceDisconnected {
            channel_id: id(30),
            reason: VoiceDisconnectReason::MovedElsewhere,
        },
        GatewayEvent::Speaking { .. } => GatewayEvent::Speaking {
            guild_id: id(1),
            channel_id: id(30),
            user_id: id(10),
            speaking: true,
        },
        GatewayEvent::VoiceSoundPlayed { .. } => GatewayEvent::VoiceSoundPlayed {
            channel_id: id(30),
            user_id: id(10),
            sound_id: id(600),
        },
    })
}

fn server_frames() -> Vec<ServerFrame> {
    every_variant!(ServerFrame {
        ServerFrame::Hello { .. } => ServerFrame::Hello {
            d: Hello {
                heartbeat_interval_milliseconds: 41_250,
            },
        },
        ServerFrame::Ready { .. } => ServerFrame::Ready {
            d: Box::new(ready()),
        },
        ServerFrame::Resumed => ServerFrame::Resumed,
        ServerFrame::InvalidSession => ServerFrame::InvalidSession,
        ServerFrame::HeartbeatAck => ServerFrame::HeartbeatAck,
        ServerFrame::Dispatch { .. } => ServerFrame::Dispatch {
            sequence: 42,
            event: Box::new(GatewayEvent::GuildDelete { id: id(1) }),
        },
    })
}

fn client_frames() -> Vec<ClientFrame> {
    every_variant!(ClientFrame {
        ClientFrame::Identify { .. } => ClientFrame::Identify {
            token: "mlw_token".into(),
        },
        ClientFrame::Resume { .. } => ClientFrame::Resume {
            token: "mlw_token".into(),
            session_id: "session-1".into(),
            last_sequence: 42,
        },
        ClientFrame::Heartbeat => ClientFrame::Heartbeat,
        ClientFrame::VoiceJoin { .. } => ClientFrame::VoiceJoin {
            channel_id: id(30),
            offer_id: 7,
            sdp_offer: "v=0".into(),
            self_mute: false,
            self_deaf: false,
        },
        ClientFrame::VoiceLeave => ClientFrame::VoiceLeave,
        ClientFrame::VoiceState { .. } => ClientFrame::VoiceState {
            self_mute: true,
            self_deaf: true,
            self_video: true,
            self_stream: true,
        },
        ClientFrame::VoiceVideoWatch { .. } => ClientFrame::VoiceVideoWatch {
            cameras: vec![
                WatchedCamera {
                    user_id: id(10),
                    size: VideoTileSize::Large,
                },
                WatchedCamera {
                    user_id: id(11),
                    size: VideoTileSize::Small,
                },
            ],
        },
        ClientFrame::VoicePlaySound { .. } => ClientFrame::VoicePlaySound {
            sound_id: id(600),
        },
    })
}

/// Every type `protocol.ts` declares for the wire, in its order.
#[allow(clippy::too_many_lines)] // One flat list: the samples of each type.
fn sampled_types() -> serde_json::Result<Vec<SampledType>> {
    use TypeScriptShape::{Interface, StringUnion, TaggedUnion};
    Ok(vec![
        sampled("ProfileImage", Interface, &[picture()])?,
        sampled(
            "GifSettings",
            Interface,
            &[GifSettings {
                klipy_app_key: "app-key".into(),
                customer_id: "00112233445566778899aabbccddeeff".into(),
            }],
        )?,
        sampled(
            "CurrentUser",
            Interface,
            &[current_user(), sparse_current_user()],
        )?,
        sampled(
            "ChosenStatus",
            StringUnion,
            &every_variant!(ChosenStatus: Online, Idle, DoNotDisturb, Invisible),
        )?,
        sampled(
            "PresenceStatus",
            StringUnion,
            &every_variant!(PresenceStatus: Online, Idle, DoNotDisturb, Offline),
        )?,
        sampled("Presence", Interface, &[presence(), sparse_presence()])?,
        sampled(
            "PublicUser",
            Interface,
            &[public_user(), sparse_public_user()],
        )?,
        sampled("CustomEmoji", Interface, &[custom_emoji()])?,
        sampled("SoundboardSound", Interface, &[sound(), sparse_sound()])?,
        sampled(
            "SessionCreated",
            Interface,
            &[SessionCreated {
                token: "mlw_token".into(),
                user: current_user(),
            }],
        )?,
        sampled(
            "ChannelKind",
            StringUnion,
            &every_variant!(ChannelKind: Category, Text, Voice),
        )?,
        sampled(
            "PermissionOverwrite",
            Interface,
            &every_variant!(OverwriteKind: Role, Member)
                .into_iter()
                .map(|kind| PermissionOverwrite {
                    kind,
                    ..overwrite()
                })
                .collect::<Vec<_>>(),
        )?,
        sampled("Channel", Interface, &[channel(), sparse_channel()])?,
        sampled(
            "ChannelPlacement",
            Interface,
            &[
                ChannelPlacement {
                    id: id(30),
                    parent_id: Some(id(31)),
                },
                ChannelPlacement {
                    id: id(31),
                    parent_id: None,
                },
            ],
        )?,
        sampled(
            "InitialCategory",
            Interface,
            &[InitialCategory {
                name: "Text channels".into(),
                channels: vec![InitialTextChannel {
                    name: "general".into(),
                    topic: "Say hello".into(),
                }],
            }],
        )?,
        sampled("Role", Interface, &[role()])?,
        sampled("Guild", Interface, &[guild(), sparse_guild()])?,
        sampled("Member", Interface, &[member()])?,
        sampled(
            "Attachment",
            Interface,
            &[attachment(), sparse_attachment()],
        )?,
        sampled("Message", Interface, &[message(), sparse_message()])?,
        sampled("ReactionSummary", Interface, &[reaction_summary()])?,
        sampled(
            "MessageReaction",
            Interface,
            &[message_reaction(), sparse_message_reaction()],
        )?,
        sampled("ReadState", Interface, &[read_state()])?,
        sampled("ChannelUnread", Interface, &[channel_unread()])?,
        sampled(
            "MessageSearchSort",
            StringUnion,
            &every_variant!(MessageSearchSort: Relevance, Newest),
        )?,
        sampled(
            "MessageSearchQuery",
            Interface,
            &[MessageSearchQuery {
                q: "hello from:ada".into(),
                sort: Some(MessageSearchSort::Newest),
                offset: Some(25),
                limit: Some(25),
                utc_offset_minutes: Some(-300),
            }],
        )?,
        sampled(
            "MessageSearchResults",
            Interface,
            &[message_search_results()],
        )?,
        sampled(
            "DirectMessageChannel",
            Interface,
            &[direct_message_channel(), sparse_direct_message_channel()],
        )?,
        sampled(
            "Invite",
            Interface,
            &[
                invite(),
                Invite {
                    max_uses: None,
                    expires_at_unix_milliseconds: None,
                    ..invite()
                },
            ],
        )?,
        sampled(
            "GuildInvite",
            Interface,
            &[
                guild_invite(),
                GuildInvite {
                    max_uses: None,
                    expires_at_unix_milliseconds: None,
                    ..guild_invite()
                },
            ],
        )?,
        sampled(
            "Ban",
            Interface,
            &[Ban {
                user: public_user(),
                reason: String::new(),
                created_at_unix_milliseconds: 1_700_000_000_000,
            }],
        )?,
        sampled(
            "AuditAction",
            StringUnion,
            &every_variant!(AuditAction:
                ChannelCreate, ChannelUpdate, ChannelDelete, OverwriteSet, OverwriteDelete,
                RoleCreate, RoleUpdate, RoleDelete, RolesReorder, ChannelsReorder, MemberRoleAdd,
                MemberRoleRemove, MemberKick, MemberBanAdd, MemberBanRemove, MessageDelete,
                MessagePin, MessageUnpin, MessageReactionRemove, GuildUpdate, InviteCreate,
                InviteDelete, EmojiCreate, EmojiUpdate, EmojiDelete, SoundCreate, SoundUpdate,
                SoundDelete, Unknown,
            ),
        )?,
        sampled(
            "AuditChange",
            Interface,
            &every_variant!(AuditChangeKey:
                Name, Topic, Color, Permissions, Allow, Deny, Code, Icon, Parent, UserLimit,
                Emoji, DisplayedSeparately, Mentionable, Unknown,
            )
            .into_iter()
            .map(|key| AuditChange {
                key,
                old_value: "old".into(),
                new_value: "new".into(),
            })
            .collect::<Vec<_>>(),
        )?,
        sampled(
            "AuditLogEntry",
            Interface,
            &[audit_log_entry(), sparse_audit_log_entry()],
        )?,
        sampled("Ready", Interface, &[ready(), sparse_ready()])?,
        sampled(
            "VoiceState",
            Interface,
            &[voice_state(), sparse_voice_state()],
        )?,
        sampled(
            "VoiceJoinRefusal",
            StringUnion,
            &every_variant!(VoiceJoinRefusal:
                MissingPermissions, ChannelFull, InstanceFull, InvalidOffer, VoiceUnavailable,
                RateLimited,
            ),
        )?,
        sampled(
            "VoiceVideoRefusal",
            StringUnion,
            &every_variant!(VoiceVideoRefusal:
                MissingPermissions, TooManyCameras, VideoUnavailable,
            ),
        )?,
        sampled(
            "VideoTileSize",
            StringUnion,
            &every_variant!(VideoTileSize: Large, Medium, Small),
        )?,
        sampled(
            "SimulcastLayer",
            StringUnion,
            &every_variant!(SimulcastLayer: High, Low, Quarter),
        )?,
        sampled(
            "VoiceDisconnectReason",
            StringUnion,
            &every_variant!(VoiceDisconnectReason:
                MovedElsewhere, PermissionsChanged, ChannelDeleted, RemovedFromGuild, MediaTimeout,
            ),
        )?,
        sampled(
            "RelationshipKind",
            StringUnion,
            &every_variant!(RelationshipKind: Friend, IncomingRequest, OutgoingRequest, Blocked),
        )?,
        sampled("Relationship", Interface, &[relationship()])?,
        sampled(
            "ApiErrorCode",
            StringUnion,
            &every_variant!(ApiErrorCode:
                ValidationFailed, UsernameTaken, EmailTaken, RegistrationClosed,
                InvalidCredentials, Unauthorized, AccountDisabled, RateLimited, ServerBusy,
                NotFound, MissingPermissions, InvalidInvite, OwnerCannotLeave, Banned,
                LimitReached, Conflict, Internal,
            ),
        )?,
        sampled("GatewayEvent", TaggedUnion("t"), &gateway_events())?,
        sampled("ServerFrame", TaggedUnion("op"), &server_frames())?,
        sampled("ClientFrame", TaggedUnion("op"), &client_frames())?,
    ])
}

/// The constants `protocol.ts` mirrors, under its names.
fn shared_constants() -> Value {
    json!({
        "MAXIMUM_COUNTED_UNREAD_MENTIONS": MAXIMUM_COUNTED_UNREAD_MENTIONS,
        "MAXIMUM_GIF_FAVOURITES": MAXIMUM_GIF_FAVOURITES,
        "MAXIMUM_POLL_QUESTION_LENGTH": MAXIMUM_POLL_QUESTION_LENGTH,
        "MINIMUM_POLL_ANSWERS": MINIMUM_POLL_ANSWERS,
        "MAXIMUM_POLL_ANSWERS": MAXIMUM_POLL_ANSWERS,
        "MAXIMUM_POLL_ANSWER_LENGTH": MAXIMUM_POLL_ANSWER_LENGTH,
        "MAXIMUM_GROUP_CONVERSATION_NAME_LENGTH": MAXIMUM_GROUP_CONVERSATION_NAME_LENGTH,
        "MAXIMUM_GROUP_CONVERSATION_PARTICIPANTS": MAXIMUM_GROUP_CONVERSATION_PARTICIPANTS,
        "MAXIMUM_REACHABLE_SEARCH_RESULTS": MAXIMUM_REACHABLE_SEARCH_RESULTS,
        "MAXIMUM_SEARCH_QUERY_LENGTH": MAXIMUM_SEARCH_QUERY_LENGTH,
        "MAXIMUM_SEARCH_QUERY_PARTS": MAXIMUM_SEARCH_QUERY_PARTS,
        "MAXIMUM_SEARCH_RESULTS_PER_PAGE": MAXIMUM_SEARCH_RESULTS_PER_PAGE,
        "MAXIMUM_VOICE_USER_LIMIT": MAXIMUM_VOICE_CHANNEL_PARTICIPANTS,
        "MINIMUM_MILLISECONDS_BETWEEN_SOUNDS": MINIMUM_MILLISECONDS_BETWEEN_SOUNDS,
        "VOICE_RECEIVE_SLOTS": MAXIMUM_VOICE_RECEIVE_SLOTS,
        "VIDEO_RECEIVE_SLOTS": MAXIMUM_VIDEO_RECEIVE_SLOTS,
        "SCREEN_SOUND_RECEIVE_SLOTS": MAXIMUM_SCREEN_SOUND_RECEIVE_SLOTS,
    })
}

/// Every named permission bit, and the channel-scoped set, as decimal strings.
fn permission_bits() -> Value {
    let named: serde_json::Map<String, Value> = Permissions::all()
        .iter_names()
        .map(|(name, bit)| (name.to_owned(), Value::String(bit.bits().to_string())))
        .collect();
    json!({
        "named": named,
        "channelScoped": Permissions::CHANNEL_SCOPED.bits().to_string(),
    })
}

/// `ChannelKind` → `channelKind`.
fn lower_camel_case(type_name: &str) -> String {
    let mut characters = type_name.chars();
    characters.next().map_or_else(String::new, |first| {
        first.to_ascii_lowercase().to_string() + characters.as_str()
    })
}

fn render(sampled_types: &[SampledType]) -> serde_json::Result<String> {
    let mut type_names: Vec<&str> = sampled_types
        .iter()
        .map(|sampled| sampled.type_name)
        .collect();
    type_names.sort_unstable();
    let mut sections = vec![format!(
        "// Generated by crates/mellow-protocol/tests/typescript_wire_samples.rs: do not edit.\n\
         // Regenerate: {UPDATE_VARIABLE}=1 cargo test -p mellow-protocol --test typescript_wire_samples\n\
         // `npm run check` type-checks every sample against protocol.ts; protocol.test.ts compares\n\
         // the constants and permission bits.\n\n\
         import type {{ {} }} from \"./protocol\";\n\n\
         /** `true` only when every member of `Union` has a sample. */\n\
         type HasEveryMember<Union, Sampled> = [Exclude<Union, Sampled>] extends [never] ? true : false;\n",
        type_names.join(", "),
    )];
    for sampled in sampled_types {
        let name = lower_camel_case(sampled.type_name);
        let type_name = sampled.type_name;
        let samples = serde_json::to_string_pretty(&sampled.samples)?;
        sections.push(format!(
            "\nexport const {name}Samples = {samples} satisfies {type_name}[];\n"
        ));
        let (union_members, sampled_members) = match sampled.shape {
            TypeScriptShape::Interface => continue,
            TypeScriptShape::StringUnion => (
                type_name.to_owned(),
                format!("(typeof {name}Samples)[number]"),
            ),
            TypeScriptShape::TaggedUnion(tag) => (
                format!("{type_name}[\"{tag}\"]"),
                format!("(typeof {name}Samples)[number][\"{tag}\"]"),
            ),
        };
        sections.push(format!(
            "export const {name}SamplesHaveEveryMember: HasEveryMember<{union_members}, {sampled_members}> = true;\n"
        ));
    }
    sections.push(format!(
        "\nexport const sharedConstants = {} as const;\n\nexport const permissionBits = {} as const;\n",
        serde_json::to_string_pretty(&shared_constants())?,
        serde_json::to_string_pretty(&permission_bits())?,
    ));
    Ok(sections.concat())
}

#[test]
fn web_wire_samples_are_up_to_date() {
    let generated = render(&sampled_types().unwrap()).unwrap();
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(GENERATED_FILE_PATH);
    if std::env::var_os(UPDATE_VARIABLE).is_some() {
        std::fs::write(&path, &generated).unwrap();
        return;
    }
    let committed = std::fs::read_to_string(&path).unwrap_or_default();
    assert!(
        committed == generated,
        "{} is stale: run `{UPDATE_VARIABLE}=1 cargo test -p mellow-protocol --test \
         typescript_wire_samples`, then `npm run check` in web/ and fix protocol.ts",
        path.display(),
    );
}
