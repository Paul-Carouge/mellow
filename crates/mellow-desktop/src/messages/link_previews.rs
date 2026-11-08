//! `MessageLinkPreviews.svelte` and `components/embeds/` (ADR-0041, ADR-0044, DESIGN.md §5.4,
//! `docs/design/web-link-previews.md`, `web-provider-embeds.md`, `desktop-forwards-and-link-previews.md`):
//! the previews the instance fetched for a message's links, under its files. A picture link alone,
//! a provider's card by its kind (YouTube, YouTube Music, GitHub, Wikipedia), or the plain card
//! with the page's colour, name, title, description and picture.
//!
//! The desktop embeds no player: the play key, Listen and every title open the link in the
//! system browser, through the messages' own opener (http and https only). Every text is plain;
//! pictures come from the instance through the picture pipeline, in boxes sized before they load.

use gpui::{
    AnyElement, App, ElementId, FontWeight, Hsla, InteractiveElement as _, IntoElement, ObjectFit,
    ParentElement as _, RenderOnce, Role, SharedString, StatefulInteractiveElement as _,
    Styled as _, StyledImage as _, Window, div, img, prelude::FluentBuilder as _,
};
use mellow_client_core::DownloadableFile;
use mellow_client_core::shared_logic::format::format_medium_date_without_repeated_year;
use mellow_client_core::shared_logic::link_previews::{
    IssueStateGlyph, IssueStateTone, LinkPreviewLayout, MAXIMUM_SHOWN_TOPICS, YouTubeEmbed,
    clock_duration_label, compact_count, duration_parts, issue_state_badge, known_link_embed_of,
    language_colour_of, link_host_name, link_preview_bar_colour, link_preview_layout,
    wikipedia_language_name, youtube_embed_of,
};
use mellow_protocol::{
    GitHubIssueEmbed, GitHubIssueState, GitHubRepositoryEmbed, LinkEmbed, LinkPreview, Snowflake,
    WikipediaArticleEmbed,
};

use super::attachments::picture_box;
use super::rich_text::open_message_link;
use super::time::{local_date_time, locale};
use super::{picture_animation, picture_for_box};
use crate::components::{
    Button, ButtonSize, ButtonVariant, Tooltip, child_id, focus_ring, icon, is_focus_visible,
    remembered_focus_handle, remembered_hover, with_type,
};
use crate::i18n::{self, Strings};
use crate::theme::icons::IconName;
use crate::theme::tokens::{
    COMPONENT_ATTACHMENT_GAP, COMPONENT_ATTACHMENT_IMAGE_RADIUS, COMPONENT_LINK_PREVIEW_BAR_WIDTH,
    COMPONENT_LINK_PREVIEW_BORDER_WIDTH, COMPONENT_LINK_PREVIEW_COVER_RADIUS,
    COMPONENT_LINK_PREVIEW_COVER_SIZE, COMPONENT_LINK_PREVIEW_DESCRIPTION_LINES,
    COMPONENT_LINK_PREVIEW_GAP, COMPONENT_LINK_PREVIEW_LANGUAGE_DOT_SIZE,
    COMPONENT_LINK_PREVIEW_MAX_WIDTH, COMPONENT_LINK_PREVIEW_OWNER_AVATAR_SIZE,
    COMPONENT_LINK_PREVIEW_PADDING, COMPONENT_LINK_PREVIEW_PICTURE_RADIUS,
    COMPONENT_LINK_PREVIEW_PLAY_BUTTON_ICON_SIZE, COMPONENT_LINK_PREVIEW_PLAY_BUTTON_SIZE,
    COMPONENT_LINK_PREVIEW_RADIUS, COMPONENT_LINK_PREVIEW_SHORT_PLAYER_MAX_HEIGHT,
    COMPONENT_LINK_PREVIEW_START_CHIP_INSET, COMPONENT_LINK_PREVIEW_START_CHIP_RADIUS,
    COMPONENT_LINK_PREVIEW_STAT_GAP, COMPONENT_LINK_PREVIEW_STAT_ICON_SIZE,
    COMPONENT_LINK_PREVIEW_STATE_BADGE_HEIGHT, COMPONENT_LINK_PREVIEW_STATE_BADGE_ICON_SIZE,
    COMPONENT_LINK_PREVIEW_STATE_BADGE_RADIUS, COMPONENT_LINK_PREVIEW_TEXT_GAP,
    COMPONENT_LINK_PREVIEW_THUMBNAIL_SIZE, COMPONENT_LINK_PREVIEW_TOPIC_PADDING_BLOCK,
    COMPONENT_LINK_PREVIEW_TOPIC_PADDING_INLINE, COMPONENT_LINK_PREVIEW_TOPIC_RADIUS,
    COMPONENT_LINK_PREVIEW_VIDEO_PLAYER_WIDTH, COMPONENT_LINK_PREVIEW_WIKIPEDIA_EXTRACT_LINES,
    FONT_WEIGHT_SEMIBOLD, RADIUS_XS, SIZE_FOCUS_RING_WIDTH, SIZE_HIT_DESKTOP_MIN, SPACE_2XS,
    SPACE_3XS, SPACE_XS, TYPE_BODY, TYPE_BODY_LG, TYPE_BODY_SM, ThemeTokens,
};
use crate::theme::{color, length, theme};

