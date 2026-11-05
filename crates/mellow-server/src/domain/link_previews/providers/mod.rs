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

/// The provider's preview of `link` (already canonical), or none when no provider knows the link
/// or its provider did not answer.
pub(crate) async fn provider_preview(fetcher: &LinkFetcher, link: &Url) -> Option<ProviderPreview> {
    if let Some(youtube_link) = youtube::youtube_link_of(link) {
        return youtube::youtube_preview(fetcher, &youtube_link).await;
    }
    None
}
