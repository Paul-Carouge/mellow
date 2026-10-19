//! Opaque session tokens (ARCHITECTURE.md §4.7): 256 random bits, shown to the client once,
//! stored only as SHA-256.

use sha2::{Digest as _, Sha256};
use zeroize::Zeroizing;

/// Lets secret scanners and humans recognise a leaked Mellow token.
const SESSION_TOKEN_PREFIX: &str = "mlw_";
const SESSION_TOKEN_RANDOM_BYTES: usize = 32;
const SESSION_TOKEN_LENGTH: usize = SESSION_TOKEN_PREFIX.len() + SESSION_TOKEN_RANDOM_BYTES * 2;

/// What the database stores and looks up.
pub type SessionTokenSha256 = [u8; 32];

/// # Errors
/// Fails only if the operating system cannot provide randomness.
pub fn generate_session_token() -> Result<Zeroizing<String>, getrandom::Error> {
    let mut random_bytes = Zeroizing::new([0u8; SESSION_TOKEN_RANDOM_BYTES]);
    getrandom::fill(random_bytes.as_mut())?;
    let mut token = Zeroizing::new(String::with_capacity(SESSION_TOKEN_LENGTH));
    token.push_str(SESSION_TOKEN_PREFIX);
    token.extend(
        random_bytes
            .iter()
            .flat_map(|byte| [byte >> 4, byte & 0x0F])
            // A nibble is always below 16, so `from_digit` never drops one.
            .filter_map(|nibble| char::from_digit(u32::from(nibble), 16)),
    );
    Ok(token)
}

/// Returns the lookup hash if `token` has the shape of a Mellow session token, so malformed
/// headers are rejected without touching the database.
#[must_use]
pub fn session_token_sha256(token: &str) -> Option<SessionTokenSha256> {
    let random_part = token.strip_prefix(SESSION_TOKEN_PREFIX)?;
    let is_well_formed = token.len() == SESSION_TOKEN_LENGTH
        && random_part
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte));
    is_well_formed.then(|| Sha256::digest(token.as_bytes()).into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_tokens_are_unique_well_formed_and_hash_deterministically() {
        let first = generate_session_token().unwrap();
        let second = generate_session_token().unwrap();
        assert_ne!(first.as_str(), second.as_str());
        assert_eq!(first.len(), SESSION_TOKEN_LENGTH);
        assert_eq!(session_token_sha256(&first), session_token_sha256(&first));
        assert_ne!(session_token_sha256(&first), session_token_sha256(&second));
    }

    #[test]
    fn malformed_tokens_have_no_hash() {
        let valid = generate_session_token().unwrap();
        for token in [
            "",
            "mlw_",
            &valid[1..],
            &valid.to_uppercase(),
            &format!("{}0", valid.as_str()),
            &valid.replacen("mlw_", "xyz_", 1),
        ] {
            assert_eq!(session_token_sha256(token), None, "{token}");
        }
    }
}