/// A video's poster box, 16:9; a Short's, 9:16.
const VIDEO_ASPECT_RATIO: f32 = 16.0 / 9.0;
const SHORT_ASPECT_RATIO: f32 = 9.0 / 16.0;

/// The previews of one sent message; a preview with nothing to show takes no room.
#[derive(IntoElement)]
pub struct MessageLinkPreviews {
    id: ElementId,
    channel_id: Snowflake,
    message_id: Snowflake,
    previews: Vec<LinkPreview>,
}

impl MessageLinkPreviews {
    #[must_use]
    pub fn new(
        id: impl Into<ElementId>,
        channel_id: Snowflake,
        message_id: Snowflake,
        previews: Vec<LinkPreview>,
    ) -> Self {
        Self {
            id: id.into(),
            channel_id,
            message_id,
            previews,
        }
    }
}

impl std::fmt::Debug for MessageLinkPreviews {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("MessageLinkPreviews")
            .field("message_id", &self.message_id)
            .field("previews", &self.previews.len())
            .finish_non_exhaustive()
    }
}

/// What one preview needs to draw itself.
struct Shown<'a> {
    id: ElementId,
    preview: &'a LinkPreview,
    /// The page's colour for the pill, or the token's dusk.
    bar: Hsla,
    /// The title, else the site's host name: every card has its link.
    title: SharedString,
    picture: Option<DownloadableFile>,
}

impl RenderOnce for MessageLinkPreviews {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let tokens = theme(cx);
        let strings = i18n::strings(cx);
        let cards: Vec<AnyElement> = self
            .previews
            .iter()
            .filter_map(|preview| {
                let youtube = youtube_embed_of(&preview.url);
                let embed = known_link_embed_of(preview);
                let layout = link_preview_layout(preview, youtube.as_ref(), embed);
                if layout == LinkPreviewLayout::None {
                    return None;
                }
                let shown = Shown {
                    id: ElementId::NamedChild(
                        std::sync::Arc::new(self.id.clone()),
                        preview.id.to_string().into(),
                    ),
                    preview,
                    bar: link_preview_bar_colour(preview.theme_color).map_or_else(
                        || color(tokens.component_link_preview_bar_fallback),
                        |rgb| color((rgb << 8) | 0xFF),
                    ),
                    title: preview
                        .title
                        .clone()
                        .unwrap_or_else(|| link_host_name(&preview.url))
                        .into(),
                    picture: preview.image.is_some().then_some(
                        DownloadableFile::LinkPreviewImage {
                            channel_id: self.channel_id,
                            message_id: self.message_id,
                            preview_id: preview.id,
                        },
                    ),
                };
                Some(match (embed, layout, youtube) {
                    (Some(LinkEmbed::GitHubRepository(repository)), _, _) => {
                        github_repository_card(&shown, repository, window, cx)
                    }
                    (Some(LinkEmbed::GitHubIssue(issue)), _, _) => {
                        github_issue_card(&shown, issue, false, window, cx)
                    }
                    (Some(LinkEmbed::GitHubPullRequest(issue)), _, _) => {
                        github_issue_card(&shown, issue, true, window, cx)
                    }
                    (Some(LinkEmbed::WikipediaArticle(article)), _, _) => {
                        wikipedia_card(&shown, article, window, cx)
                    }
                    (_, LinkPreviewLayout::PictureOnly, _) => picture_only(&shown, window, cx),
                    (
                        Some(LinkEmbed::YouTubeMusicTrack(track)),
                        LinkPreviewLayout::YouTubeMusic,
                        _,
                    ) => music_card(&shown, &track.artist, window, cx),
                    (_, LinkPreviewLayout::YouTube, Some(video)) => {
                        video_card(&shown, &video, strings, window, cx)
                    }
                    _ => plain_card(&shown, layout, window, cx),
                })
            })
            .collect();
        div()
            .flex()
            .flex_col()
            .items_start()
            .gap(length(COMPONENT_LINK_PREVIEW_GAP))
            .pt(length(COMPONENT_ATTACHMENT_GAP))
            .children(cards)
    }
}

// ---- Shared parts ----

/// The frame every preview shares (`EmbedCard.svelte`): the page's colour as a pill, then the
/// kind's content. `label` names the card for screen readers.
fn card_frame(
    shown: &Shown<'_>,
    label: Option<SharedString>,
    content: Vec<AnyElement>,
    tokens: &ThemeTokens,
) -> AnyElement {
    div()
        .id(child_id(&shown.id, "card"))
        .when_some(label, |card, label| {
            card.role(Role::Group).aria_label(label)
        })
        .flex()
        .gap(length(COMPONENT_LINK_PREVIEW_GAP))
        .w_full()
        .max_w(length(COMPONENT_LINK_PREVIEW_MAX_WIDTH))
        .p(length(COMPONENT_LINK_PREVIEW_PADDING))
        .rounded(length(COMPONENT_LINK_PREVIEW_RADIUS))
        .border(length(COMPONENT_LINK_PREVIEW_BORDER_WIDTH))
        .border_color(color(tokens.component_link_preview_border))
        .bg(color(tokens.component_link_preview_background))
        // Decoration only: the page's colour is never checked for contrast (DESIGN.md 1.4).
        .child(
            div()
                .flex_none()
                .w(length(COMPONENT_LINK_PREVIEW_BAR_WIDTH))
                .rounded_full()
                .bg(shown.bar),
        )
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .gap(length(COMPONENT_LINK_PREVIEW_GAP))
                .children(content),
        )
        .into_any_element()
}

