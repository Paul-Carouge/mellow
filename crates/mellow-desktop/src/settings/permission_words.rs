//! The words of the permission editors (`serverSettings.permissions`, `serverSettings.roles.groups`
//! and the channel words of `channelSettings.permissions.channelWords`).

use mellow_client_core::shared_logic::server_settings::{PermissionGroupKey, PermissionKey};

use crate::i18n::Strings;

/// A permission's name and the sentence under it, as the role editor says them.
#[must_use]
pub fn permission_words(key: PermissionKey, catalogue: Strings) -> (&'static str, &'static str) {
    let words = catalogue.server_settings().permissions();
    macro_rules! pair {
        ($accessor:ident) => {{
            let permission = words.$accessor();
            (permission.label(), permission.description())
        }};
    }
    match key {
        PermissionKey::ViewChannels => pair!(view_channels),
        PermissionKey::ManageChannels => pair!(manage_channels),
        PermissionKey::ManageRoles => pair!(manage_roles),
        PermissionKey::ManageServer => pair!(manage_server),
        PermissionKey::ManageExpressions => pair!(manage_expressions),
        PermissionKey::ViewModerationLog => pair!(view_moderation_log),
        PermissionKey::CreateInvite => pair!(create_invite),
        PermissionKey::SendMessages => pair!(send_messages),
        PermissionKey::ReadMessageHistory => pair!(read_message_history),
        PermissionKey::AttachFiles => pair!(attach_files),
        PermissionKey::AddReactions => pair!(add_reactions),
        PermissionKey::SendPolls => pair!(send_polls),
        PermissionKey::SendChecklists => pair!(send_checklists),
        PermissionKey::MentionEveryone => pair!(mention_everyone),
        PermissionKey::ManageMessages => pair!(manage_messages),
        PermissionKey::Connect => pair!(connect),
        PermissionKey::Speak => pair!(speak),
        PermissionKey::UseSoundboard => pair!(use_soundboard),
        PermissionKey::Stream => pair!(stream),
        PermissionKey::KickMembers => pair!(kick_members),
        PermissionKey::BanMembers => pair!(ban_members),
        PermissionKey::Administrator => pair!(administrator),
    }
}

/// In a channel's editor, the channel words where the server-wide ones would mislead ("View
/// channels" reads "View channel").
#[must_use]
pub fn channel_permission_words(
    key: PermissionKey,
    catalogue: Strings,
) -> (&'static str, &'static str) {
    let words = catalogue.channel_settings().permissions().channel_words();
    match key {
        PermissionKey::ViewChannels => {
            let permission = words.view_channels();
            (permission.label(), permission.description())
        }
        PermissionKey::ManageChannels => {
            let permission = words.manage_channels();
            (permission.label(), permission.description())
        }
        PermissionKey::ManageRoles => {
            let permission = words.manage_roles();
            (permission.label(), permission.description())
        }
        _ => permission_words(key, catalogue),
    }
}

#[must_use]
pub fn permission_group_label(group: PermissionGroupKey, catalogue: Strings) -> &'static str {
    let groups = catalogue.server_settings().roles().groups();
    match group {
        PermissionGroupKey::General => groups.general(),
        PermissionGroupKey::Membership => groups.membership(),
        PermissionGroupKey::TextChannels => groups.text_channels(),
        PermissionGroupKey::VoiceChannels => groups.voice_channels(),
        PermissionGroupKey::Moderation => groups.moderation(),
        PermissionGroupKey::Advanced => groups.advanced(),
    }
}
