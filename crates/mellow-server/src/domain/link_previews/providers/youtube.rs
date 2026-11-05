//! YouTube videos, Shorts and YouTube Music tracks through YouTube's oEmbed endpoint (ADR-0044):
//! a few hundred bytes of JSON instead of a 1.4 MB page that some servers only reach through the
//! consent wall. The poster is built from the video id on `i.ytimg.com`, never taken from an
//! answer, so the instance only ever fetches these two hosts for a YouTube link.

use mellow_protocol::{LinkEmbed, YouTubeMusicTrackEmbed, YouTubeVideoEmbed};
use url::Url;

use super::ProviderPreview;
use crate::domain::link_previews::canonical_links::{
    is_public_youtube_playlist_id, is_youtube_video_id,
};
use crate::domain::link_previews::fetcher::LinkFetcher;
use crate::domain::link_previews::page::{MAXIMUM_TITLE_CHARACTERS, PageDetails, plain_text};

const YOUTUBE_OEMBED_ENDPOINT: &str = "https://www.youtube.com/oembed";
const YOUTUBE_THUMBNAIL_ORIGIN: &str = "https://i.ytimg.com";
/// 1280 × 720, absent for some videos (404); the square art of a music track fills its middle
/// exactly, and a Short's frame fills a 9:16 crop of it.
const LARGEST_THUMBNAIL_NAME: &str = "maxresdefault.jpg";
/// 480 × 360, which every video has.
const FALLBACK_THUMBNAIL_NAME: &str = "hqdefault.jpg";
const YOUTUBE_HOST: &str = "www.youtube.com";
const YOUTUBE_MUSIC_HOST: &str = "music.youtube.com";
const YOUTUBE_SITE_NAME: &str = "YouTube";
const YOUTUBE_MUSIC_SITE_NAME: &str = "YouTube Music";
/// YouTube's generated channels for an artist's tracks are "{artist} - Topic".
const GENERATED_ARTIST_CHANNEL_SUFFIX: &str = " - Topic";

/// A canonical YouTube link (`canonical_links`), read back into its parts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct YouTubeLink {
    video_id: String,
    is_short: bool,
    is_music: bool,
    playlist_id: Option<String>,
    start_seconds: Option<u32>,
}

/// The parts of a link in one of the canonical YouTube forms; none for any other link.
pub(super) fn youtube_link_of(link: &Url) -> Option<YouTubeLink> {
    let host = link.host_str()?;
    let query_value = |name: &str| {
        link.query_pairs()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.into_owned())
    };
    let (video_id, is_short) = match (host, link.path()) {
        (YOUTUBE_HOST | YOUTUBE_MUSIC_HOST, "/watch") => (query_value("v")?, false),
        (YOUTUBE_HOST, path) => (path.strip_prefix("/shorts/")?.to_owned(), true),
        _ => return None,
    };
    if link.scheme() != "https" || !is_youtube_video_id(&video_id) {
        return None;
    }
    Some(YouTubeLink {
        video_id,
        is_short,
        is_music: host == YOUTUBE_MUSIC_HOST,
        playlist_id: query_value("list").filter(|id| is_public_youtube_playlist_id(id)),
        start_seconds: query_value("t")
            .and_then(|seconds| seconds.parse().ok())
            .filter(|seconds| *seconds > 0),
    })
}

/// The part of YouTube's oEmbed answer a card uses; the rest (its `html`, its thumbnail link) is
/// ignored.
#[derive(Debug, serde::Deserialize)]
struct OEmbedAnswer {
    title: String,
    #[serde(default)]
    author_name: Option<String>,
}

/// The video's card and embed, or none when YouTube's oEmbed does not answer (a private, removed
/// or unknown video), so the link gets the page's card as before.
pub(super) async fn youtube_preview(
    fetcher: &LinkFetcher,
    link: &YouTubeLink,
) -> Option<ProviderPreview> {
    let answer: OEmbedAnswer = fetcher.fetch_json(oembed_url(&link.video_id)?).await.ok()?;
    let title = plain_text(&answer.title, MAXIMUM_TITLE_CHARACTERS)?;
    let author = answer
        .author_name
        .and_then(|author| plain_text(&author, MAXIMUM_TITLE_CHARACTERS));
    let artist = author.as_deref().map(artist_of_channel);
    let embed = match artist.filter(|_| link.is_music) {
        Some(artist) => LinkEmbed::YouTubeMusicTrack(YouTubeMusicTrackEmbed {
            video_id: link.video_id.clone(),
            artist: artist.to_owned(),
            album: None,
            duration_seconds: None,
        }),
        None => LinkEmbed::YouTubeVideo(YouTubeVideoEmbed {
            video_id: link.video_id.clone(),
            is_short: link.is_short,
            start_seconds: link.start_seconds,
            playlist_id: link.playlist_id.clone(),
            duration_seconds: None,
        }),
    };
    let site_name = if matches!(embed, LinkEmbed::YouTubeMusicTrack(_)) {
        YOUTUBE_MUSIC_SITE_NAME
    } else {
        YOUTUBE_SITE_NAME
    };
    Some(ProviderPreview {
        details: PageDetails {
            title: Some(title),
            // The channel, or the artist of a track: what an older client's card says under
            // the title.
            description: artist.map(str::to_owned),
            site_name: Some(site_name.to_owned()),
            image_url: None,
            theme_color: None,
        },
        picture: poster(fetcher, &link.video_id).await,
        embed,
    })
}

