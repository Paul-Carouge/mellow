use crate::ClientError;

/// 128 bits: the server deduplicates sends on (channel, author, nonce) forever, so nonces must
/// never repeat across launches or devices.
const MESSAGE_NONCE_RANDOM_BYTES: usize = 16;

/// A fresh nonce for `ClientState::queue_message`, 32 lowercase hexadecimal characters.
///
/// # Errors
/// `RandomGeneratorFailed` when the operating system's random generator is unavailable.
pub fn generate_message_nonce() -> Result<String, ClientError> {
    let mut random_bytes = [0u8; MESSAGE_NONCE_RANDOM_BYTES];
    getrandom::fill(&mut random_bytes).map_err(ClientError::RandomGeneratorFailed)?;
    Ok(random_bytes
        .iter()
        .flat_map(|byte| [byte >> 4, byte & 0x0F])
        // A nibble is always below 16, so `from_digit` never drops one.
        .filter_map(|nibble| char::from_digit(u32::from(nibble), 16))
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nonces_are_hexadecimal_and_distinct() {
        let first = generate_message_nonce().unwrap();
        let second = generate_message_nonce().unwrap();
        assert_eq!(first.len(), MESSAGE_NONCE_RANDOM_BYTES * 2);
        assert!(first.bytes().all(|byte| byte.is_ascii_hexdigit()));
        assert_ne!(first, second);
    }
}
