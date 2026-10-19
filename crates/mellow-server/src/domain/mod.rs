//! Use cases and invariants. Knows nothing about HTTP; storage is reached through
//! `crate::storage` functions (ARCHITECTURE.md §4.1).

pub mod account_rules;
pub mod accounts;
pub mod clock;
pub(crate) mod guild_rules;
pub(crate) mod guilds;
pub(crate) mod password;
pub(crate) mod permissions;
pub(crate) mod session_token;
pub mod snowflake_generator;
