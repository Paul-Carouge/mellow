/// Number of digits in `u64::MAX`; anything longer cannot be a canonical `u64`.
const MAXIMUM_U64_DECIMAL_DIGITS: usize = 20;

/// Parses the one canonical decimal spelling of a `u64`: ASCII digits only, no sign, no leading
/// zero, so every value has exactly one string form on the wire.
pub(crate) fn parse_canonical_decimal_u64(text: &str) -> Option<u64> {
    let has_only_digits = !text.is_empty() && text.bytes().all(|byte| byte.is_ascii_digit());
    let has_leading_zero = text.len() > 1 && text.starts_with('0');
    if !has_only_digits || has_leading_zero || text.len() > MAXIMUM_U64_DECIMAL_DIGITS {
        return None;
    }
    text.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonical_values_parse() {
        assert_eq!(parse_canonical_decimal_u64("0"), Some(0));
        assert_eq!(parse_canonical_decimal_u64("42"), Some(42));
        assert_eq!(
            parse_canonical_decimal_u64("18446744073709551615"),
            Some(u64::MAX)
        );
    }

    #[test]
    fn non_canonical_values_are_rejected() {
        for text in [
            "",
            "+1",
            "-1",
            " 1",
            "1 ",
            "1.0",
            "0x10",
            "01",
            "00",
            "١",
            "18446744073709551616",
            "000000000000000000001",
        ] {
            assert_eq!(parse_canonical_decimal_u64(text), None, "{text:?}");
        }
    }
}
