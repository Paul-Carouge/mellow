//! Wire types shared by the Mellow server and clients. No I/O lives here (ADR-0001).

mod accounts;
mod canonical_decimal;
mod direct_messages;
mod gateway;
mod guilds;
mod health;
mod messages;
mod permissions;
mod relationships;
mod search;
mod snowflake;

pub use accounts::{
    ApiErrorBody, ApiErrorCode, ChangePasswordRequest, ChosenStatus, CurrentUser, LoginRequest,
    RegisterRequest, SessionCreated, UpdateCurrentUserRequest, UpdatePresenceRequest,
};
pub use direct_messages::{
    CreateGroupConversationRequest, DirectMessageChannel, MAXIMUM_GROUP_CONVERSATION_NAME_LENGTH,
    MAXIMUM_GROUP_CONVERSATION_PARTICIPANTS, OpenDirectMessageRequest,
    UpdateGroupConversationRequest,
};
pub use gateway::{
    ClientFrame, GATEWAY_VERSION, GatewayEvent, Hello, MAXIMUM_VOICE_OFFER_BYTES,
    MAXIMUM_VOICE_RECEIVE_SLOTS, MessageReaction, Presence, PresenceStatus, Ready, ServerFrame,
    VoiceDisconnectReason, VoiceJoinRefusal, VoiceState,
};
pub use guilds::{
    AuditAction, AuditChange, AuditChangeKey, AuditLogEntry, AuditLogQuery, Ban, Channel,
    ChannelKind, ChannelPlacement, CreateBanRequest, CreateChannelRequest, CreateEmojiQuery,
    CreateGuildRequest, CreateInviteRequest, CreateRoleRequest, CustomEmoji, Guild, GuildInvite,
    GuildSummary, IconCropQuery, InitialCategory, InitialTextChannel, Invite,
    MAXIMUM_VOICE_CHANNEL_PARTICIPANTS, Member, OverwriteKind, PermissionOverwrite, ProfileImage,
    PublicUser, ReorderChannelsRequest, ReorderRolesRequest, Role, SetPermissionOverwriteRequest,
    UpdateChannelRequest, UpdateEmojiRequest, UpdateGuildRequest, UpdateRoleRequest,
    UserLimitChange,
};
pub use health::{HealthResponse, HealthStatus};
pub use messages::{
    AcknowledgeRequest, Attachment, ChannelUnread, EditMessageRequest,
    MAXIMUM_COUNTED_UNREAD_MENTIONS, Message, MessageHistoryQuery, ReactionSummary,
    ReactionUsersQuery, ReadState, SendMessageRequest,
};
pub use permissions::{
    Permissions, compute_channel_permissions, compute_guild_permissions, highest_role_position,
};
pub use relationships::{
    MAXIMUM_RELATIONSHIPS_PER_USER, Relationship, RelationshipKind, SendFriendRequest,
    SetRelationshipKind, SetRelationshipRequest,
};
pub use search::{
    AuthorSearchFacet, ChannelSearchFacet, HighlightRange, MAXIMUM_COUNTED_SEARCH_MATCHES,
    MAXIMUM_REACHABLE_SEARCH_RESULTS, MAXIMUM_SEARCH_FACETS, MAXIMUM_SEARCH_QUERY_LENGTH,
    MAXIMUM_SEARCH_QUERY_PARTS, MAXIMUM_SEARCH_RESULTS_PER_PAGE, MessageSearchHit,
    MessageSearchQuery, MessageSearchResults, MessageSearchSort,
};
pub use snowflake::{
    MAXIMUM_SEQUENCE, MAXIMUM_TIMESTAMP_MILLISECONDS_SINCE_EPOCH, MELLOW_EPOCH_UNIX_MILLISECONDS,
    Snowflake, SnowflakeError,
};