fn site_line(text: impl Into<SharedString>, tokens: &ThemeTokens) -> AnyElement {
    with_type(div(), TYPE_BODY_SM)
        .text_color(color(tokens.component_link_preview_site_text))
        .child(text.into())
        .into_any_element()
}

fn description(text: &str, lines: f32, tokens: &ThemeTokens) -> AnyElement {
    // Token line counts are small whole numbers.
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let lines = lines as usize;
    with_type(div(), TYPE_BODY)
        .line_clamp(lines)
        .text_color(color(tokens.component_link_preview_description_text))
        .child(text.to_owned())
        .into_any_element()
}

/// The card's one Tab stop: its title, opening the link in the browser.
fn title_link(
    id: ElementId,
    url: &str,
    title: SharedString,
    window: &mut Window,
    cx: &mut App,
) -> AnyElement {
    let tokens = theme(cx);
    let focus = remembered_focus_handle(&id, window, cx);
    let hover = remembered_hover(&id, window, cx);
    let is_hovered = *hover.read(cx);
    let url = url.to_owned();
    with_type(div(), TYPE_BODY_LG)
        .id(id)
        .role(Role::Link)
        .aria_label(title.clone())
        .track_focus(&focus)
        .relative()
        .min_h(length(SIZE_HIT_DESKTOP_MIN))
        .rounded(length(RADIUS_XS))
        .font_weight(FontWeight(f32::from(FONT_WEIGHT_SEMIBOLD)))
        .text_color(color(tokens.component_link_preview_title_text))
        .cursor_pointer()
        .when(is_hovered, gpui::Styled::underline)
        // The real address, as a browser's status bar shows it: the card's text is the page's own.
        .tooltip(link_address_tooltip(&url))
        .tooltip_show_delay(Tooltip::show_delay())
        .on_hover(move |is_hovered, _, cx| {
            hover.update(cx, |remembered, cx| {
                *remembered = *is_hovered;
                cx.notify();
            });
        })
        .on_click(move |_, _, cx| open_message_link(&url, cx))
        .child(title)
        .when(is_focus_visible(&focus, window), |link| {
            link.child(focus_ring(RADIUS_XS, cx))
        })
        .into_any_element()
}

/// A preview's picture in a box of `width` × `height` token pixels, cropped to fill it; a
/// picture the instance cannot serve leaves the box's placeholder, never a blank focusable hole.
fn picture(
    file: DownloadableFile,
    width: f32,
    height: f32,
    radius: f32,
    window: &mut Window,
    cx: &App,
) -> AnyElement {
    let tokens = theme(cx);
    let source = picture_for_box(file, width, height, picture_animation(cx), window);
    div()
        .flex_none()
        .w(length(width))
        .max_w_full()
        .aspect_ratio(width / height)
        .overflow_hidden()
        .rounded(length(radius))
        .bg(color(tokens.component_link_preview_picture_placeholder))
        .child(
            img(source)
                .size_full()
                .object_fit(ObjectFit::Cover)
                .with_fallback(|| div().into_any_element()),
        )
        .into_any_element()
}

/// The link's full address, shown before it opens.
fn link_address_tooltip(url: &str) -> impl Fn(&mut Window, &mut App) -> gpui::AnyView + 'static {
    let address = SharedString::from(url.to_owned());
    move |_, cx| Tooltip::view(address.clone(), cx)
}

/// A picture that repeats the title link for the pointer only (`tabindex="-1"`, `aria-hidden`).
fn picture_link(url: &str, picture: AnyElement, id: ElementId) -> AnyElement {
    let url = url.to_owned();
    div()
        .id(id)
        .flex_none()
        .max_w_full()
        .cursor_pointer()
        .tooltip(link_address_tooltip(&url))
        .tooltip_show_delay(Tooltip::show_delay())
        .on_click(move |_, _, cx| open_message_link(&url, cx))
        .child(picture)
        .into_any_element()
}

/// The picture's width in the thumbnail square, never above its own pixels (`thumbnailWidth`).
fn thumbnail_box(width: u32, height: u32) -> (f32, f32) {
    #[allow(clippy::cast_precision_loss)] // Picture sides are far below 2^24.
    let (width, height) = (width as f32, height as f32);
    let shown_width = COMPONENT_LINK_PREVIEW_THUMBNAIL_SIZE
        .min(COMPONENT_LINK_PREVIEW_THUMBNAIL_SIZE * width / height)
        .min(width);
    (shown_width, shown_width * height / width)
}

// ---- Layouts ----

/// A link that is itself a picture, shown like an attached one; it opens the link.
fn picture_only(shown: &Shown<'_>, window: &mut Window, cx: &mut App) -> AnyElement {
    let (Some(file), Some(image)) = (shown.picture, shown.preview.image) else {
        return div().into_any_element();
    };
    let (width, height) = picture_box(Some(image.width), Some(image.height));
    let name: SharedString = i18n::strings(cx)
        .link_previews()
        .picture_from(&link_host_name(&shown.preview.url))
        .into();
    let url = shown.preview.url.clone();
    let focus = remembered_focus_handle(&shown.id, window, cx);
    div()
        .id(shown.id.clone())
        .role(Role::Link)
        .aria_label(name)
        .track_focus(&focus)
        .relative()
        .max_w_full()
        .cursor_pointer()
        .tooltip(link_address_tooltip(&url))
        .tooltip_show_delay(Tooltip::show_delay())
        .on_click(move |_, _, cx| open_message_link(&url, cx))
        .child(picture(
            file,
            width,
            height,
            COMPONENT_ATTACHMENT_IMAGE_RADIUS,
            window,
            cx,
        ))
        .when(is_focus_visible(&focus, window), |link| {
            link.child(focus_ring(COMPONENT_ATTACHMENT_IMAGE_RADIUS, cx))
        })
        .into_any_element()
}

