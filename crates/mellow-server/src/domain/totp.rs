//! Time-based one-time passwords (RFC 6238, ADR-0049): HMAC-SHA-1, 6 digits, 30-second steps,
//! one step of clock drift either way; and the recovery codes that replace a lost device.

use aws_lc_rs::constant_time::verify_slices_are_equal;
use aws_lc_rs::hmac;
use sha2::{Digest as _, Sha256};
use zeroize::Zeroizing;

/// 160 bits, the HMAC-SHA-1 block-size recommendation of RFC 4226 §4.
pub(crate) const TOTP_SECRET_BYTES: usize = 20;
pub(crate) const TOTP_DIGITS: usize = 6;
pub(crate) const TOTP_STEP_SECONDS: u64 = 30;
/// Steps accepted on each side of the current one, for phones whose clock drifts.
const ACCEPTED_CLOCK_DRIFT_STEPS: u64 = 1;
const TOTP_MODULUS: u32 = 1_000_000;

pub(crate) const RECOVERY_CODE_COUNT: usize = 10;
/// 80 random bits per code: too many to guess online or to brute-force from a stolen hash, so a
/// fast hash is enough, as for session tokens (ADR-0049).
const RECOVERY_CODE_RANDOM_BYTES: usize = 10;
const RECOVERY_CODE_CHARACTERS: usize = RECOVERY_CODE_RANDOM_BYTES * 8 / 5;
const RECOVERY_CODE_GROUP_LENGTH: usize = 4;

/// RFC 4648 base32, which authenticator apps read in `otpauth://` URIs.
const BASE32_ALPHABET: &[u8; 32] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";

pub(crate) type TotpSecret = Zeroizing<[u8; TOTP_SECRET_BYTES]>;
/// What the database stores of a recovery code.
pub(crate) type RecoveryCodeSha256 = [u8; 32];

/// # Errors
/// Fails only if the operating system cannot provide randomness.
pub(crate) fn generate_totp_secret() -> Result<TotpSecret, getrandom::Error> {
    let mut secret = Zeroizing::new([0u8; TOTP_SECRET_BYTES]);
    getrandom::fill(secret.as_mut())?;
    Ok(secret)
}

/// Unpadded RFC 4648 base32 (a 20-byte secret is exactly 32 characters).
pub(crate) fn base32(bytes: &[u8]) -> Zeroizing<String> {
    let mut text = Zeroizing::new(String::with_capacity(bytes.len().div_ceil(5) * 8));
    let mut buffer: u32 = 0;
    let mut buffered_bits = 0;
    for &byte in bytes {
        buffer = (buffer << 8) | u32::from(byte);
        buffered_bits += 8;
        while buffered_bits >= 5 {
            buffered_bits -= 5;
            text.push(char::from(
                BASE32_ALPHABET[((buffer >> buffered_bits) & 0x1F) as usize],
            ));
        }
    }
    if buffered_bits > 0 {
        text.push(char::from(
            BASE32_ALPHABET[((buffer << (5 - buffered_bits)) & 0x1F) as usize],
        ));
    }
    text
}

/// The time step a Unix time falls in.
pub(crate) fn time_step(now_unix_milliseconds: u64) -> u64 {
    now_unix_milliseconds / 1_000 / TOTP_STEP_SECONDS
}

/// The 6 ASCII digits of RFC 4226 §5.3 for one step.
pub(crate) fn code_at_time_step(secret: &[u8], step: u64) -> Zeroizing<[u8; TOTP_DIGITS]> {
    let key = hmac::Key::new(hmac::HMAC_SHA1_FOR_LEGACY_USE_ONLY, secret);
    let tag = hmac::sign(&key, &step.to_be_bytes());
    // HMAC-SHA-1 tags are 20 bytes: the offset is at most 15, so offset + 3 is in range.
    let digest = tag.as_ref();
    let offset = usize::from(digest[digest.len() - 1] & 0x0F);
    let truncated = u32::from_be_bytes([
        digest[offset] & 0x7F,
        digest[offset + 1],
        digest[offset + 2],
        digest[offset + 3],
    ]);
    let mut value = truncated % TOTP_MODULUS;
    let mut digits = Zeroizing::new([b'0'; TOTP_DIGITS]);
    for digit in digits.iter_mut().rev() {
        // Below 10, so the cast never truncates.
        *digit = b'0' + (value % 10) as u8;
        value /= 10;
    }
    digits
}

/// Six digits, spaces ignored ("123 456" as apps show it). `None` for anything else.
pub(crate) fn normalize_totp_code(typed: &str) -> Option<Zeroizing<[u8; TOTP_DIGITS]>> {
    let mut digits = Zeroizing::new([0u8; TOTP_DIGITS]);
    let mut count = 0;
    for byte in typed.bytes().filter(|byte| *byte != b' ') {
        if !byte.is_ascii_digit() || count == TOTP_DIGITS {
            return None;
        }
        digits[count] = byte;
        count += 1;
    }
    (count == TOTP_DIGITS).then_some(digits)
}

