//! Wikipedia articles (ADR-0044): the REST `page/summary` endpoint of the article's own wiki gives
//! its plain-text extract and a thumbnail, where the page's `meta description` is often empty.

use mellow_protocol::{LinkEmbed, WikipediaArticleEmbed};
use serde::Deserialize;
use url::Url;

use super::{ProviderOutcome, ProviderPreview};
use crate::domain::link_previews::fetcher::LinkFetcher;
use crate::domain::link_previews::page::{
    MAXIMUM_DESCRIPTION_CHARACTERS, MAXIMUM_TITLE_CHARACTERS, PageDetails, plain_text,
};

const WIKIPEDIA_DOMAIN: &str = "wikipedia.org";
/// `fr.m.wikipedia.org`, the mobile site, names the same article.
const MOBILE_LABEL: &str = "m";
const ARTICLE_PATH_PREFIX: &str = "/wiki/";
const SUMMARY_PATH: &str = "api/rest_v1/page/summary/";
const SITE_NAME: &str = "Wikipedia";
/// The summary's `type` of a disambiguation page.
const DISAMBIGUATION_TYPE: &str = "disambiguation";
/// Wiki codes: `fr`, `simple`, `zh-min-nan`, `be-tarask`.
const LANGUAGE_CODE_LENGTHS: std::ops::RangeInclusive<usize> = 2..=16;
/// Query parameters that show another page than the article's current version (a revision, a
/// diff, a search): such links are left as they are.
const PAGE_CHANGING_PARAMETERS: [&str; 4] = ["oldid", "diff", "curid", "search"];
/// Wikipedia's titles are at most 255 bytes, up to three times that once percent-encoded.
const MAXIMUM_TITLE_IN_PATH_BYTES: usize = 3 * 255;

/// An article of a Wikipedia, as its link names it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct WikipediaArticle {
    /// `fr` for `fr.wikipedia.org`.
    language: String,
    /// The title as the link's path writes it, percent-encoded.
    title_in_path: String,
}

/// Where the summaries are: each article's own wiki, or a test site.
#[derive(Debug, Clone, Default)]
pub(crate) struct Wikipedia {
    summary_origin_for_tests: Option<Url>,
}

impl Wikipedia {
    #[cfg(test)]
    pub(crate) fn served_by_for_tests(origin: &Url) -> Self {
        Self {
            summary_origin_for_tests: Some(origin.clone()),
        }
    }

    pub(crate) async fn preview_of(
        &self,
        fetcher: &LinkFetcher,
        article: &WikipediaArticle,
    ) -> ProviderOutcome {
        let Some(summary_url) = self.summary_url(article) else {
            return ProviderOutcome::Failed;
        };
        match fetcher.fetch_json::<Summary>(summary_url).await {
            Ok(summary) => summary_preview(summary, &article.language)
                .map_or(ProviderOutcome::Failed, |preview| {
                    ProviderOutcome::Found(Box::new(preview))
                }),
            Err(_) => ProviderOutcome::Failed,
        }
    }

    fn summary_url(&self, article: &WikipediaArticle) -> Option<Url> {
        let origin = match &self.summary_origin_for_tests {
            Some(origin) => origin.clone(),
            None => {
                Url::parse(&format!("https://{}.{WIKIPEDIA_DOMAIN}/", article.language)).ok()?
            }
        };
        // A title with a slash (`AC/DC`) is one segment of the summary's path.
        let title = article.title_in_path.replace('/', "%2F");
        origin.join(&format!("{SUMMARY_PATH}{title}")).ok()
    }
}

/// The article a link to `<language>.wikipedia.org/wiki/<title>` (or the mobile site) names.
pub(crate) fn article_of(link: &Url) -> Option<WikipediaArticle> {
    if link.port().is_some() || !matches!(link.scheme(), "http" | "https") {
        return None;
    }
    let host = link
        .host_str()?
        .strip_suffix(WIKIPEDIA_DOMAIN)?
        .strip_suffix('.')?;
    let language = host
        .strip_suffix(MOBILE_LABEL)
        .and_then(|language| language.strip_suffix('.'))
        .unwrap_or(host);
    let title_in_path = link.path().strip_prefix(ARTICLE_PATH_PREFIX)?;
    if link
        .query_pairs()
        .any(|(name, _)| PAGE_CHANGING_PARAMETERS.contains(&name.as_ref()))
    {
        return None;
    }
    if !is_language_code(language)
        || title_in_path.is_empty()
        || title_in_path.len() > MAXIMUM_TITLE_IN_PATH_BYTES
    {
        return None;
    }
    Some(WikipediaArticle {
        language: language.to_owned(),
        title_in_path: title_in_path.to_owned(),
    })
}

