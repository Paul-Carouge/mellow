//! Rust names and literals for the generated catalogue.

/// Rust's strict and reserved keywords: a catalogue key spelled like one becomes a raw identifier.
const RUST_KEYWORDS: [&str; 51] = [
    "as", "async", "await", "break", "const", "continue", "crate", "dyn", "else", "enum", "extern",
    "false", "fn", "for", "if", "impl", "in", "let", "loop", "match", "mod", "move", "mut", "pub",
    "ref", "return", "self", "Self", "static", "struct", "super", "trait", "true", "type",
    "unsafe", "use", "where", "while", "abstract", "become", "box", "do", "final", "gen", "macro",
    "override", "priv", "typeof", "unsized", "virtual", "yield",
];

/// The words of a `camelCase`, `PascalCase`, `snake_case` or `SCREAMING_CASE` name.
fn words(name: &str) -> Vec<String> {
    let characters: Vec<char> = name.chars().collect();
    let mut words = Vec::new();
    let mut word = String::new();
    for (index, &character) in characters.iter().enumerate() {
        if !character.is_ascii_alphanumeric() {
            if !word.is_empty() {
                words.push(std::mem::take(&mut word));
            }
            continue;
        }
        let previous = index
            .checked_sub(1)
            .and_then(|before| characters.get(before));
        let next = characters.get(index + 1);
        let starts_a_word = character.is_ascii_uppercase()
            && previous.is_some_and(|previous| {
                previous.is_ascii_lowercase()
                    || previous.is_ascii_digit()
                    || (previous.is_ascii_uppercase() && next.is_some_and(char::is_ascii_lowercase))
            });
        if starts_a_word && !word.is_empty() {
            words.push(std::mem::take(&mut word));
        }
        word.push(character);
    }
    if !word.is_empty() {
        words.push(word);
    }
    words
}

/// `withMentions` → `with_mentions`, `NOT_FOUND` → `not_found`, `move` → `r#move`, and
/// `720p` → `key_720p` (a Rust name cannot start with a digit).
pub fn snake_case_identifier(name: &str) -> String {
    let snake = words(name)
        .iter()
        .map(|word| word.to_ascii_lowercase())
        .collect::<Vec<_>>()
        .join("_");
    if snake.starts_with(|character: char| character.is_ascii_digit()) {
        format!("key_{snake}")
    } else if RUST_KEYWORDS.contains(&snake.as_str()) {
        format!("r#{snake}")
    } else {
        snake
    }
}

/// `errors.reasons` → `ErrorsReasons`, `KB` → `Kb`.
pub fn pascal_case(name: &str) -> String {
    words(name)
        .iter()
        .map(|word| {
            let mut characters = word.chars();
            characters.next().map_or_else(String::new, |first| {
                first.to_ascii_uppercase().to_string() + &characters.as_str().to_ascii_lowercase()
            })
        })
        .collect()
}

/// A Rust string literal; invisible characters (no-break spaces, controls) are escaped so the
/// generated file shows them.
pub fn rust_string_literal(text: &str) -> String {
    let mut literal = String::from("\"");
    for character in text.chars() {
        match character {
            '"' => literal.push_str("\\\""),
            '\\' => literal.push_str("\\\\"),
            '\n' => literal.push_str("\\n"),
            '\t' => literal.push_str("\\t"),
            '\r' => literal.push_str("\\r"),
            ' ' => literal.push(' '),
            character
                if character.is_control()
                    || character.is_whitespace()
                    || matches!(character, '\u{200B}'..='\u{200F}' | '\u{2060}'..='\u{206F}' | '\u{FEFF}') =>
            {
                literal.extend(character.escape_unicode());
            }
            character => literal.push(character),
        }
    }
    literal.push('"');
    literal
}

/// The name of the enum standing for a union of string literals: `"KB" | "MB"` → `KbOrMb`.
pub fn choice_enum_name(literals: &[String]) -> String {
    literals
        .iter()
        .map(|literal| pascal_case(literal))
        .collect::<Vec<_>>()
        .join("Or")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_every_key_style() {
        assert_eq!(snake_case_identifier("withMentions"), "with_mentions");
        assert_eq!(snake_case_identifier("NOT_FOUND"), "not_found");
        assert_eq!(snake_case_identifier("do_not_disturb"), "do_not_disturb");
        assert_eq!(snake_case_identifier("heading1"), "heading1");
        assert_eq!(
            snake_case_identifier("copyUserIdHTTPServer"),
            "copy_user_id_http_server"
        );
        assert_eq!(snake_case_identifier("move"), "r#move");
        assert_eq!(snake_case_identifier("720p"), "key_720p");
        assert_eq!(snake_case_identifier("15_minutes"), "key_15_minutes");
        assert_eq!(pascal_case("errors.reasons"), "ErrorsReasons");
        assert_eq!(
            choice_enum_name(&["KB".to_owned(), "MB".to_owned()]),
            "KbOrMb"
        );
    }

    #[test]
    fn escapes_what_a_reader_could_not_see() {
        assert_eq!(
            rust_string_literal("a\u{a0}b \"c\"\\"),
            r#""a\u{a0}b \"c\"\\""#
        );
        assert_eq!(rust_string_literal("1\u{202f}000"), r#""1\u{202f}000""#);
    }
}
