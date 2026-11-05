//! Use cases and invariants. Knows nothing about HTTP; storage is reached through
//! `crate::storage` functions (ARCHITECTURE.md §4.1).

pub mod account_rules;
pub mod accounts;
pub(crate) mod attachments;
pub(crate) mod audio_transcoding;
pub mod clock;
pub(crate) mod gifs;
pub(crate) mod guild_rules;
pub(crate) mod guilds;
pub(crate) mod image_processing;
pub(crate) mod image_recompression;
pub(crate) mod link_previews;
pub(crate) mod media_headers;
pub(crate) mod mentions;
pub(crate) mod message_rules;
pub(crate) mod messages;
pub(crate) mod password;
pub(crate) mod permissions;
pub(crate) mod profile_images;
pub(crate) mod relationships;
pub(crate) mod search_query;
pub(crate) mod session_token;
pub mod snowflake_generator;
pub(crate) mod sound_rules;
pub(crate) mod totp;
pub(crate) mod two_factor;
pub(crate) mod video_rules;
pub(crate) mod voice_rules;