/// `https://<language>.wikipedia.org/wiki/<title>`, without query: the mobile site and the
/// desktop one share one preview.
pub(crate) fn canonical_link(link: &Url) -> Option<Url> {
    let article = article_of(link)?;
    Url::parse(&format!(
        "https://{}.{WIKIPEDIA_DOMAIN}{ARTICLE_PATH_PREFIX}{}",
        article.language, article.title_in_path
    ))
    .ok()
}

fn is_language_code(code: &str) -> bool {
    LANGUAGE_CODE_LENGTHS.contains(&code.len())
        && code != "www"
        && code.starts_with(|character: char| character.is_ascii_lowercase())
        && !code.ends_with('-')
        && code
            .chars()
            .all(|character| character.is_ascii_lowercase() || character == '-')
}

#[derive(Debug, Deserialize)]
struct Titles {
    #[serde(default)]
    normalized: Option<String>,
}

#[derive(Debug, Deserialize)]
struct Thumbnail {
    source: String,
}

/// The fields of `page/summary` the card shows.
#[derive(Debug, Deserialize)]
struct Summary {
    #[serde(default, rename = "type")]
    page_type: Option<String>,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    titles: Option<Titles>,
    #[serde(default)]
    extract: Option<String>,
    #[serde(default)]
    thumbnail: Option<Thumbnail>,
}

fn summary_preview(summary: Summary, language: &str) -> Option<ProviderPreview> {
    let title = summary
        .titles
        .and_then(|titles| titles.normalized)
        .or(summary.title)
        .and_then(|title| plain_text(&title, MAXIMUM_TITLE_CHARACTERS))?;
    Some(ProviderPreview {
        details: PageDetails {
            title: Some(title),
            description: summary
                .extract
                .and_then(|extract| plain_text(&extract, MAXIMUM_DESCRIPTION_CHARACTERS)),
            site_name: Some(SITE_NAME.to_owned()),
            image_url: summary
                .thumbnail
                .and_then(|thumbnail| Url::parse(&thumbnail.source).ok())
                .filter(|url| matches!(url.scheme(), "http" | "https")),
            theme_color: None,
        },
        picture: None,
        embed: LinkEmbed::WikipediaArticle(WikipediaArticleEmbed {
            language: language.to_owned(),
            is_disambiguation: summary.page_type.as_deref() == Some(DISAMBIGUATION_TYPE),
        }),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn canonical(link: &str) -> Option<String> {
        canonical_link(&Url::parse(link).unwrap()).map(|url| url.to_string())
    }

    #[test]
    fn desktop_and_mobile_links_to_an_article_share_one_form() {
        for link in [
            "https://fr.wikipedia.org/wiki/Cr%C3%A9puscule",
            "https://fr.m.wikipedia.org/wiki/Crépuscule",
            "http://fr.wikipedia.org/wiki/Crépuscule?useskin=vector",
        ] {
            assert_eq!(
                canonical(link).as_deref(),
                Some("https://fr.wikipedia.org/wiki/Cr%C3%A9puscule"),
                "{link}"
            );
        }
        assert_eq!(
            canonical("https://zh-min-nan.wikipedia.org/wiki/AC/DC").as_deref(),
            Some("https://zh-min-nan.wikipedia.org/wiki/AC/DC")
        );
    }

    #[test]
    fn other_wikimedia_pages_and_look_alikes_are_left_alone() {
        for link in [
            "https://www.wikipedia.org/wiki/Twilight",
            "https://wikipedia.org/wiki/Twilight",
            "https://en.wikipedia.org/w/index.php?title=Twilight",
            "https://en.wikipedia.org/wiki/",
            "https://en.wiktionary.org/wiki/twilight",
            "https://en.wikipedia.org.evil.example/wiki/Twilight",
            "https://evilwikipedia.org/wiki/Twilight",
            "https://en.wikipedia.org:8443/wiki/Twilight",
            "https://e.wikipedia.org/wiki/Twilight",
            "https://e1.wikipedia.org/wiki/Twilight",
            "https://en.wikipedia.org/wiki/Twilight?oldid=123",
            "https://en.wikipedia.org/wiki/Twilight?diff=prev&oldid=123",
            "https://en.wikipedia.org/wiki/Twilight?curid=42",
            "https://en.wikipedia.org/wiki/Special:Search?search=dusk",
        ] {
            assert_eq!(canonical(link), None, "{link}");
        }
    }

    #[test]
    fn the_summary_is_asked_of_the_article_s_own_wiki_with_slashes_encoded() {
        let article =
            article_of(&Url::parse("https://en.wikipedia.org/wiki/AC/DC").unwrap()).unwrap();
        assert_eq!(
            Wikipedia::default().summary_url(&article).unwrap().as_str(),
            "https://en.wikipedia.org/api/rest_v1/page/summary/AC%2FDC"
        );
    }
}
