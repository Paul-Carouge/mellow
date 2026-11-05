//! What a page says about itself in its head: Open Graph, Twitter cards, `<title>` and
//! `meta description` (ADR-0041). A small reader of `<meta>` and `<title>` tags, not an HTML
//! parser: unusual markup gives less, never something the page did not say.

use url::Url;

use crate::domain::message_rules::is_forbidden_in_messages;

pub(crate) const MAXIMUM_TITLE_CHARACTERS: usize = 256;
pub(crate) const MAXIMUM_DESCRIPTION_CHARACTERS: usize = 350;
const MAXIMUM_SITE_NAME_CHARACTERS: usize = 64;
/// Attribute values longer than this are not read: no real `content` needs more.
const MAXIMUM_ATTRIBUTE_VALUE_BYTES: usize = 4_096;

/// What a page says about itself; every text is plain, without markup.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct PageDetails {
    pub(crate) title: Option<String>,
    pub(crate) description: Option<String>,
    pub(crate) site_name: Option<String>,
    pub(crate) image_url: Option<Url>,
    /// `0xRRGGBB`.
    pub(crate) theme_color: Option<u32>,
    /// The site icon a profile link shows (ADR-0050): a touch icon, else the first
    /// `<link rel="icon">` that is not SVG, which pictures are never decoded from.
    pub(crate) icon_url: Option<Url>,
}

/// The values found in the head, by priority: Open Graph, then Twitter, then plain HTML.
#[derive(Default)]
struct HeadValues {
    open_graph_title: Option<String>,
    twitter_title: Option<String>,
    html_title: Option<String>,
    open_graph_description: Option<String>,
    twitter_description: Option<String>,
    meta_description: Option<String>,
    open_graph_site_name: Option<String>,
    open_graph_image: Option<String>,
    open_graph_secure_image: Option<String>,
    twitter_image: Option<String>,
    theme_color: Option<String>,
    touch_icon: Option<String>,
    icon: Option<String>,
}

impl HeadValues {
    fn take_meta(&mut self, key: &str, content: String) {
        let slot = match key {
            "og:title" => &mut self.open_graph_title,
            "twitter:title" => &mut self.twitter_title,
            "og:description" => &mut self.open_graph_description,
            "twitter:description" => &mut self.twitter_description,
            "description" => &mut self.meta_description,
            "og:site_name" => &mut self.open_graph_site_name,
            "og:image" | "og:image:url" => &mut self.open_graph_image,
            "og:image:secure_url" => &mut self.open_graph_secure_image,
            "twitter:image" | "twitter:image:src" => &mut self.twitter_image,
            "theme-color" => &mut self.theme_color,
            _ => return,
        };
        // The first one wins, as browsers and Discord read them.
        slot.get_or_insert(content);
    }
}

/// The page's details; relative picture links are resolved against `page_url`.
pub(crate) fn page_details(html: &str, page_url: &Url) -> PageDetails {
    let values = head_values(html);
    let title = values
        .open_graph_title
        .or(values.twitter_title)
        .or(values.html_title);
    let description = values
        .open_graph_description
        .or(values.twitter_description)
        .or(values.meta_description);
    let image_url = [
        values.open_graph_secure_image,
        values.open_graph_image,
        values.twitter_image,
    ]
    .into_iter()
    .flatten()
    .find_map(|link| page_url.join(link.trim()).ok())
    .filter(|url| matches!(url.scheme(), "http" | "https"));
    PageDetails {
        title: title.and_then(|text| plain_text(&text, MAXIMUM_TITLE_CHARACTERS)),
        description: description.and_then(|text| plain_text(&text, MAXIMUM_DESCRIPTION_CHARACTERS)),
        site_name: values
            .open_graph_site_name
            .and_then(|text| plain_text(&text, MAXIMUM_SITE_NAME_CHARACTERS)),
        image_url,
        theme_color: values.theme_color.as_deref().and_then(hex_colour),
        icon_url: [values.touch_icon, values.icon]
            .into_iter()
            .flatten()
            .find_map(|link| page_url.join(link.trim()).ok())
            .filter(|url| matches!(url.scheme(), "http" | "https")),
    }
}

