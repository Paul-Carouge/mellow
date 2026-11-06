//! What the generated catalogue calls to speak numbers the way the web client's `Intl` does:
//! CLDR plural rules, `toLocaleString` grouping and rounding, and JavaScript's own `String(n)`.

use super::Language;

/// `Intl.NumberFormat` and `Intl.PluralRules`' default.
const DEFAULT_MAXIMUM_FRACTION_DIGITS: usize = 3;

/// `Intl.PluralRules(language).select(count) === "one"` (`web/src/i18n/plural.ts`). English: only
/// 1. French: 0 and 1 and everything between, so "0 pièce jointe".
#[must_use]
pub fn is_plural_one(language: Language, count: f64) -> bool {
    // ECMA-402 applies the CLDR rule to the number as formatted with at most three fraction
    // digits, so 1.0001 reads "1" (English singular) and 1.9996 reads "2" (French plural).
    let rounded = format_number(
        Language::English,
        count.abs(),
        0,
        DEFAULT_MAXIMUM_FRACTION_DIGITS,
    );
    let (integer, fraction) = rounded.split_once('.').unwrap_or((&rounded, ""));
    match language {
        // CLDR `i = 1 and v = 0`.
        Language::English => integer == "1" && fraction.is_empty(),
        // CLDR `i = 0,1`.
        Language::French => integer == "0" || integer == "1",
    }
}

/// `value.toLocaleString(language)`: grouped, at most three fraction digits.
#[must_use]
pub fn format_count(language: Language, value: f64) -> String {
    format_number(language, value, 0, DEFAULT_MAXIMUM_FRACTION_DIGITS)
}

/// `value.toLocaleString(language, { minimumFractionDigits, maximumFractionDigits })`: rounded
/// half away from zero on the shortest decimal form, as ICU does (1.0005 gives "1.001").
#[must_use]
pub fn format_number(
    language: Language,
    value: f64,
    minimum_fraction_digits: usize,
    maximum_fraction_digits: usize,
) -> String {
    let sign = if value.is_sign_negative() { "-" } else { "" };
    if value.is_nan() {
        return "NaN".to_owned();
    }
    if value.is_infinite() {
        return format!("{sign}∞");
    }
    // Rust's `Display` gives the shortest decimal digits that read back as `value`, never an
    // exponent: the same digits ICU rounds.
    let shortest = format!("{}", value.abs());
    let (integer, fraction) = shortest.split_once('.').unwrap_or((&shortest, ""));
    let mut digits: Vec<u8> = integer
        .bytes()
        .chain(fraction.bytes().take(maximum_fraction_digits))
        .map(|digit| digit - b'0')
        .collect();
    let mut integer_length = integer.len();
    let rounds_up = fraction
        .as_bytes()
        .get(maximum_fraction_digits)
        .is_some_and(|digit| *digit >= b'5');
    if rounds_up && round_up(&mut digits) {
        digits.insert(0, 1);
        integer_length += 1;
    }
    let (integer_digits, fraction_digits) = digits.split_at(integer_length);
    let mut fraction_digits = fraction_digits.to_vec();
    while fraction_digits.len() > minimum_fraction_digits && fraction_digits.last() == Some(&0) {
        fraction_digits.pop();
    }
    fraction_digits.resize(fraction_digits.len().max(minimum_fraction_digits), 0);

    let (group_separator, decimal_separator) = match language {
        Language::English => (",", "."),
        // CLDR French: a narrow no-break space between groups, a comma before the fraction.
        Language::French => ("\u{202f}", ","),
    };
    let mut text = String::from(sign);
    for (index, digit) in integer_digits.iter().enumerate() {
        if index > 0 && (integer_digits.len() - index) % 3 == 0 {
            text.push_str(group_separator);
        }
        text.push(char::from(b'0' + digit));
    }
    if !fraction_digits.is_empty() {
        text.push_str(decimal_separator);
        text.extend(fraction_digits.iter().map(|digit| char::from(b'0' + digit)));
    }
    text
}

