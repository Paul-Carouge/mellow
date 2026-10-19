//! The signed-in session, kept in the OS secret store (Keychain, Credential Manager, Secret
//! Service) so the app does not ask for the password on every launch (ARCHITECTURE §4.7).
//!
//! Every function blocks on the platform store: call them from `spawn_blocking`, never from the
//! UI thread.

use std::sync::{Mutex, MutexGuard, PoisonError};

use mellow_client_core::InstanceAddress;
use zeroize::Zeroizing;

const SECRET_STORE_SERVICE: &str = "org.mellow.desktop";
/// shortcut: one entry, so one instance at a time; key entries by instance with multi-instance (M2).
const SECRET_STORE_ACCOUNT: &str = "session";
const INSTANCE_AND_TOKEN_SEPARATOR: char = '\n';
/// Loopback instances speak plain http; a release build never replays a token over it at launch,
/// where any local user could be listening on the port.
pub const SHOULD_STORE_LOOPBACK_SESSIONS: bool = cfg!(debug_assertions);

/// Store calls run on different blocking threads; serialized, the removal at sign-out cannot land
/// after the next sign-in's save and wipe it.
static SECRET_STORE_ACCESS: Mutex<()> = Mutex::new(());

fn lock_secret_store_access() -> MutexGuard<'static, ()> {
    // The guarded value is `()`: a panic while holding it leaves nothing inconsistent.
    SECRET_STORE_ACCESS
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
}

pub struct StoredSession {
    pub instance: InstanceAddress,
    pub token: Zeroizing<String>,
}

/// The token is never printed.
impl std::fmt::Debug for StoredSession {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("StoredSession")
            .field("instance", &self.instance)
            .field("token", &"<redacted>")
            .finish()
    }
}

fn secret_store_entry() -> keyring::Result<keyring_core::Entry> {
    // Installs the platform store as keyring-core's default on first use.
    if keyring::Entry::store_status().is_err() {
        return Err(keyring::Error::NoDefaultStore);
    }
    // The default "Enterprise" persistence would roam the token to every machine of a domain profile.
    #[cfg(windows)]
    let modifiers = std::collections::HashMap::from([("persistence", "Local")]);
    #[cfg(not(windows))]
    let modifiers = std::collections::HashMap::new();
    keyring_core::Entry::new_with_modifiers(SECRET_STORE_SERVICE, SECRET_STORE_ACCOUNT, &modifiers)
}

/// # Errors
/// The secret store is unavailable or refuses the write.
pub fn save_session(instance: &InstanceAddress, token: &str) -> keyring::Result<()> {
    let _access = lock_secret_store_access();
    secret_store_entry()?.set_password(&encode_session(instance, token))
}

/// `None` when nothing is stored, or when the entry cannot be read back as a session (it is then
/// removed, so the next sign-in starts clean).
///
/// # Errors
/// The secret store is unavailable or refuses the read.
pub fn load_session() -> keyring::Result<Option<StoredSession>> {
    let _access = lock_secret_store_access();
    let entry = secret_store_entry()?;
    let secret = match entry.get_password() {
        Ok(secret) => Zeroizing::new(secret),
        Err(keyring::Error::NoEntry) => return Ok(None),
        Err(error) => return Err(error),
    };
    let session = decode_session(&secret);
    if session.is_none() {
        tracing::warn!("discarding an unreadable stored session");
        forget_entry(&entry)?;
    }
    Ok(session)
}

/// # Errors
/// The secret store is unavailable or refuses the deletion.
pub fn forget_session() -> keyring::Result<()> {
    let _access = lock_secret_store_access();
    forget_entry(&secret_store_entry()?)
}

fn forget_entry(entry: &keyring_core::Entry) -> keyring::Result<()> {
    match entry.delete_credential() {
        Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
        Err(error) => Err(error),
    }
}

fn encode_session(instance: &InstanceAddress, token: &str) -> Zeroizing<String> {
    Zeroizing::new(format!(
        "{}{INSTANCE_AND_TOKEN_SEPARATOR}{token}",
        instance.as_str()
    ))
}

/// The store is outside our control, so its content is validated like any other input: the
/// address keeps the https rule and the token must be a plain header value.
fn decode_session(secret: &str) -> Option<StoredSession> {
    let (address, token) = secret.split_once(INSTANCE_AND_TOKEN_SEPARATOR)?;
    let is_token_valid = !token.is_empty() && token.bytes().all(|byte| byte.is_ascii_graphic());
    if !is_token_valid {
        return None;
    }
    let instance = InstanceAddress::parse(address).ok()?;
    if instance.is_loopback() && !SHOULD_STORE_LOOPBACK_SESSIONS {
        return None;
    }
    Some(StoredSession {
        instance,
        token: Zeroizing::new(token.to_owned()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_encoded_session_decodes_to_the_same_instance_and_token() {
        let instance = InstanceAddress::parse("chat.example.org").unwrap();
        let encoded = encode_session(&instance, "mlw_secret");
        let decoded = decode_session(&encoded).unwrap();
        assert_eq!(decoded.instance, instance);
        assert_eq!(decoded.token.as_str(), "mlw_secret");
    }

    #[test]
    fn tampered_or_corrupt_entries_are_rejected() {
        for secret in [
            "",
            "https://chat.example.org/",
            "https://chat.example.org/\n",
            "http://chat.example.org/\nmlw_secret",
            "https://chat.example.org/\nmlw secret",
            "https://chat.example.org/\nmlw_secret\r\nX-Injected: 1",
        ] {
            assert!(decode_session(secret).is_none(), "{secret:?}");
        }
    }

    #[test]
    fn debug_output_never_shows_the_token() {
        let session = StoredSession {
            instance: InstanceAddress::parse("chat.example.org").unwrap(),
            token: Zeroizing::new("mlw_secret".into()),
        };
        assert!(!format!("{session:?}").contains("mlw_secret"));
    }
}
