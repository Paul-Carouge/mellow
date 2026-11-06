//! Splits the TypeScript of the web catalogues into tokens. Only what the catalogues use is
//! understood; anything else (a regular expression, a hexadecimal number, an unknown escape) stops
//! generation with its line, so a new shape in the catalogues is never misread.

use anyhow::{Result, bail};

#[derive(Debug, Clone, PartialEq)]
pub enum TokenKind {
    Identifier(String),
    /// A quoted string, its escapes already decoded.
    String(String),
    Number(f64),
    Template(Template),
    Punctuation(&'static str),
}

/// A template literal: `quasis` has one more element than `expressions`, as in JavaScript.
#[derive(Debug, Clone, PartialEq)]
pub struct Template {
    pub quasis: Vec<String>,
    pub expressions: Vec<Vec<Token>>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Token {
    pub kind: TokenKind,
    pub line: usize,
}

/// Longest first, so `===` is never read as `==` then `=`.
const PUNCTUATIONS: [&str; 30] = [
    "...", "===", "!==", "=>", "<=", ">=", "&&", "||", "??", "?.", "==", "!=", "{", "}", "(", ")",
    "[", "]", ",", ":", ";", ".", "?", "!", "<", ">", "+", "-", "*", "/",
];
const OTHER_SINGLE_PUNCTUATIONS: [&str; 4] = ["%", "=", "|", "&"];

/// Every token of `source`.
pub fn tokenize(source: &str) -> Result<Vec<Token>> {
    let mut lexer = Lexer::new(source, 1);
    lexer.tokens_until(Stop::EndOfInput)
}

/// The tokens of the statement that starts `source`, up to its `;` (excluded). Lets one constant
/// be read from a file whose other statements the lexer does not understand.
pub fn tokenize_statement(source: &str, first_line: usize) -> Result<Vec<Token>> {
    let mut lexer = Lexer::new(source, first_line);
    lexer.tokens_until(Stop::TopLevelSemicolon)
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Stop {
    EndOfInput,
    TopLevelSemicolon,
    /// The `}` closing a template's `${`.
    ClosingBrace,
}

struct Lexer {
    characters: Vec<char>,
    position: usize,
    line: usize,
}

impl Lexer {
    fn new(source: &str, first_line: usize) -> Self {
        Self {
            characters: source.chars().collect(),
            position: 0,
            line: first_line,
        }
    }

    fn peek(&self, offset: usize) -> Option<char> {
        self.characters.get(self.position + offset).copied()
    }

    fn advance(&mut self) -> Option<char> {
        let character = self.peek(0)?;
        self.position += 1;
        if character == '\n' {
            self.line += 1;
        }
        Some(character)
    }

    fn starts_with(&self, text: &str) -> bool {
        text.chars()
            .enumerate()
            .all(|(offset, character)| self.peek(offset) == Some(character))
    }

    fn tokens_until(&mut self, stop: Stop) -> Result<Vec<Token>> {
        let mut tokens = Vec::new();
        let mut depth = 0_usize;
        loop {
            self.skip_whitespace_and_comments()?;
            let Some(character) = self.peek(0) else {
                if stop == Stop::EndOfInput {
                    return Ok(tokens);
                }
                bail!("line {}: the input ends inside a statement", self.line);
            };
            let line = self.line;
            if depth == 0 {
                if stop == Stop::ClosingBrace && character == '}' {
                    self.advance();
                    return Ok(tokens);
                }
                if stop == Stop::TopLevelSemicolon && character == ';' {
                    return Ok(tokens);
                }
            }
            let kind = if character.is_ascii_alphabetic() || character == '_' || character == '$' {
                self.identifier()
            } else if character.is_ascii_digit() {
                self.number()?
            } else if character == '"' || character == '\'' {
                self.quoted_string(character)?
            } else if character == '`' {
                self.template()?
            } else {
                self.punctuation()?
            };
            match kind {
                TokenKind::Punctuation("{" | "(" | "[") => depth += 1,
                TokenKind::Punctuation("}" | ")" | "]") => {
                    depth = depth.checked_sub(1).ok_or_else(|| {
                        anyhow::anyhow!("line {line}: a closing bracket has no opening one")
                    })?;
                }
                _ => {}
            }
            tokens.push(Token { kind, line });
        }
    }

    fn skip_whitespace_and_comments(&mut self) -> Result<()> {
        loop {
            match self.peek(0) {
                Some(character) if character.is_whitespace() => {
                    self.advance();
                }
                Some('/') if self.peek(1) == Some('/') => {
                    while self.peek(0).is_some_and(|character| character != '\n') {
                        self.advance();
                    }
                }
                Some('/') if self.peek(1) == Some('*') => {
                    let line = self.line;
                    self.advance();
                    self.advance();
                    while !self.starts_with("*/") {
                        if self.advance().is_none() {
                            bail!("line {line}: a comment is never closed");
                        }
                    }
                    self.advance();
                    self.advance();
                }
                _ => return Ok(()),
            }
        }
    }

    fn identifier(&mut self) -> TokenKind {
        let mut name = String::new();
        while let Some(character) = self
            .peek(0)
            .filter(|character| character.is_ascii_alphanumeric() || matches!(character, '_' | '$'))
        {
            name.push(character);
            self.advance();
        }
        TokenKind::Identifier(name)
    }

    fn number(&mut self) -> Result<TokenKind> {
        let line = self.line;
        let mut digits = String::new();
        while let Some(character) = self
            .peek(0)
            .filter(|character| character.is_ascii_digit() || matches!(character, '.' | '_'))
        {
            // `1.toString()` is not in the catalogues; a dot not followed by a digit ends the number.
            if character == '.' && !self.peek(1).is_some_and(|next| next.is_ascii_digit()) {
                break;
            }
            if character != '_' {
                digits.push(character);
            }
            self.advance();
        }
        if self
            .peek(0)
            .is_some_and(|character| character.is_ascii_alphabetic())
        {
            bail!("line {line}: only decimal numbers without exponent are supported");
        }
        digits
            .parse()
            .map(TokenKind::Number)
            .map_err(|_| anyhow::anyhow!("line {line}: `{digits}` is not a number"))
    }

    fn quoted_string(&mut self, quote: char) -> Result<TokenKind> {
        let line = self.line;
        self.advance();
        let mut text = String::new();
        loop {
            match self.advance() {
                None | Some('\n') => bail!("line {line}: a string is never closed"),
                Some(character) if character == quote => return Ok(TokenKind::String(text)),
                Some('\\') => text.push(self.escape()?),
                Some(character) => text.push(character),
            }
        }
    }

    fn template(&mut self) -> Result<TokenKind> {
        let line = self.line;
        self.advance();
        let mut quasis = Vec::new();
        let mut expressions = Vec::new();
        let mut text = String::new();
        loop {
            match self.advance() {
                None => bail!("line {line}: a template string is never closed"),
                Some('`') => {
                    quasis.push(text);
                    return Ok(TokenKind::Template(Template {
                        quasis,
                        expressions,
                    }));
                }
                Some('\\') => text.push(self.escape()?),
                Some('$') if self.peek(0) == Some('{') => {
                    self.advance();
                    quasis.push(std::mem::take(&mut text));
                    expressions.push(self.tokens_until(Stop::ClosingBrace)?);
                }
                Some(character) => text.push(character),
            }
        }
    }

    fn escape(&mut self) -> Result<char> {
        let line = self.line;
        let escaped = match self.advance() {
            Some('n') => '\n',
            Some('t') => '\t',
            Some('r') => '\r',
            Some('0') => '\0',
            Some(character @ ('\\' | '\'' | '"' | '`' | '$')) => character,
            Some('u') => return self.unicode_escape(line),
            other => bail!(
                "line {line}: unsupported escape `\\{}`",
                other.unwrap_or(' ')
            ),
        };
        Ok(escaped)
    }

    fn unicode_escape(&mut self, line: usize) -> Result<char> {
        let mut hexadecimal = String::new();
        if self.peek(0) == Some('{') {
            self.advance();
            while let Some(character) = self.advance() {
                if character == '}' {
                    break;
                }
                hexadecimal.push(character);
            }
        } else {
            for _ in 0..4 {
                hexadecimal.extend(self.advance());
            }
        }
        u32::from_str_radix(&hexadecimal, 16)
            .ok()
            .and_then(char::from_u32)
            .ok_or_else(|| anyhow::anyhow!("line {line}: `\\u{hexadecimal}` is not a character"))
    }

    fn punctuation(&mut self) -> Result<TokenKind> {
        let line = self.line;
        let Some(punctuation) = PUNCTUATIONS
            .iter()
            .chain(OTHER_SINGLE_PUNCTUATIONS.iter())
            .find(|punctuation| self.starts_with(punctuation))
        else {
            bail!(
                "line {line}: unsupported character `{}`",
                self.peek(0).unwrap_or(' ')
            );
        };
        for _ in 0..punctuation.chars().count() {
            self.advance();
        }
        Ok(TokenKind::Punctuation(punctuation))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(source: &str) -> Vec<TokenKind> {
        tokenize(source)
            .unwrap()
            .into_iter()
            .map(|token| token.kind)
            .collect()
    }

    #[test]
    fn reads_strings_with_their_escapes_decoded() {
        assert_eq!(
            kinds(r#"'it\'s' "a\u00a0b" "\u{2212}""#),
            vec![
                TokenKind::String("it's".to_owned()),
                TokenKind::String("a\u{a0}b".to_owned()),
                TokenKind::String("\u{2212}".to_owned()),
            ]
        );
    }

    #[test]
    fn reads_nested_templates_as_parts_and_expression_tokens() {
        let kinds: [TokenKind; 1] = kinds("`a${b ? `c${d}` : \"\"}e`").try_into().unwrap();
        let [TokenKind::Template(template)] = kinds else {
            panic!("not one template");
        };
        assert_eq!(template.quasis, vec!["a".to_owned(), "e".to_owned()]);
        assert_eq!(template.expressions[0].len(), 5);
    }

    #[test]
    fn skips_comments_and_counts_lines() {
        let tokens = tokenize("// one\n/** two\n */\nname").unwrap();
        assert_eq!(tokens[0].line, 4);
    }

    #[test]
    fn prefers_the_longest_punctuation() {
        assert_eq!(
            kinds("a !== b ... =>"),
            vec![
                TokenKind::Identifier("a".to_owned()),
                TokenKind::Punctuation("!=="),
                TokenKind::Identifier("b".to_owned()),
                TokenKind::Punctuation("..."),
                TokenKind::Punctuation("=>"),
            ]
        );
    }

    #[test]
    fn stops_a_statement_at_its_top_level_semicolon() {
        let tokens = tokenize_statement("{ a: `;` }; /regex/", 1).unwrap();
        assert_eq!(tokens.len(), 5);
    }

    #[test]
    fn rejects_what_it_does_not_understand() {
        assert!(tokenize("0x10").is_err());
        assert!(tokenize("\"\\q\"").is_err());
        assert!(tokenize("a @ b").is_err());
        assert!(tokenize("\"open").is_err());
    }
}
