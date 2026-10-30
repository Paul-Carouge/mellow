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
    ClientFrame, GATEWAY_VERSION, GatewayEvent, Hello, MessageReaction, Presence, PresenceStatus,
    Ready, ServerFrame,
};
pub use guilds::{
    AuditAction, AuditChange, AuditChangeKey, AuditLogEntry, AuditLogQuery, Ban, Channel,
    ChannelKind, ChannelPlacement, CreateBanRequest, CreateChannelRequest, CreateEmojiQuery,
    CreateGuildRequest, CreateInviteRequest, CreateRoleRequest, CustomEmoji, Guild, GuildInvite,
    GuildSummary, IconCropQuery, InitialCategory, InitialTextChannel, Invite, Member,
    OverwriteKind, PermissionOverwrite, ProfileImage, PublicUser, ReorderChannelsRequest,
    ReorderRolesRequest, Role, SetPermissionOverwriteRequest, UpdateChannelRequest,
    UpdateEmojiRequest, UpdateGuildRequest, UpdateRoleRequest,
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
pub use snowflake::{
    MAXIMUM_SEQUENCE, MAXIMUM_TIMESTAMP_MILLISECONDS_SINCE_EPOCH, MELLOW_EPOCH_UNIX_MILLISECONDS,
    Snowflake, SnowflakeError,
};
