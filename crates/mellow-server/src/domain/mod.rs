//! Use cases and invariants. Knows nothing about HTTP; storage is reached through
//! `crate::storage` functions (ARCHITECTURE.md §4.1).

pub mod account_rules;
pub mod accounts;
pub(crate) mod attachments;
pub mod clock;
pub(crate) mod guild_rules;
pub(crate) mod guilds;
pub(crate) mod image_processing;
pub(crate) mod mentions;
pub(crate) mod message_rules;
pub(crate) mod messages;
pub(crate) mod password;
pub(crate) mod permissions;
pub(crate) mod profile_images;
pub(crate) mod session_token;
pub mod snowflake_generator;