/// The plain card: site, title, description, and a thumbnail beside or a wide picture under.
fn plain_card(
    shown: &Shown<'_>,
    layout: LinkPreviewLayout,
    window: &mut Window,
    cx: &mut App,
) -> AnyElement {
    let tokens = theme(cx);
    let preview = shown.preview;
    let image = preview
        .image
        .filter(|image| image.width > 0 && image.height > 0);
    let thumbnail = (layout == LinkPreviewLayout::Thumbnail)
        .then_some(shown.picture.zip(image))
        .flatten()
        .map(|(file, image)| {
            let (width, height) = thumbnail_box(image.width, image.height);
            picture_link(
                &preview.url,
                picture(
                    file,
                    width,
                    height,
                    COMPONENT_LINK_PREVIEW_PICTURE_RADIUS,
                    window,
                    cx,
                ),
                child_id(&shown.id, "thumbnail"),
            )
        });
    let large_picture = (layout == LinkPreviewLayout::LargePicture)
        .then_some(shown.picture.zip(image))
        .flatten()
        .map(|(file, image)| {
            #[allow(clippy::cast_precision_loss)] // Picture sides are far below 2^24.
            let (width, height) = (image.width as f32, image.height as f32);
            let shown_width = width.min(COMPONENT_LINK_PREVIEW_MAX_WIDTH);
            picture_link(
                &preview.url,
                picture(
                    file,
                    shown_width,
                    shown_width * height / width,
                    COMPONENT_LINK_PREVIEW_PICTURE_RADIUS,
                    window,
                    cx,
                ),
                child_id(&shown.id, "picture"),
            )
        });
    let mut texts = Vec::new();
    if let Some(site_name) = &preview.site_name {
        texts.push(site_line(site_name.clone(), tokens));
    }
    texts.push(title_link(
        child_id(&shown.id, "title"),
        &preview.url,
        shown.title.clone(),
        window,
        cx,
    ));
    if let Some(text) = &preview.description {
        texts.push(description(
            text,
            COMPONENT_LINK_PREVIEW_DESCRIPTION_LINES,
            tokens,
        ));
    }
    let top = div()
        .flex()
        .items_start()
        .gap(length(COMPONENT_LINK_PREVIEW_GAP))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .gap(length(COMPONENT_LINK_PREVIEW_TEXT_GAP))
                .children(texts),
        )
        .children(thumbnail)
        .into_any_element();
    let mut content = vec![top];
    content.extend(large_picture);
    card_frame(shown, None, content, tokens)
}

