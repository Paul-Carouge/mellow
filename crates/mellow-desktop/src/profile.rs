//! Separate profiles: `MELLOW_PROFILE=<name>` gives the app its own secret store entry,
//! preferences, drafts, picture cache and single-instance lock, so a development or test build can
//! run signed in to another instance without touching the everyday session.

use std::borrow::Cow;
use std::sync::OnceLock;

const PROFILE_VARIABLE: &str = "MELLOW_PROFILE";
/// Keeps macOS's `~/Library/Application Support/mellow-<profile>/instance.socket` under the
/// 104 bytes of `sun_path` for ordinary account names.
const MAXIMUM_PROFILE_NAME_LENGTH: usize = 16;

/// The profile named by `MELLOW_PROFILE`, or `None` for the everyday one. A name that is empty or
/// not made of lowercase ASCII letters, digits and dashes ends the process, never silently mapped
/// onto the everyday profile, whose session it would expose.
#[must_use]
pub fn active_profile_name() -> Option<&'static str> {
    static PROFILE: OnceLock<Option<String>> = OnceLock::new();
    PROFILE
        .get_or_init(|| {
            let value = std::env::var_os(PROFILE_VARIABLE)?;
            let Some(name) = value.to_str().and_then(validated_profile_name) else {
                tracing::error!(
                    "{PROFILE_VARIABLE} must be 1 to {MAXIMUM_PROFILE_NAME_LENGTH} lowercase \
                     letters, digits or dashes"
                );
                std::process::exit(2);
            };
            Some(name.to_owned())
        })
        .as_deref()
}

fn validated_profile_name(value: &str) -> Option<&str> {
    let is_valid = !value.is_empty()
        && value.len() <= MAXIMUM_PROFILE_NAME_LENGTH
        && value
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-');
    is_valid.then_some(value)
}

/// `base` for the everyday profile, `base-<profile>` otherwise.
#[must_use]
pub fn with_profile_suffix(base: &'static str) -> Cow<'static, str> {
    suffixed(base, active_profile_name(), '-')
}

/// `base` for the everyday profile, `base.<profile>` otherwise (reverse-DNS names).
#[must_use]
pub fn with_profile_dotted_suffix(base: &'static str) -> Cow<'static, str> {
    suffixed(base, active_profile_name(), '.')
}

/// The app id the portals, the tray and the compositor know this profile by: `base` for the
/// everyday profile, `base.p_<profile>` otherwise, since an app id element may not start with a
/// digit nor hold a dash.
#[must_use]
pub fn profile_app_identifier(base: &'static str) -> Cow<'static, str> {
    app_identifier(base, active_profile_name())
}

fn app_identifier(base: &'static str, profile: Option<&str>) -> Cow<'static, str> {
    match profile {
        None => Cow::Borrowed(base),
        Some(profile) => Cow::Owned(format!("{base}.p_{}", profile.replace('-', "_"))),
    }
}

fn suffixed(base: &'static str, profile: Option<&str>, separator: char) -> Cow<'static, str> {
    match profile {
        None => Cow::Borrowed(base),
        Some(profile) => Cow::Owned(format!("{base}{separator}{profile}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_lowercase_letters_digits_and_dashes_name_a_profile() {
        assert_eq!(validated_profile_name("test-1"), Some("test-1"));
        assert_eq!(validated_profile_name(""), None);
        assert_eq!(validated_profile_name("Test"), None);
        assert_eq!(validated_profile_name("../evil"), None);
        assert_eq!(validated_profile_name("a b"), None);
        assert_eq!(
            validated_profile_name(&"a".repeat(16)),
            Some("aaaaaaaaaaaaaaaa")
        );
        assert_eq!(validated_profile_name(&"a".repeat(17)), None);
    }

    #[cfg(unix)]
    #[test]
    fn a_name_that_is_not_unicode_is_refused_rather_than_ignored() {
        use std::os::unix::ffi::OsStrExt as _;
        let value = std::ffi::OsStr::from_bytes(b"test\xff");
        assert_eq!(value.to_str().and_then(validated_profile_name), None);
    }

    #[test]
    fn app_identifiers_stay_valid_for_every_profile_name() {
        assert_eq!(
            app_identifier("org.mellow.desktop", None),
            "org.mellow.desktop"
        );
        assert_eq!(
            app_identifier("org.mellow.desktop", Some("2-test")),
            "org.mellow.desktop.p_2_test"
        );
    }

    #[test]
    fn the_everyday_profile_keeps_the_base_names() {
        assert_eq!(suffixed("mellow", None, '-'), "mellow");
        assert_eq!(suffixed("mellow", Some("test"), '-'), "mellow-test");
        assert_eq!(
            suffixed("org.mellow.desktop", Some("test"), '.'),
            "org.mellow.desktop.test"
        );
    }
}