fn head_values(html: &str) -> HeadValues {
    let mut values = HeadValues::default();
    let lowercase = html.to_ascii_lowercase();
    let mut position = 0;
    while let Some(offset) = lowercase[position..].find('<') {
        let start = position + offset;
        let rest = &lowercase[start..];
        if rest.starts_with("<!--") {
            let Some(end) = rest.find("-->") else { break };
            position = start + end + 3;
        } else if rest.starts_with("</head") || rest.starts_with("<body") {
            break;
        } else if let Some(skipped) = ["script", "style", "noscript", "template"]
            .into_iter()
            .find(|tag| is_tag(rest, tag))
        {
            let closing = format!("</{skipped}");
            let Some(end) = rest.find(&closing) else {
                break;
            };
            position = start + end + closing.len();
        } else if is_tag(rest, "title") {
            // The closing tag first, then the opening's end before it, and the scan resumes after
            // the closing tag: each byte is read a bounded number of times.
            let Some(close) = rest.find("</title") else {
                break;
            };
            if let Some(open_end) = rest[..close].find('>')
                && values.html_title.is_none()
            {
                values.html_title =
                    Some(decode_entities(&html[start + open_end + 1..start + close]));
            }
            position = start + close + "</title".len();
        } else if is_tag(rest, "link") {
            let Some(end) = tag_end(rest) else { break };
            let attributes = attributes_of(&html[start + "<link".len()..start + end]);
            take_icon_link(&mut values, &attributes);
            position = start + end + 1;
        } else if is_tag(rest, "meta") {
            let Some(end) = tag_end(rest) else { break };
            let attributes = attributes_of(&html[start + "<meta".len()..start + end]);
            let key = attribute(&attributes, "property").or_else(|| attribute(&attributes, "name"));
            if let (Some(key), Some(content)) = (key, attribute(&attributes, "content")) {
                values.take_meta(&key.to_ascii_lowercase(), decode_entities(content));
            }
            position = start + end + 1;
        } else {
            position = start + 1;
        }
    }
    values
}

/// Keeps the first touch icon and the first other icon a `<link>` names, SVG ones left out.
fn take_icon_link(values: &mut HeadValues, attributes: &[(String, &str)]) {
    let (Some(relation), Some(href)) =
        (attribute(attributes, "rel"), attribute(attributes, "href"))
    else {
        return;
    };
    let is_svg = attribute(attributes, "type")
        .is_some_and(|kind| kind.trim().eq_ignore_ascii_case("image/svg+xml"))
        || href
            .split(['?', '#'])
            .next()
            .is_some_and(|path| path.to_ascii_lowercase().ends_with(".svg"));
    if is_svg {
        return;
    }
    let relation = relation.to_ascii_lowercase();
    let mut kinds = relation.split_ascii_whitespace();
    let slot = if kinds.clone().any(|kind| kind == "apple-touch-icon") {
        &mut values.touch_icon
    } else if kinds.any(|kind| kind == "icon") {
        &mut values.icon
    } else {
        return;
    };
    slot.get_or_insert_with(|| decode_entities(href));
}

/// Whether `rest` (lowercase, starting at `<`) opens the tag `name`.
fn is_tag(rest: &str, name: &str) -> bool {
    rest[1..].starts_with(name)
        && rest[1 + name.len()..].starts_with(|character: char| {
            character.is_ascii_whitespace() || matches!(character, '>' | '/')
        })
}

/// Where the tag at the start of `text` ends (its `>`), quotes respected.
fn tag_end(text: &str) -> Option<usize> {
    let mut quote = None;
    for (index, character) in text.char_indices() {
        match (quote, character) {
            (None, '"' | '\'') => quote = Some(character),
            (Some(open), _) if character == open => quote = None,
            (None, '>') => return Some(index),
            _ => {}
        }
    }
    None
}