/// "Noëlle Dupuis" for YouTube's generated channel "Noëlle Dupuis - Topic"; other channels as
/// they are.
fn artist_of_channel(channel: &str) -> &str {
    channel
        .strip_suffix(GENERATED_ARTIST_CHANNEL_SUFFIX)
        .filter(|artist| !artist.is_empty())
        .unwrap_or(channel)
}

fn oembed_url(video_id: &str) -> Option<Url> {
    let mut video = Url::parse(&format!("https://{YOUTUBE_HOST}/watch")).ok()?;
    video.query_pairs_mut().append_pair("v", video_id);
    let mut oembed = Url::parse(YOUTUBE_OEMBED_ENDPOINT).ok()?;
    oembed
        .query_pairs_mut()
        .append_pair("format", "json")
        .append_pair("url", video.as_str());
    Some(oembed)
}

/// The largest thumbnail the video has, else the one every video has.
async fn poster(fetcher: &LinkFetcher, video_id: &str) -> Option<Vec<u8>> {
    for name in [LARGEST_THUMBNAIL_NAME, FALLBACK_THUMBNAIL_NAME] {
        let Ok(url) = Url::parse(&format!("{YOUTUBE_THUMBNAIL_ORIGIN}/vi/{video_id}/{name}"))
        else {
            continue;
        };
        if let Ok(bytes) = fetcher.fetch_picture(url).await {
            return Some(bytes);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use axum::http::{StatusCode, header};

    use super::*;

    fn link(text: &str) -> Option<YouTubeLink> {
        youtube_link_of(&Url::parse(text).unwrap())
    }

    #[test]
    fn canonical_youtube_links_are_read_back_into_their_parts() {
        assert_eq!(
            link("https://www.youtube.com/watch?v=dQw4w9WgXcQ&list=PL0123456789&t=90"),
            Some(YouTubeLink {
                video_id: "dQw4w9WgXcQ".into(),
                is_short: false,
                is_music: false,
                playlist_id: Some("PL0123456789".into()),
                start_seconds: Some(90),
            })
        );
        let short = link("https://www.youtube.com/shorts/aqz-KE-bpKQ").unwrap();
        assert!(short.is_short && !short.is_music);
        let track = link("https://music.youtube.com/watch?v=lYBUbBu4W08").unwrap();
        assert!(track.is_music && !track.is_short);
        for other in [
            "https://youtu.be/dQw4w9WgXcQ",
            "http://www.youtube.com/watch?v=dQw4w9WgXcQ",
            "https://www.youtube.com/watch?v=short",
            "https://www.youtube.com/shorts/aqz-KE-bpKQ/more",
            "https://music.youtube.com/shorts/aqz-KE-bpKQ",
            "https://www.youtube.com/@channel",
            "https://example.org/watch?v=dQw4w9WgXcQ",
        ] {
            assert_eq!(link(other), None, "{other}");
        }
    }

    #[test]
    fn a_generated_topic_channel_names_its_artist() {
        assert_eq!(artist_of_channel("Noëlle Dupuis - Topic"), "Noëlle Dupuis");
        assert_eq!(artist_of_channel("RustConf"), "RustConf");
        assert_eq!(artist_of_channel(" - Topic"), " - Topic");
    }

    /// A stand-in for YouTube: oEmbed answers for `known` ids only, `maxresdefault` exists for
    /// `with_largest_thumbnail` only, `hqdefault` for every id. Records each path asked.
    async fn stand_in_youtube(
        channel: &'static str,
        with_largest_thumbnail: bool,
    ) -> (String, Arc<Mutex<Vec<String>>>) {
        let asked = Arc::new(Mutex::new(Vec::new()));
        let recorder = asked.clone();
        let site =
            axum::Router::new()
                .route(
                    "/www.youtube.com/oembed",
                    axum::routing::get(
                        move |axum::extract::Query(query): axum::extract::Query<
                            std::collections::HashMap<String, String>,
                        >| async move {
                            if query.get("url").map(String::as_str)
                                != Some("https://www.youtube.com/watch?v=dQw4w9WgXcQ")
                            {
                                return StatusCode::NOT_FOUND.into_response();
                            }
                            (
                                [(header::CONTENT_TYPE, "application/json")],
                                serde_json::json!({
                                    "title": "Les Cités d'or",
                                    "author_name": channel,
                                    "thumbnail_url": "https://elsewhere.example/poster.jpg",
                                    "html": "<iframe></iframe>",
                                })
                                .to_string(),
                            )
                                .into_response()
                        },
                    ),
                )
                .route(
                    "/i.ytimg.com/vi/{id}/{name}",
                    axum::routing::get(
                        move |axum::extract::Path((_, name)): axum::extract::Path<(
                            String,
                            String,
                        )>| async move {
                            if name == LARGEST_THUMBNAIL_NAME && !with_largest_thumbnail {
                                return StatusCode::NOT_FOUND.into_response();
                            }
                            ([(header::CONTENT_TYPE, "image/jpeg")], name).into_response()
                        },
                    ),
                )
                .layer(axum::middleware::from_fn(
                    move |request: axum::extract::Request, next: axum::middleware::Next| {
                        recorder
                            .lock()
                            .unwrap()
                            .push(request.uri().path().to_owned());
                        next.run(request)
                    },
                ));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, site).await });
        (format!("http://{address}"), asked)
    }

    use axum::response::IntoResponse as _;

    #[tokio::test]
    async fn a_video_gets_its_card_from_oembed_and_its_largest_poster() {
        let (site, asked) = stand_in_youtube("RustConf", true).await;
        let fetcher = LinkFetcher::stand_in_for_every_host_for_tests(&site);
        let video = link("https://www.youtube.com/watch?v=dQw4w9WgXcQ&t=90").unwrap();

        let preview = youtube_preview(&fetcher, &video).await.unwrap();

        assert_eq!(preview.details.title.as_deref(), Some("Les Cités d'or"));
        assert_eq!(preview.details.description.as_deref(), Some("RustConf"));
        assert_eq!(preview.details.site_name.as_deref(), Some("YouTube"));
        assert_eq!(
            preview.embed,
            LinkEmbed::YouTubeVideo(YouTubeVideoEmbed {
                video_id: "dQw4w9WgXcQ".into(),
                is_short: false,
                start_seconds: Some(90),
                playlist_id: None,
                duration_seconds: None,
            })
        );
        assert_eq!(preview.picture.as_deref(), Some(&b"maxresdefault.jpg"[..]));
        assert_eq!(
            *asked.lock().unwrap(),
            [
                "/www.youtube.com/oembed",
                "/i.ytimg.com/vi/dQw4w9WgXcQ/maxresdefault.jpg"
            ],
            "only oEmbed and the thumbnail host, never the answer's thumbnail link"
        );
    }

    #[tokio::test]
    async fn a_music_track_names_its_artist_and_falls_back_to_the_smaller_poster() {
        let (site, _) = stand_in_youtube("Noëlle Dupuis - Topic", false).await;
        let fetcher = LinkFetcher::stand_in_for_every_host_for_tests(&site);
        let track = link("https://music.youtube.com/watch?v=dQw4w9WgXcQ").unwrap();

        let preview = youtube_preview(&fetcher, &track).await.unwrap();

        assert_eq!(preview.details.site_name.as_deref(), Some("YouTube Music"));
        assert_eq!(
            preview.embed,
            LinkEmbed::YouTubeMusicTrack(YouTubeMusicTrackEmbed {
                video_id: "dQw4w9WgXcQ".into(),
                artist: "Noëlle Dupuis".into(),
                album: None,
                duration_seconds: None,
            })
        );
        assert_eq!(preview.picture.as_deref(), Some(&b"hqdefault.jpg"[..]));
    }

    #[tokio::test]
    async fn a_video_oembed_does_not_know_gets_no_provider_preview() {
        let (site, asked) = stand_in_youtube("RustConf", true).await;
        let fetcher = LinkFetcher::stand_in_for_every_host_for_tests(&site);
        let unknown = link("https://www.youtube.com/watch?v=aaaaaaaaaaa").unwrap();

        assert!(youtube_preview(&fetcher, &unknown).await.is_none());
        assert_eq!(*asked.lock().unwrap(), ["/www.youtube.com/oembed"]);
    }
}
