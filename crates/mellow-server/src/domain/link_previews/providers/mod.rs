//! Providers whose public API or oEmbed endpoint says more than their page (ADR-0044): each module
//! recognises its canonical links and turns its provider's answer into a card and an embed. Any
//! other link, or a provider that does not answer, gets the page's card.

use mellow_protocol::LinkEmbed;
use url::Url;

use super::fetcher::LinkFetcher;
use super::page::PageDetails;

mod youtube;

/// What a provider says about a link: the card's texts, its one picture, and the embed.
#[derive(Debug)]
pub(crate) struct ProviderPreview {
    /// `image_url` is unused: the provider fetches its picture itself, with its own fallbacks.
    pub(crate) details: PageDetails,
    /// The picture's bytes, not yet checked as a picture.
    pub(crate) picture: Option<Vec<u8>>,
    pub(crate) embed: LinkEmbed,
}

/// What the provider of a link answered.
#[derive(Debug)]
pub(crate) enum ProviderAnswer {
    /// No provider knows the link, or its provider says the item is unknown or private: the
    /// link gets the page's card.
    NotProvided,
    Preview(Box<ProviderPreview>),
    /// The provider did not answer this time (timeout, network, 429, 5xx): no card for now, and
    /// the link is asked again soon rather than given the page's card, which for YouTube can be
    /// its consent wall, for a whole day.
    Unavailable,
}

/// The provider's answer for `link` (already canonical).
pub(crate) async fn provider_answer(fetcher: &LinkFetcher, link: &Url) -> ProviderAnswer {
    if let Some(youtube_link) = youtube::youtube_link_of(link) {
        return youtube::youtube_preview(fetcher, &youtube_link).await;
    }
    ProviderAnswer::NotProvided
}
