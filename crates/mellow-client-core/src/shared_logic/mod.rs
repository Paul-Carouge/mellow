//! Pure rules the web client applies to text, search, notifications and lists, ported to Rust so
//! a native client reads messages, queries and settings exactly as the web does (ADR-0051).
//!
//! Each module names the web file it ports. Positions are byte offsets into UTF-8 text where the
//! web counts UTF-16 units; words shown to people stay out: functions return structured values
//! the client's own catalogue puts into words, except dates and numbers, which take a [`format::Locale`].

pub mod channel_order;
pub mod channel_settings;
pub mod code_blocks;
pub mod context_menus_people;
pub mod context_menus_places;
pub mod custom_emoji;
pub mod emoji_catalogue;
pub mod emoji_picker;
pub mod emoji_suggestions;
pub mod format;
pub mod forward_messages;
pub mod friends;
pub mod group_conversations;
pub mod klipy;
mod linguist_colours;
pub mod link_previews;
pub mod markdown;
pub mod member_order;
pub mod mentions;
pub mod message_length;
pub mod message_rows;
pub mod message_search;
pub mod notification_settings;
pub mod notifications;
pub mod poll_form;
pub mod poll_view;
pub mod profiles;
pub mod qr_code;
pub mod quick_switcher;
pub mod role_colors;
pub mod role_mentions;
pub mod search_highlights;
pub mod search_query;
pub mod search_results;
pub mod search_suggestions;
pub mod search_text;
pub mod server_settings;
pub mod slash_commands;
pub mod soundboard;
pub mod tracker_form;
mod unicode_classes;
pub mod user_settings;