/// A YouTube video or Short (`VideoEmbed.svelte`): the instance's poster with the play key, which
/// opens the video in the browser where the link starts it; nothing comes from YouTube here.
#[allow(clippy::too_many_lines)] // One card's element tree, in the order it is drawn.
fn video_card(
    shown: &Shown<'_>,
    video: &YouTubeEmbed,
    strings: Strings,
    window: &mut Window,
    cx: &mut App,
) -> AnyElement {
    let tokens = theme(cx);
    let preview = shown.preview;
    let words = strings.link_previews();
    let name = if video.is_short {
        words.play_you_tube_short(&shown.title)
    } else {
        words.play_you_tube_video(&shown.title)
    };
    let play_label: SharedString = match video.start_seconds {
        None => name.into(),
        Some(start) => {
            let parts = duration_parts(start);
            let spoken = words.spoken_duration(
                i64::from(parts.hours),
                i64::from(parts.minutes),
                i64::from(parts.seconds),
            );
            format!("{name}{}", words.starting_at(&spoken)).into()
        }
    };
    let (box_width, aspect_ratio) = if video.is_short {
        (
            COMPONENT_LINK_PREVIEW_SHORT_PLAYER_MAX_HEIGHT * SHORT_ASPECT_RATIO,
            SHORT_ASPECT_RATIO,
        )
    } else {
        (
            COMPONENT_LINK_PREVIEW_VIDEO_PLAYER_WIDTH,
            VIDEO_ASPECT_RATIO,
        )
    };
    let poster = shown.picture.map(|file| {
        let source = picture_for_box(
            file,
            box_width,
            box_width / aspect_ratio,
            picture_animation(cx),
            window,
        );
        img(source)
            .absolute()
            .inset_0()
            .size_full()
            .object_fit(ObjectFit::Cover)
            .with_fallback(|| div().into_any_element())
    });
    let start_chip = video.start_seconds.map(|start| {
        // Said in the play key's name; shown for the eye.
        with_type(div(), TYPE_BODY_SM)
            .absolute()
            .left(length(COMPONENT_LINK_PREVIEW_START_CHIP_INSET))
            .bottom(length(COMPONENT_LINK_PREVIEW_START_CHIP_INSET))
            .px(length(SPACE_XS))
            .py(length(SPACE_3XS))
            .rounded(length(COMPONENT_LINK_PREVIEW_START_CHIP_RADIUS))
            .bg(color(tokens.component_link_preview_start_chip_background))
            .text_color(color(tokens.component_link_preview_start_chip_text))
            .child(words.starts_at(&clock_duration_label(start)))
    });
    let key_id = child_id(&shown.id, "play");
    let focus = remembered_focus_handle(&key_id, window, cx);
    let hover = remembered_hover(&key_id, window, cx);
    let is_hovered = *hover.read(cx);
    let url = preview.url.clone();
    let tooltip = play_label.clone();
    // The whole box is the key, as on the web; the disc is opaque so it reads on any picture.
    let stage = div()
        .id(key_id)
        .role(Role::Button)
        .aria_label(play_label)
        .track_focus(&focus)
        .relative()
        .map(|stage| {
            if video.is_short {
                stage.w(length(box_width))
            } else {
                stage.w_full()
            }
        })
        .max_w_full()
        .aspect_ratio(aspect_ratio)
        .overflow_hidden()
        .rounded(length(COMPONENT_LINK_PREVIEW_PICTURE_RADIUS))
        .bg(color(tokens.component_link_preview_player_background))
        .flex()
        .items_center()
        .justify_center()
        .cursor_pointer()
        .tooltip(move |_, cx| Tooltip::view(tooltip.clone(), cx))
        .tooltip_show_delay(Tooltip::show_delay())
        .on_hover(move |is_hovered, _, cx| {
            hover.update(cx, |remembered, cx| {
                *remembered = *is_hovered;
                cx.notify();
            });
        })
        .on_click(move |_, _, cx| open_message_link(&url, cx))
        .children(poster)
        .child(
            div()
                .size(length(COMPONENT_LINK_PREVIEW_PLAY_BUTTON_SIZE))
                .flex()
                .items_center()
                .justify_center()
                .rounded_full()
                .bg(color(if is_hovered {
                    tokens.component_link_preview_play_button_background_hover
                } else {
                    tokens.component_link_preview_play_button_background
                }))
                .child(icon(
                    IconName::Play,
                    COMPONENT_LINK_PREVIEW_PLAY_BUTTON_ICON_SIZE,
                    color(tokens.component_link_preview_play_button_icon),
                )),
        )
        .children(start_chip)
        .when(is_focus_visible(&focus, window), |stage| {
            // Inside the box, in the media stage's own focus colour.
            stage.child(
                div()
                    .absolute()
                    .inset_0()
                    .rounded(length(COMPONENT_LINK_PREVIEW_PICTURE_RADIUS))
                    .border(length(SIZE_FOCUS_RING_WIDTH))
                    .border_color(color(tokens.component_link_preview_play_button_focus_ring)),
            )
        })
        .into_any_element();
    let mut texts = vec![title_link(
        child_id(&shown.id, "title"),
        &preview.url,
        shown.title.clone(),
        window,
        cx,
    )];
    if let Some(text) = &preview.description {
        texts.push(description(
            text,
            COMPONENT_LINK_PREVIEW_DESCRIPTION_LINES,
            tokens,
        ));
    }
    let mut content = Vec::new();
    if let Some(site_name) = &preview.site_name {
        content.push(site_line(site_name.clone(), tokens));
    }
    content.push(
        div()
            .flex()
            .flex_col()
            .gap(length(COMPONENT_LINK_PREVIEW_TEXT_GAP))
            .children(texts)
            .into_any_element(),
    );
    content.push(stage);
    card_frame(shown, Some(shown.title.clone()), content, tokens)
}

/// A YouTube Music track (`MusicCard.svelte`): cover, title, artist and Listen, which opens the
/// track in the browser (YouTube's terms forbid sound without its visible player).
fn music_card(shown: &Shown<'_>, artist: &str, window: &mut Window, cx: &mut App) -> AnyElement {
    let tokens = theme(cx);
    let preview = shown.preview;
    let words = i18n::strings(cx).link_previews();
    let service = preview
        .site_name
        .clone()
        .unwrap_or_else(|| "YouTube Music".to_owned());
    let cover_size = COMPONENT_LINK_PREVIEW_COVER_SIZE;
    let cover = match shown.picture {
        Some(file) => picture(
            file,
            cover_size,
            cover_size,
            COMPONENT_LINK_PREVIEW_COVER_RADIUS,
            window,
            cx,
        ),
        None => div()
            .flex_none()
            .size(length(cover_size))
            .flex()
            .items_center()
            .justify_center()
            .rounded(length(COMPONENT_LINK_PREVIEW_COVER_RADIUS))
            .bg(color(tokens.component_link_preview_cover_placeholder))
            .child(icon(
                IconName::MusicNotes,
                COMPONENT_LINK_PREVIEW_PLAY_BUTTON_ICON_SIZE,
                color(tokens.component_link_preview_cover_placeholder_icon),
            ))
            .into_any_element(),
    };
    let url = preview.url.clone();
    let listen = Button::new(child_id(&shown.id, "listen"), words.listen())
        .variant(ButtonVariant::Secondary)
        .size(ButtonSize::Small)
        .accessible_label(words.listen_on(&shown.title, artist, &service))
        .on_click(move |_, cx| open_message_link(&url, cx));
    let column = div()
        .flex_1()
        .min_w_0()
        .flex()
        .flex_col()
        .items_start()
        .gap(length(COMPONENT_LINK_PREVIEW_TEXT_GAP))
        .child(title_link(
            child_id(&shown.id, "title"),
            &preview.url,
            shown.title.clone(),
            window,
            cx,
        ))
        .child(
            with_type(div(), TYPE_BODY)
                .text_color(color(tokens.component_link_preview_description_text))
                .child(artist.to_owned()),
        )
        .child(
            div()
                .mt(length(COMPONENT_LINK_PREVIEW_TEXT_GAP))
                .child(listen),
        );
    let label: SharedString = format!("{}, {artist}, {service}", shown.title).into();
    let row = div()
        .flex()
        .items_start()
        .gap(length(COMPONENT_LINK_PREVIEW_GAP))
        .child(cover)
        .child(column)
        .into_any_element();
    card_frame(
        shown,
        Some(label),
        vec![site_line(service, tokens), row],
        tokens,
    )
}

