//! Providers whose public API or oEmbed endpoint says more about a link than its page
//! (ADR-0044): each module recognises its links, puts them in one canonical form, and turns its
//! provider's answer into the card's texts and an embed. Any other link, or a provider that
//! fails, leaves the link to the page's own card.

pub(crate) mod github;
pub(crate) mod wikipedia;
mod youtube;

use mellow_protocol::LinkEmbed;
use url::Url;

use super::fetcher::LinkFetcher;
use super::page::PageDetails;

/// What a provider says about a link: the card's texts, its picture, and the embed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ProviderPreview {
    pub(crate) details: PageDetails,
    /// The picture's bytes when the provider fetched it itself, with its own fallbacks (not yet
    /// checked as a picture); otherwise the picture at `details.image_url`, if any.
    pub(crate) picture: Option<Vec<u8>>,
    pub(crate) embed: LinkEmbed,
}

/// What became of asking the providers about a link.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum ProviderOutcome {
    /// No provider knows this link, or its provider says the item is unknown, private or not
    /// embeddable: the page's card.
    NotProvided,
    Found(Box<ProviderPreview>),
    /// The link's provider refused, is rate limited or answered something unreadable: the page's
    /// card, kept only as long as an empty one so the provider's comes once it answers again.
    Failed,
    /// The provider did not answer this time (timeout, network, 429, 5xx): no card for now, and
    /// the link is asked again soon rather than given the page's card, which for YouTube can be
    /// its consent wall, for a whole day.
    Unavailable,
}

/// Every provider, with where its API is; cheap to clone, the clones share GitHub's guard.
#[derive(Debug, Clone, Default)]
pub(crate) struct LinkProviders {
    github: github::GitHub,
    wikipedia: wikipedia::Wikipedia,
}

impl LinkProviders {
    /// GitHub's links and API, and Wikipedia's summaries, all served by the test site `origin`.
    #[cfg(test)]
    pub(crate) fn served_by_for_tests(origin: &Url) -> Self {
        Self {
            github: github::GitHub::served_by_for_tests(origin),
            wikipedia: wikipedia::Wikipedia::served_by_for_tests(origin),
        }
    }

    /// `link` is already in its canonical form (`super::canonical_links`).
    pub(crate) async fn preview_of(&self, fetcher: &LinkFetcher, link: &Url) -> ProviderOutcome {
        if let Some(youtube_link) = youtube::youtube_link_of(link) {
            return youtube::youtube_preview(fetcher, &youtube_link).await;
        }
        if let Some(thing) = self.github.thing_of(link) {
            return self.github.preview_of(fetcher, &thing).await;
        }
        if let Some(article) = wikipedia::article_of(link) {
            return self.wikipedia.preview_of(fetcher, &article).await;
        }
        ProviderOutcome::NotProvided
    }
}

/// The canonical form of a link one of these providers knows (ADR-0044).
pub(crate) fn canonical_link(link: &Url) -> Option<Url> {
    github::canonical_link(link).or_else(|| wikipedia::canonical_link(link))
}
