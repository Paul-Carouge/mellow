//! Wire types shared by the Mellow server and clients. No I/O lives here (ADR-0001).

mod accounts;
mod canonical_decimal;
mod gateway;
mod guilds;
mod health;
mod messages;
mod permissions;
mod snowflake;

pub use accounts::{
    ApiErrorBody, ApiErrorCode, CurrentUser, LoginRequest, RegisterRequest, SessionCreated,
};
pub use gateway::{
    ClientFrame, GATEWAY_VERSION, GatewayEvent, Hello, PresenceStatus, Ready, ServerFrame,
};
pub use guilds::{
    Channel, ChannelKind, CreateChannelRequest, CreateGuildRequest, CreateInviteRequest,
    CreateRoleRequest, Guild, GuildSummary, Invite, Member, OverwriteKind, PermissionOverwrite,
    PublicUser, ReorderRolesRequest, Role, SetPermissionOverwriteRequest, UpdateChannelRequest,
    UpdateRoleRequest,
};
pub use health::{HealthResponse, HealthStatus};
pub use messages::{
    AcknowledgeRequest, EditMessageRequest, Message, MessageHistoryQuery, ReadState,
    SendMessageRequest,
};
pub use permissions::{
    Permissions, compute_channel_permissions, compute_guild_permissions, highest_role_position,
};
pub use snowflake::{
    MAXIMUM_SEQUENCE, MAXIMUM_TIMESTAMP_MILLISECONDS_SINCE_EPOCH, MELLOW_EPOCH_UNIX_MILLISECONDS,
    Snowflake, SnowflakeError,
};