/// One fact of a repository: a glyph and the compact figure, the exact one read out and in the
/// tooltip.
fn stat(id: ElementId, glyph: IconName, shown: String, spoken: String, cx: &App) -> AnyElement {
    let tokens = theme(cx);
    let tooltip = SharedString::from(spoken.clone());
    div()
        .id(id)
        .role(Role::Group)
        .aria_label(spoken)
        .flex()
        .items_center()
        .gap(length(SPACE_2XS))
        .tooltip(move |_, cx| Tooltip::view(tooltip.clone(), cx))
        .tooltip_show_delay(Tooltip::show_delay())
        .child(icon(
            glyph,
            COMPONENT_LINK_PREVIEW_STAT_ICON_SIZE,
            color(tokens.component_link_preview_stat_icon),
        ))
        .child(shown)
        .into_any_element()
}

fn stats_row(children: Vec<AnyElement>, tokens: &ThemeTokens) -> AnyElement {
    with_type(div(), TYPE_BODY_SM)
        .flex()
        .flex_wrap()
        .items_center()
        .gap_x(length(COMPONENT_LINK_PREVIEW_STAT_GAP))
        .gap_y(length(SPACE_3XS))
        .text_color(color(tokens.component_link_preview_stat_text))
        .children(children)
        .into_any_element()
}

/// A badge in Mellow's tones (draft outlined), never GitHub's colours.
fn state_badge(
    label: &'static str,
    glyph: Option<IconName>,
    tone: IssueStateTone,
    cx: &App,
) -> AnyElement {
    let tokens = theme(cx);
    let (background, text) = match tone {
        IssueStateTone::Open => (
            Some(tokens.component_link_preview_state_open_background),
            tokens.component_link_preview_state_open_text,
        ),
        IssueStateTone::Draft => (None, tokens.component_link_preview_state_draft_text),
        IssueStateTone::Merged => (
            Some(tokens.component_link_preview_state_merged_background),
            tokens.component_link_preview_state_merged_text,
        ),
        IssueStateTone::Completed => (
            Some(tokens.component_link_preview_state_completed_background),
            tokens.component_link_preview_state_completed_text,
        ),
        IssueStateTone::Closed => (
            Some(tokens.component_link_preview_state_closed_background),
            tokens.component_link_preview_state_closed_text,
        ),
    };
    with_type(div(), TYPE_BODY_SM)
        .flex()
        .items_center()
        .gap(length(SPACE_2XS))
        .h(length(COMPONENT_LINK_PREVIEW_STATE_BADGE_HEIGHT))
        .px(length(SPACE_XS))
        .rounded(length(COMPONENT_LINK_PREVIEW_STATE_BADGE_RADIUS))
        .font_weight(FontWeight(f32::from(FONT_WEIGHT_SEMIBOLD)))
        .text_color(color(text))
        .map(|badge| match background {
            Some(background) => badge.bg(color(background)),
            None => badge
                .border(length(COMPONENT_LINK_PREVIEW_BORDER_WIDTH))
                .border_color(color(tokens.component_link_preview_state_draft_border)),
        })
        .children(glyph.map(|glyph| {
            icon(
                glyph,
                COMPONENT_LINK_PREVIEW_STATE_BADGE_ICON_SIZE,
                color(text),
            )
        }))
        .child(label)
        .into_any_element()
}