/// Adds one unit in the last place of `digits`; true when it carries past the first digit.
fn round_up(digits: &mut [u8]) -> bool {
    for digit in digits.iter_mut().rev() {
        if *digit == 9 {
            *digit = 0;
        } else {
            *digit += 1;
            return false;
        }
    }
    true
}

/// JavaScript's `String(value)`: what a number becomes inside a template string.
#[must_use]
pub fn format_number_like_javascript(value: f64) -> String {
    /// From these magnitudes JavaScript switches to exponent notation.
    const LARGEST_PLAIN_NUMBER: f64 = 1e21;
    const SMALLEST_PLAIN_NUMBER: f64 = 1e-6;
    if value.is_nan() {
        return "NaN".to_owned();
    }
    if value.is_infinite() {
        return if value > 0.0 { "Infinity" } else { "-Infinity" }.to_owned();
    }
    if value == 0.0 {
        // -0 too.
        return "0".to_owned();
    }
    if (SMALLEST_PLAIN_NUMBER..LARGEST_PLAIN_NUMBER).contains(&value.abs()) {
        return format!("{value}");
    }
    let exponent_form = format!("{value:e}");
    match exponent_form.split_once('e') {
        Some((mantissa, exponent)) if !exponent.starts_with('-') => {
            format!("{mantissa}e+{exponent}")
        }
        _ => exponent_form,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Expected values are what Node's `Intl` gives.
    #[test]
    fn groups_and_rounds_like_intl() {
        let english = |value| format_count(Language::English, value);
        assert_eq!(english(0.0), "0");
        assert_eq!(english(-0.0), "-0");
        assert_eq!(english(1000.0), "1,000");
        assert_eq!(english(1_234_567.891), "1,234,567.891");
        assert_eq!(english(-1204.0), "-1,204");
        assert_eq!(english(0.0005), "0.001");
        assert_eq!(english(1.0005), "1.001");
        assert_eq!(english(999.9999), "1,000");
        assert_eq!(english(1e21), "1,000,000,000,000,000,000,000");
        assert_eq!(english(f64::NAN), "NaN");
        assert_eq!(english(f64::INFINITY), "∞");
        assert_eq!(
            format_count(Language::French, 1_234_567.891),
            "1\u{202f}234\u{202f}567,891"
        );
        assert_eq!(format_count(Language::French, 1000.0), "1\u{202f}000");
    }

    #[test]
    fn keeps_the_fraction_digits_asked_for() {
        assert_eq!(format_number(Language::English, 1.25, 1, 1), "1.3");
        assert_eq!(format_number(Language::English, 0.05, 1, 1), "0.1");
        assert_eq!(format_number(Language::French, 5.2, 1, 1), "5,2");
        assert_eq!(format_number(Language::French, 5.0, 1, 1), "5,0");
    }

    #[test]
    fn chooses_the_singular_by_each_languages_rule() {
        assert!(is_plural_one(Language::English, 1.0));
        assert!(is_plural_one(Language::English, -1.0));
        assert!(!is_plural_one(Language::English, 0.0));
        assert!(!is_plural_one(Language::English, 1.5));
        assert!(is_plural_one(Language::English, 1.0001));
        assert!(!is_plural_one(Language::English, 0.9994));
        assert!(!is_plural_one(Language::English, f64::INFINITY));
        assert!(!is_plural_one(Language::French, 1.9996));
        assert!(is_plural_one(Language::French, 1.9994));
        assert!(is_plural_one(Language::French, 0.0));
        assert!(is_plural_one(Language::French, 1.5));
        assert!(!is_plural_one(Language::French, 2.0));
        assert!(!is_plural_one(Language::French, 1_000_000.0));
        assert!(!is_plural_one(Language::French, f64::NAN));
    }

    #[test]
    fn writes_numbers_as_javascript_does() {
        assert_eq!(format_number_like_javascript(1023.0), "1023");
        assert_eq!(format_number_like_javascript(-0.0), "0");
        assert_eq!(format_number_like_javascript(0.5), "0.5");
        assert_eq!(format_number_like_javascript(1e21), "1e+21");
        assert_eq!(
            format_number_like_javascript(f64::NEG_INFINITY),
            "-Infinity"
        );
    }
}