/// The attributes of a tag's inside, names lowercased, in order.
fn attributes_of(inside: &str) -> Vec<(String, &str)> {
    let mut attributes = Vec::new();
    let mut rest = inside;
    loop {
        rest = rest.trim_start_matches(|character: char| {
            character.is_ascii_whitespace() || character == '/'
        });
        let name_length = rest
            .find(|character: char| {
                character.is_ascii_whitespace() || matches!(character, '=' | '/')
            })
            .unwrap_or(rest.len());
        if name_length == 0 {
            return attributes;
        }
        let name = rest[..name_length].to_ascii_lowercase();
        rest = rest[name_length..].trim_start();
        let Some(after_equals) = rest.strip_prefix('=') else {
            attributes.push((name, ""));
            continue;
        };
        rest = after_equals.trim_start();
        let (value, after) = if let Some(quote @ ('"' | '\'')) = rest.chars().next() {
            let Some(end) = rest[1..].find(quote) else {
                return attributes;
            };
            (&rest[1..=end], &rest[end + 2..])
        } else {
            let end = rest
                .find(|character: char| character.is_ascii_whitespace())
                .unwrap_or(rest.len());
            (&rest[..end], &rest[end..])
        };
        if value.len() <= MAXIMUM_ATTRIBUTE_VALUE_BYTES {
            attributes.push((name, value));
        }
        rest = after;
    }
}

fn attribute<'a>(attributes: &[(String, &'a str)], name: &str) -> Option<&'a str> {
    attributes
        .iter()
        .find(|(attribute_name, _)| attribute_name == name)
        .map(|(_, value)| *value)
}

/// HTML character references: the five XML ones, `&nbsp;` and numeric ones. Others stay as written.
fn decode_entities(text: &str) -> String {
    let mut decoded = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find('&') {
        decoded.push_str(&rest[..start]);
        rest = &rest[start..];
        // References are short: look for their ";" in the next few characters only.
        let Some(end) = rest
            .char_indices()
            .take(12)
            .find_map(|(index, character)| (character == ';').then_some(index))
        else {
            decoded.push('&');
            rest = &rest[1..];
            continue;
        };
        let reference = &rest[1..end];
        let character = match reference {
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            "nbsp" => Some(' '),
            _ => reference
                .strip_prefix("#x")
                .or_else(|| reference.strip_prefix("#X"))
                .map_or_else(
                    || {
                        reference
                            .strip_prefix('#')
                            .and_then(|digits| digits.parse::<u32>().ok())
                    },
                    |hexadecimal| u32::from_str_radix(hexadecimal, 16).ok(),
                )
                .and_then(char::from_u32),
        };
        if let Some(character) = character {
            decoded.push(character);
            rest = &rest[end + 1..];
        } else {
            decoded.push('&');
            rest = &rest[1..];
        }
    }
    decoded.push_str(rest);
    decoded
}

/// One line of text without control characters, cut to `maximum` characters with an ellipsis;
/// `None` when nothing is left.
pub(crate) fn plain_text(text: &str, maximum: usize) -> Option<String> {
    let words: Vec<&str> = text
        .split(|character: char| {
            // Direction overrides would let a page make its title read as something else.
            character.is_whitespace() || is_forbidden_in_messages(character)
        })
        .filter(|word| !word.is_empty())
        .collect();
    let line = words.join(" ");
    if line.is_empty() {
        return None;
    }
    if line.chars().count() <= maximum {
        return Some(line);
    }
    let mut cut: String = line.chars().take(maximum - 1).collect();
    cut.push('…');
    Some(cut)
}