/// A repository (`GitHubCard.svelte`): avatar, owner / name, description, topics, then
/// language, stars, forks and licence.
#[allow(clippy::too_many_lines)] // One card's element tree, in the order it is drawn.
fn github_repository_card(
    shown: &Shown<'_>,
    repository: &GitHubRepositoryEmbed,
    window: &mut Window,
    cx: &mut App,
) -> AnyElement {
    let tokens = theme(cx);
    let strings = i18n::strings(cx);
    let words = strings.provider_cards();
    let preview = shown.preview;
    let avatar_size = COMPONENT_LINK_PREVIEW_OWNER_AVATAR_SIZE;
    let avatar = match shown.picture {
        Some(file) => picture(
            file,
            avatar_size,
            avatar_size,
            COMPONENT_LINK_PREVIEW_PICTURE_RADIUS,
            window,
            cx,
        ),
        None => with_type(div(), TYPE_BODY_LG)
            .flex_none()
            .size(length(avatar_size))
            .flex()
            .items_center()
            .justify_center()
            .rounded(length(COMPONENT_LINK_PREVIEW_PICTURE_RADIUS))
            .bg(color(tokens.component_link_preview_picture_placeholder))
            .font_weight(FontWeight(f32::from(FONT_WEIGHT_SEMIBOLD)))
            .text_color(color(tokens.component_link_preview_site_text))
            .child(
                repository
                    .owner
                    .chars()
                    .next()
                    .map(|initial| initial.to_uppercase().to_string())
                    .unwrap_or_default(),
            )
            .into_any_element(),
    };
    let heading = div()
        .flex()
        .items_center()
        .gap(length(COMPONENT_LINK_PREVIEW_GAP))
        .child(avatar)
        .child(
            div()
                .min_w_0()
                .flex()
                .flex_wrap()
                .items_center()
                .gap_x(length(COMPONENT_LINK_PREVIEW_GAP))
                .child(title_link(
                    child_id(&shown.id, "title"),
                    &preview.url,
                    format!("{} / {}", repository.owner, repository.name).into(),
                    window,
                    cx,
                ))
                .when(repository.is_archived, |line| {
                    line.child(state_badge(
                        words.archived(),
                        None,
                        IssueStateTone::Draft,
                        cx,
                    ))
                }),
        )
        .into_any_element();
    let mut content = Vec::new();
    if let Some(site_name) = &preview.site_name {
        content.push(site_line(site_name.clone(), tokens));
    }
    content.push(heading);
    if let Some(text) = &preview.description {
        content.push(description(
            text,
            COMPONENT_LINK_PREVIEW_DESCRIPTION_LINES,
            tokens,
        ));
    }
    if !repository.topics.is_empty() {
        let topics = repository
            .topics
            .iter()
            .take(MAXIMUM_SHOWN_TOPICS)
            .map(|topic| {
                with_type(div(), TYPE_BODY_SM)
                    .px(length(COMPONENT_LINK_PREVIEW_TOPIC_PADDING_INLINE))
                    .py(length(COMPONENT_LINK_PREVIEW_TOPIC_PADDING_BLOCK))
                    .rounded(length(COMPONENT_LINK_PREVIEW_TOPIC_RADIUS))
                    .bg(color(tokens.component_link_preview_topic_background))
                    .text_color(color(tokens.component_link_preview_topic_text))
                    .child(topic.clone())
            });
        content.push(
            div()
                .id(child_id(&shown.id, "topics"))
                .role(Role::List)
                .aria_label(words.topics())
                .flex()
                .flex_wrap()
                .gap(length(SPACE_2XS))
                .children(topics)
                .into_any_element(),
        );
    }
    let place = locale(strings);
    let mut stats = Vec::new();
    if let Some(language) = &repository.language {
        let dot = language_colour_of(Some(language), repository.language_color).map_or_else(
            || color(tokens.component_link_preview_language_dot_fallback),
            |rgb| color((rgb << 8) | 0xFF),
        );
        stats.push(
            div()
                .id(child_id(&shown.id, "language"))
                .role(Role::Group)
                .aria_label(words.language(language))
                .flex()
                .items_center()
                .gap(length(SPACE_2XS))
                .child(
                    div()
                        .flex_none()
                        .size(length(COMPONENT_LINK_PREVIEW_LANGUAGE_DOT_SIZE))
                        .rounded_full()
                        .border_1()
                        .border_color(color(tokens.component_link_preview_language_dot_ring))
                        .bg(dot),
                )
                .child(language.clone())
                .into_any_element(),
        );
    }
    let count = |value: u64| i64::try_from(value).unwrap_or(i64::MAX);
    stats.push(stat(
        child_id(&shown.id, "stars"),
        IconName::Star,
        compact_count(place, repository.stars),
        words.stars(count(repository.stars)),
        cx,
    ));
    stats.push(stat(
        child_id(&shown.id, "forks"),
        IconName::GitFork,
        compact_count(place, repository.forks),
        words.forks(count(repository.forks)),
        cx,
    ));
    if let Some(licence) = &repository.licence_spdx {
        stats.push(stat(
            child_id(&shown.id, "licence"),
            IconName::Scales,
            licence.clone(),
            words.licence(licence),
            cx,
        ));
    }
    content.push(stats_row(stats, tokens));
    card_frame(
        shown,
        Some(
            preview
                .title
                .clone()
                .unwrap_or_else(|| preview.url.clone())
                .into(),
        ),
        content,
        tokens,
    )
}

