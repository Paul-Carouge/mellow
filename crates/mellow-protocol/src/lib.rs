//! Wire types shared by the Mellow server and clients. No I/O lives here (ADR-0001).

mod accounts;
mod canonical_decimal;
mod direct_messages;
mod gateway;
mod guilds;
mod health;
mod messages;
mod permissions;
mod snowflake;

pub use accounts::{
    ApiErrorBody, ApiErrorCode, ChangePasswordRequest, CurrentUser, LoginRequest, RegisterRequest,
    SessionCreated, UpdateCurrentUserRequest,
};
pub use direct_messages::{DirectMessageChannel, OpenDirectMessageRequest};
pub use gateway::{
    ClientFrame, GATEWAY_VERSION, GatewayEvent, Hello, MessageReaction, PresenceStatus, Ready,
    ServerFrame,
};
pub use guilds::{
    AuditAction, AuditChange, AuditChangeKey, AuditLogEntry, AuditLogQuery, Ban, Channel,
    ChannelKind, CreateBanRequest, CreateChannelRequest, CreateGuildRequest, CreateInviteRequest,
    CreateRoleRequest, Guild, GuildInvite, GuildSummary, IconCropQuery, Invite, Member,
    OverwriteKind, PermissionOverwrite, ProfileImage, PublicUser, ReorderRolesRequest, Role,
    SetPermissionOverwriteRequest, UpdateChannelRequest, UpdateGuildRequest, UpdateRoleRequest,
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
pub use snowflake::{
    MAXIMUM_SEQUENCE, MAXIMUM_TIMESTAMP_MILLISECONDS_SINCE_EPOCH, MELLOW_EPOCH_UNIX_MILLISECONDS,
    Snowflake, SnowflakeError,
};