/// The newest step within the drift window whose code is `code`, or `None`. Every candidate is
/// computed and compared in constant time, so the answer's timing says nothing about which digits
/// matched.
pub(crate) fn matching_time_step(
    secret: &[u8],
    code: [u8; TOTP_DIGITS],
    now_unix_milliseconds: u64,
) -> Option<u64> {
    let current = time_step(now_unix_milliseconds);
    let first = current.saturating_sub(ACCEPTED_CLOCK_DRIFT_STEPS);
    let last = current.saturating_add(ACCEPTED_CLOCK_DRIFT_STEPS);
    (first..=last)
        .filter(|step| {
            verify_slices_are_equal(code_at_time_step(secret, *step).as_ref(), &code).is_ok()
        })
        .fold(None, |_, step| Some(step))
}

/// Ten fresh codes, as shown once to the user (`abcd-efgh-ijkl-mnop`).
///
/// # Errors
/// Fails only if the operating system cannot provide randomness.
pub(crate) fn generate_recovery_codes() -> Result<Vec<Zeroizing<String>>, getrandom::Error> {
    (0..RECOVERY_CODE_COUNT)
        .map(|_| {
            let mut random_bytes = Zeroizing::new([0u8; RECOVERY_CODE_RANDOM_BYTES]);
            getrandom::fill(random_bytes.as_mut())?;
            let characters = base32(random_bytes.as_ref()).to_ascii_lowercase();
            let mut code = Zeroizing::new(String::with_capacity(RECOVERY_CODE_CHARACTERS + 3));
            for (index, character) in characters.chars().enumerate() {
                if index > 0 && index % RECOVERY_CODE_GROUP_LENGTH == 0 {
                    code.push('-');
                }
                code.push(character);
            }
            Ok(code)
        })
        .collect()
}

/// The stored hash of a recovery code as typed: case, dashes and spaces do not count. `None` when
/// it cannot be a recovery code.
pub(crate) fn recovery_code_sha256(typed: &str) -> Option<RecoveryCodeSha256> {
    let normalized: Zeroizing<String> = Zeroizing::new(
        typed
            .chars()
            .filter(|character| !matches!(character, '-' | ' '))
            .map(|character| character.to_ascii_uppercase())
            .collect(),
    );
    let is_well_formed = normalized.len() == RECOVERY_CODE_CHARACTERS
        && normalized
            .bytes()
            .all(|byte| BASE32_ALPHABET.contains(&byte));
    is_well_formed.then(|| Sha256::digest(normalized.as_bytes()).into())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The SHA-1 seed of RFC 6238 Appendix B.
    const RFC_6238_SECRET: &[u8] = b"12345678901234567890";

    #[test]
    fn codes_match_the_rfc_6238_test_vectors() {
        // Appendix B lists 8 digits; the last 6 are the 6-digit code.
        for (unix_seconds, eight_digits) in [
            (59_u64, "94287082"),
            (1_111_111_109, "07081804"),
            (1_111_111_111, "14050471"),
            (1_234_567_890, "89005924"),
            (2_000_000_000, "69279037"),
            (20_000_000_000, "65353130"),
        ] {
            let step = time_step(unix_seconds * 1_000);
            assert_eq!(
                code_at_time_step(RFC_6238_SECRET, step).as_ref(),
                &eight_digits.as_bytes()[2..],
                "{unix_seconds}"
            );
        }
    }

    #[test]
    fn a_code_is_accepted_one_step_either_side_and_no_further() {
        let now = 1_111_111_111_000;
        let current = time_step(now);
        for (step, is_accepted) in [
            (current - 2, false),
            (current - 1, true),
            (current, true),
            (current + 1, true),
            (current + 2, false),
        ] {
            let code = code_at_time_step(RFC_6238_SECRET, step);
            assert_eq!(
                matching_time_step(RFC_6238_SECRET, *code, now),
                is_accepted.then_some(step),
                "{step}"
            );
        }
    }

    #[test]
    fn typed_codes_are_six_digits_with_optional_spaces() {
        assert_eq!(normalize_totp_code("123 456").as_deref(), Some(b"123456"));
        for typed in ["12345", "1234567", "12345a", "", "１２３４５６"] {
            assert!(normalize_totp_code(typed).is_none(), "{typed}");
        }
    }

    #[test]
    fn base32_follows_rfc_4648() {
        for (bytes, text) in [
            (&b""[..], ""),
            (b"f", "MY"),
            (b"fo", "MZXQ"),
            (b"foo", "MZXW6"),
            (b"foob", "MZXW6YQ"),
            (b"fooba", "MZXW6YTB"),
            (b"foobar", "MZXW6YTBOI"),
        ] {
            assert_eq!(base32(bytes).as_str(), text);
        }
        assert_eq!(base32(&[0xFF; TOTP_SECRET_BYTES]).len(), 32);
    }

    #[test]
    fn recovery_codes_are_unique_and_hash_however_they_are_typed() {
        let codes = generate_recovery_codes().unwrap();
        assert_eq!(codes.len(), RECOVERY_CODE_COUNT);
        let hashes: std::collections::HashSet<_> = codes
            .iter()
            .map(|code| recovery_code_sha256(code).unwrap())
            .collect();
        assert_eq!(hashes.len(), RECOVERY_CODE_COUNT);
        let code = codes[0].as_str();
        assert_eq!(code.len(), 19, "{code}");
        assert_eq!(
            recovery_code_sha256(&code.to_uppercase().replace('-', " ")),
            recovery_code_sha256(code)
        );
        for typed in [
            "",
            "123456",
            &code[1..],
            &format!("{code}a"),
            "0000-0000-0000-0000",
        ] {
            assert!(recovery_code_sha256(typed).is_none(), "{typed}");
        }
    }
}