/// `#rrggbb` or `#rgb` as `0xRRGGBB`.
fn hex_colour(text: &str) -> Option<u32> {
    let digits = text.trim().strip_prefix('#')?;
    if !digits
        .chars()
        .all(|character| character.is_ascii_hexdigit())
    {
        return None;
    }
    match digits.len() {
        6 => u32::from_str_radix(digits, 16).ok(),
        3 => {
            let expanded: String = digits.chars().flat_map(|digit| [digit, digit]).collect();
            u32::from_str_radix(&expanded, 16).ok()
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn details(html: &str) -> PageDetails {
        page_details(
            html,
            &Url::parse("https://example.org/articles/one").unwrap(),
        )
    }

    #[test]
    fn touch_icons_win_over_icons_and_svg_ones_are_left_out() {
        let page = details(
            r#"<head><link rel="stylesheet" href="/a.css">
            <link rel="icon" type="image/svg+xml" href="/icon.svg">
            <link rel="shortcut icon" href="/favicon.png?v=2&amp;x=1">
            <link rel="icon" href="/later.png">
            <link rel="apple-touch-icon" href="https://cdn.example.org/touch.png">
            <link rel="apple-touch-icon" href="/touch.svg"></head>"#,
        );
        assert_eq!(
            page.icon_url.unwrap().as_str(),
            "https://cdn.example.org/touch.png"
        );
        let page = details(
            r#"<link rel="shortcut icon" href="/favicon.png?v=2&amp;x=1"><link rel=icon href=b.png>"#,
        );
        assert_eq!(
            page.icon_url.unwrap().as_str(),
            "https://example.org/favicon.png?v=2&x=1"
        );
        let page = details(r#"<link rel=icon href="favicon.png"><link rel=icon href=x.svg>"#);
        assert_eq!(
            page.icon_url.unwrap().as_str(),
            "https://example.org/articles/favicon.png"
        );
        assert_eq!(
            details(r#"<link rel="icon" href="data:image/png;base64,AA">"#).icon_url,
            None
        );
        assert_eq!(
            details(r#"<link rel="icon" href="/x.svg#a">"#).icon_url,
            None
        );
    }

    #[test]
    fn open_graph_wins_over_twitter_and_plain_html() {
        let page = details(
            r##"<!doctype html><html><head>
            <title>Plain &amp; simple</title>
            <meta name="description" content="Plain description">
            <meta name="twitter:title" content="Twitter title">
            <META property='og:title' content='The &quot;real&quot; title'>
            <meta property="og:description" content="A   description
               on two lines &#233;t&#xE9;.">
            <meta property="og:site_name" content="Example">
            <meta property="og:image" content="/images/cover.png">
            <meta name="theme-color" content="#3a7">
            </head><body><meta property="og:title" content="ignored"></body></html>"##,
        );
        assert_eq!(page.title.as_deref(), Some("The \"real\" title"));
        assert_eq!(
            page.description.as_deref(),
            Some("A description on two lines été.")
        );
        assert_eq!(page.site_name.as_deref(), Some("Example"));
        assert_eq!(
            page.image_url.map(String::from).as_deref(),
            Some("https://example.org/images/cover.png")
        );
        assert_eq!(page.theme_color, Some(0x33_aa77));
    }

    #[test]
    fn plain_html_is_the_fallback_and_scripts_and_comments_are_skipped() {
        let page = details(
            r#"<head><!-- <meta property="og:title" content="comment"> -->
            <script>document.write('<meta property="og:title" content="script">')</script>
            <title>  Fallback   title </title>
            <meta content="Described" name=description></head>"#,
        );
        assert_eq!(page.title.as_deref(), Some("Fallback title"));
        assert_eq!(page.description.as_deref(), Some("Described"));
        assert!(page.image_url.is_none());
    }

    #[test]
    fn long_texts_are_cut_and_odd_markup_gives_nothing_rather_than_garbage() {
        let long = "word ".repeat(200);
        let page = details(&format!(r#"<meta property="og:title" content="{long}">"#));
        let title = page.title.unwrap();
        assert_eq!(title.chars().count(), MAXIMUM_TITLE_CHARACTERS);
        assert!(title.ends_with('…'));
        assert_eq!(
            details(r#"<meta property="og:title" content="unclosed"#),
            PageDetails::default()
        );
        assert_eq!(details("<title>no end"), PageDetails::default());
        assert_eq!(
            details(r#"<meta property="og:image" content="javascript:alert(1)">"#),
            PageDetails::default()
        );
        assert_eq!(
            details(r#"<meta name="theme-color" content="red">"#).theme_color,
            None
        );
        assert_eq!(
            details("<title>evil\u{202E}gnp.exe</title>")
                .title
                .as_deref(),
            Some("evil gnp.exe")
        );
        // Once quadratic: every "<title" scanned to the page's one ">" at the end.
        let hostile = format!("{}>", "<title </title".repeat(20_000));
        assert_eq!(details(&hostile), PageDetails::default());
        assert_eq!(
            decode_entities("a &unknown; &amp &#xD83D; é&"),
            "a &unknown; &amp &#xD83D; é&"
        );
    }
}
