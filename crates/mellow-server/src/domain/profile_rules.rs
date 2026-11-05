//! What a profile may say (ADR-0050): pure checks of the "about me" text and the links.

use mellow_protocol::{
    MAXIMUM_ABOUT_ME_LENGTH, MAXIMUM_PROFILE_LINK_LABEL_LENGTH, MAXIMUM_PROFILE_LINK_URL_LENGTH,
    MAXIMUM_PROFILE_LINKS, NewProfileLink,
};
use url::{Host, Url};

use crate::domain::account_rules::is_invisible_format_character;
use crate::storage::user_profiles::NewStoredLink;

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub(crate) enum ProfileRuleViolation {
    #[error(
        "an \"about me\" text holds at most {MAXIMUM_ABOUT_ME_LENGTH} characters, without hidden ones"
    )]
    AboutMe,
    #[error("a profile holds at most {MAXIMUM_PROFILE_LINKS} links")]
    TooManyLinks,
    #[error(
        "a profile link is an https address of at most {MAXIMUM_PROFILE_LINK_URL_LENGTH} characters, on the default port, without credentials"
    )]
    LinkAddress,
    #[error(
        "a profile link's label holds at most {MAXIMUM_PROFILE_LINK_LABEL_LENGTH} characters, without hidden ones"
    )]
    LinkLabel,
}

/// Emoji joiners and presentation selectors stay allowed, as in custom statuses; tag characters,
/// which can hide text, do not.
fn has_hidden_characters(text: &str, is_line_break_allowed: bool) -> bool {
    text.chars().any(|character| {
        (character.is_control() && !(is_line_break_allowed && character == '\n'))
            || (is_invisible_format_character(character)
                && !matches!(character, '\u{200D}' | '\u{FE0E}' | '\u{FE0F}'))
    })
}

/// The text to store, trimmed, line breaks kept (`\r\n` read as one).
///
/// # Errors
/// `AboutMe` when too long or holding hidden characters.
pub(crate) fn normalize_about_me(text: &str) -> Result<String, ProfileRuleViolation> {
    let text = text.replace("\r\n", "\n");
    let trimmed = text.trim();
    if trimmed.chars().count() > MAXIMUM_ABOUT_ME_LENGTH || has_hidden_characters(trimmed, true) {
        return Err(ProfileRuleViolation::AboutMe);
    }
    Ok(trimmed.to_owned())
}

/// One link, checked: its address as the URL parser writes it back (hosts lowercased and in
/// punycode, so a look-alike name shows what it is), the host its icon is cached under.
fn normalize_link(link: &NewProfileLink) -> Result<NewStoredLink, ProfileRuleViolation> {
    let address = link.url.trim();
    if address.chars().count() > MAXIMUM_PROFILE_LINK_URL_LENGTH {
        return Err(ProfileRuleViolation::LinkAddress);
    }
    let url = Url::parse(address).map_err(|_| ProfileRuleViolation::LinkAddress)?;
    let host = match url.host() {
        Some(Host::Domain(domain)) => domain.trim_end_matches('.').to_ascii_lowercase(),
        Some(Host::Ipv4(_) | Host::Ipv6(_)) | None => {
            return Err(ProfileRuleViolation::LinkAddress);
        }
    };
    if url.scheme() != "https"
        || host.is_empty()
        || url.port().is_some()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.as_str().chars().count() > MAXIMUM_PROFILE_LINK_URL_LENGTH
    {
        return Err(ProfileRuleViolation::LinkAddress);
    }
    let label = match link.label.as_deref().map(str::trim) {
        None | Some("") => None,
        Some(label)
            if label.chars().count() <= MAXIMUM_PROFILE_LINK_LABEL_LENGTH
                && !has_hidden_characters(label, false) =>
        {
            Some(label.to_owned())
        }
        Some(_) => return Err(ProfileRuleViolation::LinkLabel),
    };
    Ok(NewStoredLink {
        url: url.into(),
        label,
        host,
    })
}

/// # Errors
/// `TooManyLinks`, or the first link's violation.
pub(crate) fn normalize_links(
    links: &[NewProfileLink],
) -> Result<Vec<NewStoredLink>, ProfileRuleViolation> {
    if links.len() > MAXIMUM_PROFILE_LINKS {
        return Err(ProfileRuleViolation::TooManyLinks);
    }
    links.iter().map(normalize_link).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn link(url: &str, label: Option<&str>) -> NewProfileLink {
        NewProfileLink {
            url: url.into(),
            label: label.map(str::to_owned),
        }
    }

    #[test]
    fn about_me_is_trimmed_keeps_lines_and_emoji_and_refuses_hidden_or_long_text() {
        assert_eq!(
            normalize_about_me("  Hi **there** 🧑‍💻\r\nsecond line \n").unwrap(),
            "Hi **there** 🧑‍💻\nsecond line"
        );
        assert_eq!(normalize_about_me("").unwrap(), "");
        assert_eq!(
            normalize_about_me(&"é".repeat(MAXIMUM_ABOUT_ME_LENGTH)).unwrap(),
            "é".repeat(MAXIMUM_ABOUT_ME_LENGTH)
        );
        for refused in [
            "é".repeat(MAXIMUM_ABOUT_ME_LENGTH + 1),
            "a\u{0007}b".into(),
            "a\u{200B}b".into(),
            "a\u{202E}b".into(),
            "a\tb".into(),
        ] {
            assert_eq!(
                normalize_about_me(&refused),
                Err(ProfileRuleViolation::AboutMe)
            );
        }
    }

    #[test]
    fn links_are_https_on_named_hosts_and_come_back_normalised() {
        let stored = normalize_links(&[
            link(" https://Example.ORG./Ada ", Some("  Blog ")),
            link("https://bücher.example/", Some("   ")),
        ])
        .unwrap();
        assert_eq!(stored[0].url, "https://example.org./Ada");
        assert_eq!(stored[0].host, "example.org");
        assert_eq!(stored[0].label.as_deref(), Some("Blog"));
        assert_eq!(stored[1].url, "https://xn--bcher-kva.example/");
        assert_eq!(stored[1].label, None);
        for refused in [
            "http://example.org/",
            "javascript:alert(1)",
            "https://user:secret@example.org/",
            "https://example.org:8443/",
            "https://127.0.0.1/",
            "https://[::1]/",
            "file:///etc/passwd",
            "example.org",
            "",
        ] {
            assert_eq!(
                normalize_links(&[link(refused, None)]),
                Err(ProfileRuleViolation::LinkAddress),
                "{refused}"
            );
        }
        let long_path = format!("https://example.org/{}", "a".repeat(300));
        assert_eq!(
            normalize_links(&[link(&long_path, None)]),
            Err(ProfileRuleViolation::LinkAddress)
        );
        assert_eq!(
            normalize_links(&[link("https://example.org/", Some(&"x".repeat(41)))]),
            Err(ProfileRuleViolation::LinkLabel)
        );
        assert_eq!(
            normalize_links(&vec![link("https://example.org/", None); 6]),
            Err(ProfileRuleViolation::TooManyLinks)
        );
    }
}