/// An issue or pull request (`GitHubCard.svelte`): repository, state badge and number, title,
/// first lines, who opened it and when, comments.
#[allow(clippy::too_many_lines)] // One card's element tree, in the order it is drawn.
fn github_issue_card(
    shown: &Shown<'_>,
    issue: &GitHubIssueEmbed,
    is_pull_request: bool,
    window: &mut Window,
    cx: &mut App,
) -> AnyElement {
    let tokens = theme(cx);
    let strings = i18n::strings(cx);
    let words = strings.provider_cards();
    let states = words.issue_states();
    let preview = shown.preview;
    let mut content = Vec::new();
    let repository_path = format!("{}/{}", issue.owner, issue.repository);
    content.push(
        with_type(div(), TYPE_BODY_SM)
            .flex()
            .flex_wrap()
            .gap_x(length(SPACE_XS))
            .text_color(color(tokens.component_link_preview_site_text))
            .children(preview.site_name.clone())
            .child(repository_path.clone())
            .into_any_element(),
    );
    if let Some((glyph, tone)) = issue_state_badge(issue.state, is_pull_request) {
        let label = match issue.state {
            GitHubIssueState::Open => states.open(),
            GitHubIssueState::Draft => states.draft(),
            GitHubIssueState::Merged => states.merged(),
            GitHubIssueState::Completed => states.completed(),
            GitHubIssueState::NotPlanned => states.not_planned(),
            GitHubIssueState::Closed | GitHubIssueState::Unknown => states.closed(),
        };
        let glyph = match glyph {
            IssueStateGlyph::Record => IconName::Record,
            IssueStateGlyph::GitPullRequest => IconName::GitPullRequest,
            IssueStateGlyph::GitMerge => IconName::GitMerge,
            IssueStateGlyph::CheckCircle => IconName::CheckCircle,
            IssueStateGlyph::XCircle => IconName::XCircle,
        };
        content.push(
            div()
                .flex()
                .flex_wrap()
                .items_center()
                .gap_x(length(SPACE_XS))
                .child(state_badge(label, Some(glyph), tone, cx))
                .child(
                    with_type(div(), TYPE_BODY_SM)
                        .text_color(color(tokens.component_link_preview_site_text))
                        .child(format!("#{}", issue.number)),
                )
                .into_any_element(),
        );
    }
    let title = preview
        .title
        .clone()
        .unwrap_or_else(|| format!("{repository_path}#{}", issue.number));
    content.push(title_link(
        child_id(&shown.id, "title"),
        &preview.url,
        title.into(),
        window,
        cx,
    ));
    if let Some(text) = &preview.description {
        content.push(description(
            text,
            COMPONENT_LINK_PREVIEW_DESCRIPTION_LINES,
            tokens,
        ));
    }
    let place = locale(strings);
    let day_of = |milliseconds: u64| local_date_time(milliseconds).date;
    let mut facts = Vec::new();
    let opened_at = issue
        .opened_at_unix_milliseconds
        .filter(|_| !issue.author.is_empty());
    if let Some(opened_at) = opened_at {
        let date = format_medium_date_without_repeated_year(place, day_of(opened_at), None);
        facts.push(
            div()
                .child(words.opened_by(&issue.author, &date))
                .into_any_element(),
        );
    }
    if let Some(merged_at) = issue.merged_at_unix_milliseconds {
        // The opening date just before shows the year, so the line stays one line.
        let date = format_medium_date_without_repeated_year(
            place,
            day_of(merged_at),
            opened_at.map(day_of),
        );
        facts.push(div().child(words.merged_on(&date)).into_any_element());
    }
    facts.push(stat(
        child_id(&shown.id, "comments"),
        IconName::ChatCircle,
        issue.comments.to_string(),
        words.comments(i64::from(issue.comments)),
        cx,
    ));
    content.push(stats_row(facts, tokens));
    card_frame(
        shown,
        Some(
            preview
                .title
                .clone()
                .unwrap_or_else(|| preview.url.clone())
                .into(),
        ),
        content,
        tokens,
    )
}

/// A Wikipedia article (`WikipediaCard.svelte`): the wiki (and its language when it is not the
/// reader's), the title, six lines of summary and the thumbnail.
fn wikipedia_card(
    shown: &Shown<'_>,
    article: &WikipediaArticleEmbed,
    window: &mut Window,
    cx: &mut App,
) -> AnyElement {
    let tokens = theme(cx);
    let strings = i18n::strings(cx);
    let words = strings.provider_cards();
    let preview = shown.preview;
    let wiki = match wikipedia_language_name(&article.language, locale(strings)) {
        None => words.wikipedia().to_owned(),
        Some(language) => words.wikipedia_in(&language),
    };
    let summary = if article.is_disambiguation {
        Some(words.disambiguation().to_owned())
    } else {
        preview.description.clone()
    };
    let thumbnail = shown
        .picture
        .zip(
            preview
                .image
                .filter(|image| image.width > 0 && image.height > 0),
        )
        .map(|(file, image)| {
            let (width, height) = thumbnail_box(image.width, image.height);
            picture_link(
                &preview.url,
                picture(
                    file,
                    width,
                    height,
                    COMPONENT_LINK_PREVIEW_PICTURE_RADIUS,
                    window,
                    cx,
                ),
                child_id(&shown.id, "thumbnail"),
            )
        });
    let mut texts = vec![
        site_line(wiki, tokens),
        title_link(
            child_id(&shown.id, "title"),
            &preview.url,
            preview
                .title
                .clone()
                .unwrap_or_else(|| preview.url.clone())
                .into(),
            window,
            cx,
        ),
    ];
    if let Some(summary) = summary {
        texts.push(description(
            &summary,
            COMPONENT_LINK_PREVIEW_WIKIPEDIA_EXTRACT_LINES,
            tokens,
        ));
    }
    let row = div()
        .flex()
        .items_start()
        .gap(length(COMPONENT_LINK_PREVIEW_GAP))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .gap(length(COMPONENT_LINK_PREVIEW_TEXT_GAP))
                .children(texts),
        )
        .children(thumbnail)
        .into_any_element();
    card_frame(
        shown,
        Some(
            preview
                .title
                .clone()
                .unwrap_or_else(|| preview.url.clone())
                .into(),
        ),
        vec![row],
        tokens,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_thumbnail_fits_the_square_without_upscaling() {
        assert_eq!(thumbnail_box(64, 32), (64.0, 32.0));
        assert_eq!(thumbnail_box(400, 200), (80.0, 40.0));
        assert_eq!(thumbnail_box(200, 400), (40.0, 80.0));
    }
}
