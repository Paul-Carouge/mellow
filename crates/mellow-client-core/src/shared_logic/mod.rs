//! Pure rules the web client applies to text, search, notifications and lists, ported to Rust so
//! a native client reads messages, queries and settings exactly as the web does (ADR-0051).
//!
//! Each module names the web file it ports. Positions are byte offsets into UTF-8 text where the
//! web counts UTF-16 units; words shown to people stay out: functions return structured values
//! the client's own catalogue puts into words, except dates and numbers, which take a [`format::Locale`].

pub mod channel_order;
pub mod code_blocks;
pub mod custom_emoji;
pub mod emoji_catalogue;
pub mod emoji_suggestions;
pub mod format;
pub mod markdown;
pub mod member_order;
pub mod mentions;
pub mod message_length;
pub mod message_rows;
pub mod notification_settings;
pub mod notifications;
pub mod quick_switcher;
pub mod role_colors;
pub mod role_mentions;
pub mod search_query;
pub mod search_suggestions;
pub mod search_text;
pub mod soundboard;
mod unicode_classes;
