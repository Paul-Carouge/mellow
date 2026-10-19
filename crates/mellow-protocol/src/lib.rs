//! Wire types shared by the Mellow server and clients. No I/O lives here (ADR-0001).

mod canonical_decimal;
mod health;
mod permissions;
mod snowflake;

pub use health::{HealthResponse, HealthStatus};
pub use permissions::Permissions;
pub use snowflake::{
    MAXIMUM_SEQUENCE, MAXIMUM_TIMESTAMP_MILLISECONDS_SINCE_EPOCH, MELLOW_EPOCH_UNIX_MILLISECONDS,
    Snowflake, SnowflakeError,
};
