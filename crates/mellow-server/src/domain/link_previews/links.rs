//! Which links of a message get a preview (ADR-0041).

use url::Url;

use super::fetcher::is_klipy_host;
use crate::domain::mentions::text_outside_code;

/// A message previews at most this many links, the first ones.
pub(crate) const MAXIMUM_PREVIEWED_LINKS_PER_MESSAGE: usize = 3;
/// Longer links are not previewed: no real page needs more, and they bound the work per link.
const MAXIMUM_PREVIEWED_LINK_LENGTH: usize = 2_048;

/// The text a reader sees without opening anything: code and spoilers are left out, so a
/// preview never shows what the writer hid.
fn text_outside_code_and_spoilers(content: &str) -> String {
    let text = text_outside_code(content);
    let mut kept = String::with_capacity(text.len());
    let mut parts = text.split("||");
    let mut is_hidden = false;
    let mut pending = parts.next().unwrap_or_default().to_owned();
    for part in parts {
        // A part after an odd number of "||" is hidden, unless no closing "||" follows it.
        if !is_hidden {
            kept.push_str(&pending);
            kept.push(' ');
        }
        is_hidden = !is_hidden;
        part.clone_into(&mut pending);
    }
    if is_hidden {
        // An unclosed "||" is plain text.
        kept.push_str("||");
    }
    kept.push_str(&pending);
    kept
}

/// Cuts what Markdown and prose put around a link: closing marks and punctuation, and a closing
/// parenthesis the link did not open (`(see https://a.b/c)`, `[x](https://a.b)`). Parentheses
/// are counted once, so a hostile run of them costs one pass.
fn trimmed_link(candidate: &str) -> &str {
    let open = candidate.matches('(').count();
    let mut close = candidate.matches(')').count();
    let mut link = candidate;
    loop {
        let before = link.len();
        // None of these marks is a parenthesis: the counts stay right.
        link = link.trim_end_matches(['.', ',', ';', ':', '!', '?', '\'', '"', '*', '_', '~', '|']);
        if open < close
            && let Some(shorter) = link.strip_suffix(')')
        {
            link = shorter;
            close -= 1;
        }
        if let Some(shorter) = link.strip_suffix('>') {
            link = shorter;
        }
        if link.len() == before {
            return link;
        }
    }
}

fn is_previewable(url: &Url) -> bool {
    !is_klipy_host(url)
        && matches!(url.scheme(), "http" | "https")
        && url.host().is_some()
        && url.username().is_empty()
        && url.password().is_none()
}

/// The links of `content` to preview, in order, once each, without their fragment: `http` and
/// `https` without credentials, outside code and spoilers (ports and addresses are the fetcher's
/// to refuse), not written `<https://…>` (which
/// keeps a link without preview, as on Discord).
pub(crate) fn links_to_preview(content: &str) -> Vec<Url> {
    if !content.contains("http") {
        return Vec::new();
    }
    let text = text_outside_code_and_spoilers(content);
    let mut links: Vec<Url> = Vec::new();
    for (start, _) in text.match_indices("http") {
        let rest = &text[start..];
        if !(rest.starts_with("http://") || rest.starts_with("https://")) {
            continue;
        }
        let is_after_angle_bracket = text[..start].ends_with('<');
        let candidate_length = rest
            .find(|character: char| character.is_whitespace() || character == '<')
            .unwrap_or(rest.len());
        let candidate = &rest[..candidate_length];
        // Checked before any other work: a link longer than this is not previewed anyway.
        if candidate.len() > MAXIMUM_PREVIEWED_LINK_LENGTH + 16
            || is_after_angle_bracket && candidate.contains('>')
        {
            continue;
        }
        let link = trimmed_link(candidate);
        if link.len() > MAXIMUM_PREVIEWED_LINK_LENGTH {
            continue;
        }
        let Ok(mut url) = Url::parse(link) else {
            continue;
        };
        url.set_fragment(None);
        if is_previewable(&url) && !links.contains(&url) {
            links.push(url);
            if links.len() == MAXIMUM_PREVIEWED_LINKS_PER_MESSAGE {
                break;
            }
        }
    }
    links
}

#[cfg(test)]
mod tests {
    use super::*;

    fn links(content: &str) -> Vec<String> {
        links_to_preview(content)
            .into_iter()
            .map(String::from)
            .collect()
    }

    #[test]
    fn links_are_found_in_prose_and_markdown_once_and_without_fragment() {
        assert_eq!(
            links("see https://example.org/a. and (http://example.com/b) or [x](https://a.b/c)!"),
            [
                "https://example.org/a",
                "http://example.com/b",
                "https://a.b/c"
            ]
        );
        assert_eq!(
            links(
                "https://en.wikipedia.org/wiki/Rust_(language) https://example.org/#top https://example.org/"
            ),
            [
                "https://en.wikipedia.org/wiki/Rust_(language)",
                "https://example.org/"
            ]
        );
        assert_eq!(
            links("**https://example.org/x**"),
            ["https://example.org/x"]
        );
    }

    #[test]
    fn code_spoilers_angle_brackets_other_schemes_and_credentials_are_left_out() {
        assert_eq!(
            links("`https://a.example/` ||https://b.example/|| <https://c.example/>"),
            Vec::<String>::new()
        );
        assert_eq!(
            links(
                "https://a.example:8080/ http://user:pw@b.example/ ftp://c.example/ https://d.example:443/"
            ),
            ["https://a.example:8080/", "https://d.example/"],
            "the fetcher refuses other ports"
        );
        assert_eq!(
            links("an || unclosed https://a.example/"),
            ["https://a.example/"]
        );
        assert_eq!(links("<https://a.example/ is open"), ["https://a.example/"]);
    }

    #[test]
    fn at_most_three_links_and_none_too_long() {
        assert_eq!(
            links("https://a.example/ https://b.example/ https://c.example/ https://d.example/")
                .len(),
            MAXIMUM_PREVIEWED_LINKS_PER_MESSAGE
        );
        assert_eq!(
            links(&format!("https://a.example/{}", "x".repeat(3_000))),
            Vec::<String>::new()
        );
        assert_eq!(links("no link here"), Vec::<String>::new());
        assert_eq!(
            links(
                "https://static.klipy.com/ii/a/b.webp https://klipy.com/gifs/x https://notklipy.com/"
            ),
            ["https://notklipy.com/"],
            "Klipy's own links only"
        );
        // Once quadratic: one parenthesis trimmed per pass, recounted each time.
        let hostile = format!("{}{}", "http://[".repeat(167), ")".repeat(2_667));
        assert_eq!(links(&hostile), Vec::<String>::new());
    }
}
