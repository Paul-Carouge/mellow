//! Argon2id password hashing with OWASP parameters (19 MiB, t=2, p=1), run off the async
//! executor because each hash takes tens of milliseconds of CPU.

use std::num::NonZeroUsize;
use std::sync::OnceLock;
use std::time::Duration;

use argon2::Argon2;
use argon2::password_hash::{PasswordHasher as _, PasswordVerifier as _};
use tokio::sync::Semaphore;
use zeroize::Zeroizing;

/// How long a request waits for a hashing slot before giving up, instead of piling up.
const MAXIMUM_WAIT_FOR_HASHING_SLOT: Duration = Duration::from_secs(5);
const FALLBACK_HASHING_SLOTS: usize = 2;

#[derive(Debug, thiserror::Error)]
pub enum PasswordHashError {
    #[error("password hashing failed")]
    Hashing(String),
    #[error("the hashing task was cancelled")]
    TaskFailed(#[from] tokio::task::JoinError),
    #[error("too many password checks are running; try again shortly")]
    Busy,
}

/// A valid hash of a random password nobody knows, verified when a login names an unknown user
/// so that response time does not reveal which usernames exist.
const UNKNOWN_USER_PASSWORD_HASH: &str = "$argon2id$v=19$m=19456,t=2,p=1$bWVsbG93LXVua25vd24tdXNlcg$7Z2x9iC1k8tlwNTJkYqvy2JcR5qKyIhAyGDDJvDF6Gg";

/// One slot per CPU core: each hash uses 19 MiB and a core, so unbounded concurrency (up to
/// tokio's 512 blocking threads) could exhaust memory under a login flood.
fn hashing_slots() -> &'static Semaphore {
    static SLOTS: OnceLock<Semaphore> = OnceLock::new();
    SLOTS.get_or_init(|| {
        Semaphore::new(
            std::thread::available_parallelism().map_or(FALLBACK_HASHING_SLOTS, NonZeroUsize::get),
        )
    })
}

async fn run_with_hashing_slot<T: Send + 'static>(
    work: impl FnOnce() -> T + Send + 'static,
) -> Result<T, PasswordHashError> {
    let _slot = tokio::time::timeout(MAXIMUM_WAIT_FOR_HASHING_SLOT, hashing_slots().acquire())
        .await
        .map_err(|_| PasswordHashError::Busy)?
        .map_err(|_| PasswordHashError::Busy)?;
    Ok(tokio::task::spawn_blocking(work).await?)
}

/// Returns a PHC string (`$argon2id$v=19$...`) with a fresh random salt.
///
/// # Errors
/// Fails if the random generator or the hashing task fails, or no slot frees up in time.
pub async fn hash_password(password: Zeroizing<String>) -> Result<String, PasswordHashError> {
    run_with_hashing_slot(move || {
        Argon2::default()
            .hash_password(password.as_bytes())
            .map(|hash| hash.to_string())
            .map_err(|error| PasswordHashError::Hashing(error.to_string()))
    })
    .await?
}

/// `stored_hash: None` (unknown user) still spends the same work, then returns false.
///
/// # Errors
/// Fails if the hashing task fails or no slot frees up in time; a malformed stored hash counts
/// as a mismatch.
pub async fn verify_password(
    password: Zeroizing<String>,
    stored_hash: Option<String>,
) -> Result<bool, PasswordHashError> {
    run_with_hashing_slot(move || {
        let is_known_user = stored_hash.is_some();
        let hash = stored_hash.unwrap_or_else(|| UNKNOWN_USER_PASSWORD_HASH.to_owned());
        let matches = Argon2::default()
            .verify_password(password.as_bytes(), hash.as_str())
            .is_ok();
        is_known_user && matches
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    fn secret(text: &str) -> Zeroizing<String> {
        Zeroizing::new(text.to_owned())
    }

    #[tokio::test]
    async fn correct_password_verifies_and_wrong_one_does_not() {
        let hash = hash_password(secret("correct horse battery"))
            .await
            .unwrap();
        assert!(hash.starts_with("$argon2id$v=19$m=19456,t=2,p=1$"));
        assert!(
            verify_password(secret("correct horse battery"), Some(hash.clone()))
                .await
                .unwrap()
        );
        assert!(
            !verify_password(secret("wrong horse battery"), Some(hash))
                .await
                .unwrap()
        );
    }

    #[tokio::test]
    async fn unknown_user_never_verifies_but_still_parses_the_decoy_hash() {
        assert!(
            argon2::password_hash::phc::PasswordHash::new(UNKNOWN_USER_PASSWORD_HASH).is_ok(),
            "the decoy must be a real hash, or unknown users would answer faster"
        );
        assert!(
            !verify_password(secret("anything at all"), None)
                .await
                .unwrap()
        );
    }
}
