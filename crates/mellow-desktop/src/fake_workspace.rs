//! Deterministic fake servers, channels and members for the desktop shell, until the client core exists.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FakePresence {
    Online,
    Idle,
    DoNotDisturb,
    Offline,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FakeServer {
    pub name: &'static str,
    pub unread_mention_count: u32,
    pub has_unread_messages: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FakeChannel {
    pub name: &'static str,
    pub topic: &'static str,
    pub has_unread_messages: bool,
    pub unread_mention_count: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FakeCategory {
    pub name: &'static str,
    pub channels: Vec<FakeChannel>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FakeMember {
    pub display_name: &'static str,
    pub activity: &'static str,
    pub presence: FakePresence,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FakeRoleGroup {
    pub role_name: &'static str,
    /// `0xRRGGBB`, as roles carry it; `None` for groups that are not roles.
    pub role_color: Option<u32>,
    pub members: Vec<FakeMember>,
}

pub fn fake_servers() -> Vec<FakeServer> {
    [
        ("Mellow Dev", 0, true),
        ("Rustaceans FR", 3, true),
        ("Board Games Night", 0, false),
        ("Climbing Crew", 1, true),
        ("Home Lab", 0, false),
    ]
    .into_iter()
    .map(
        |(name, unread_mention_count, has_unread_messages)| FakeServer {
            name,
            unread_mention_count,
            has_unread_messages,
        },
    )
    .collect()
}

pub fn fake_categories() -> Vec<FakeCategory> {
    let channel = |name, topic, has_unread_messages, unread_mention_count| FakeChannel {
        name,
        topic,
        has_unread_messages,
        unread_mention_count,
    };
    vec![
        FakeCategory {
            name: "Information",
            channels: vec![
                channel("welcome", "Read this first", false, 0),
                channel("announcements", "Releases and news", true, 0),
            ],
        },
        FakeCategory {
            name: "Text channels",
            channels: vec![
                channel("general", "Anything goes, be kind", true, 2),
                channel("dev", "Server, client core and desktop", true, 0),
                channel("design", "Tokens, screens and accessibility", false, 0),
                channel("off-topic", "Memes welcome", false, 0),
            ],
        },
        FakeCategory {
            name: "Support",
            channels: vec![channel("self-hosting", "Install, update, backup", false, 0)],
        },
    ]
}

/// The reference mockup's moderator green.
const MAINTAINER_ROLE_COLOR: u32 = 0x4F_B3_9A;

pub fn fake_role_groups() -> Vec<FakeRoleGroup> {
    let member = |display_name, activity, presence| FakeMember {
        display_name,
        activity,
        presence,
    };
    vec![
        FakeRoleGroup {
            role_name: "Maintainers",
            role_color: Some(MAINTAINER_ROLE_COLOR),
            members: vec![
                member("Aurore", "Reviewing PR #2", FakePresence::Online),
                member("Basile", "", FakePresence::DoNotDisturb),
            ],
        },
        FakeRoleGroup {
            role_name: "Contributors",
            role_color: None,
            members: vec![
                member("Chloé", "Playing Factorio", FakePresence::Online),
                member("Dimitri", "", FakePresence::Idle),
                member("Élise", "Listening to lo-fi", FakePresence::Online),
            ],
        },
        FakeRoleGroup {
            role_name: "Offline",
            role_color: None,
            members: vec![
                member("Fanny", "", FakePresence::Offline),
                member("Gaspard", "", FakePresence::Offline),
            ],
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn workspace_has_a_default_general_channel_and_an_offline_group_last() {
        let has_general_channel = fake_categories()
            .iter()
            .flat_map(|category| &category.channels)
            .any(|channel| channel.name == "general");
        assert!(has_general_channel);

        let role_groups = fake_role_groups();
        let offline_group = role_groups.last().unwrap();
        assert!(
            offline_group
                .members
                .iter()
                .all(|member| member.presence == FakePresence::Offline)
        );
    }
}
