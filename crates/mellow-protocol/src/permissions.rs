use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::canonical_decimal::parse_canonical_decimal_u64;

bitflags::bitflags! {
    /// Permission bitfield of a role, a channel overwrite or a computed member permission (ADR-0007).
    ///
    /// Bit positions are part of the wire protocol: never reorder or reuse them.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
    pub struct Permissions: u64 {
        const ADMINISTRATOR        = 1 << 0;
        const VIEW_CHANNEL         = 1 << 1;
        const SEND_MESSAGES        = 1 << 2;
        const READ_MESSAGE_HISTORY = 1 << 3;
        const ATTACH_FILES         = 1 << 4;
        const MENTION_EVERYONE     = 1 << 5;
        const MANAGE_MESSAGES      = 1 << 6;
        const MANAGE_CHANNELS      = 1 << 7;
        const MANAGE_ROLES         = 1 << 8;
        const MANAGE_GUILD         = 1 << 9;
        const CREATE_INVITE        = 1 << 10;
        const KICK_MEMBERS         = 1 << 11;
        const BAN_MEMBERS          = 1 << 12;
        const CHANGE_NICKNAME      = 1 << 13;
        const MANAGE_NICKNAMES     = 1 << 14;
        const VIEW_AUDIT_LOG       = 1 << 15;
    }
}

impl Permissions {
    /// Permissions granted to `@everyone` in a newly created guild.
    pub const DEFAULT_EVERYONE: Self = Self::VIEW_CHANNEL
        .union(Self::SEND_MESSAGES)
        .union(Self::READ_MESSAGE_HISTORY)
        .union(Self::ATTACH_FILES)
        .union(Self::CREATE_INVITE)
        .union(Self::CHANGE_NICKNAME);
}

/// Serialized as a decimal string of the raw bits, like ids, so JavaScript-style clients keep all 64 bits.
impl Serialize for Permissions {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(&self.bits())
    }
}

/// Unknown bits are kept, so an older client does not erase permissions added by a newer server.
/// The server must reject unknown bits in client requests (`Permissions::from_bits`), otherwise a
/// client could pre-grant permissions that a future version defines.
impl<'de> Deserialize<'de> for Permissions {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = <std::borrow::Cow<'de, str>>::deserialize(deserializer)?;
        parse_canonical_decimal_u64(&text)
            .map(Self::from_bits_retain)
            .ok_or_else(|| {
                serde::de::Error::custom("permissions must be a canonical decimal string")
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_uses_decimal_string_and_keeps_unknown_bits() {
        let permissions = Permissions::SEND_MESSAGES | Permissions::from_bits_retain(1 << 63);
        let json = serde_json::to_string(&permissions).unwrap();
        assert_eq!(json, "\"9223372036854775812\"");
        assert_eq!(
            serde_json::from_str::<Permissions>(&json).unwrap(),
            permissions
        );
    }

    #[test]
    fn non_decimal_strings_are_rejected() {
        for json in [
            "4",
            "\"+4\"",
            "\"-4\"",
            "\"\"",
            "\"04\"",
            "\"SEND_MESSAGES\"",
        ] {
            assert!(serde_json::from_str::<Permissions>(json).is_err(), "{json}");
        }
    }

    #[test]
    fn default_everyone_has_no_moderation_permission() {
        let moderation = Permissions::ADMINISTRATOR
            | Permissions::MANAGE_MESSAGES
            | Permissions::MANAGE_CHANNELS
            | Permissions::MANAGE_ROLES
            | Permissions::MANAGE_GUILD
            | Permissions::KICK_MEMBERS
            | Permissions::BAN_MEMBERS
            | Permissions::MANAGE_NICKNAMES
            | Permissions::VIEW_AUDIT_LOG;
        assert!(!Permissions::DEFAULT_EVERYONE.intersects(moderation));
    }
}
