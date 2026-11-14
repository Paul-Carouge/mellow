//! The two families of link cards (DESIGN.md 5.4, "Link cards", the web's `components/embeds/`):
//! a **page** is a piece of the ground headed by the site's seal (`PageCard`, `SiteLabel`); a
//! **screen** is a YouTube video on the media stage, its poster full-bleed (`VideoEmbed`,
//! `YouTubeMark`). On the desktop a screen plays in the Mellow player window (ADR-0061): while the
//! window plays its video, the card dims under an apricot "Playing in the player" key.

use gpui::{
    AnyElement, App, ElementId, FontWeight, Hsla, InteractiveElement as _, IntoElement, ObjectFit,
    ParentElement as _, Role, SharedString, StatefulInteractiveElement as _, Styled as _,
    StyledImage as _, Window, div, img, linear_color_stop, linear_gradient,
    prelude::FluentBuilder as _, svg,
};
use gpui_base::motion::Transition;
use mellow_client_core::DownloadableFile;
use mellow_client_core::shared_logic::link_previews::{
    LinkPreviewLayout, SiteSealInk, YouTubeEmbed, clock_duration_label, duration_parts,
    site_initial_of, site_seal_ink_on,
};
use mellow_protocol::LinkPreview;

use super::rich_text::open_message_link;
use super::{picture_animation, picture_for_box};
use crate::components::{
    Tooltip, child_id, colour_transition, easing, focus_ring, icon, is_focus_visible,
    remembered_focus_handle, remembered_hover, with_type,
};
use crate::i18n::Strings;
use crate::theme::icons::IconName;
use crate::theme::tokens::{
    COMPONENT_BUTTON_HEIGHT_SMALL, COMPONENT_LINK_PREVIEW_ACTION_ICON_SIZE,
    COMPONENT_LINK_PREVIEW_BORDER_WIDTH, COMPONENT_LINK_PREVIEW_BRAND_MARK_HEIGHT,
    COMPONENT_LINK_PREVIEW_DESCRIPTION_LINES, COMPONENT_LINK_PREVIEW_GAP,
    COMPONENT_LINK_PREVIEW_MAX_WIDTH, COMPONENT_LINK_PREVIEW_NOW_PLAYING_HEIGHT,
    COMPONENT_LINK_PREVIEW_PADDING, COMPONENT_LINK_PREVIEW_PICTURE_RADIUS,
    COMPONENT_LINK_PREVIEW_PLAY_BUTTON_ICON_SIZE, COMPONENT_LINK_PREVIEW_PLAY_BUTTON_RADIUS_LIT,
    COMPONENT_LINK_PREVIEW_PLAY_BUTTON_SIZE, COMPONENT_LINK_PREVIEW_RADIUS,
    COMPONENT_LINK_PREVIEW_SCREEN_AVATAR_SIZE, COMPONENT_LINK_PREVIEW_SCREEN_FADE_RATIO,
    COMPONENT_LINK_PREVIEW_SCREEN_PADDING, COMPONENT_LINK_PREVIEW_SEAL_RADIUS,
    COMPONENT_LINK_PREVIEW_SEAL_SIZE, COMPONENT_LINK_PREVIEW_SHORT_PLAYER_MAX_HEIGHT,
    COMPONENT_LINK_PREVIEW_START_CHIP_INSET, COMPONENT_LINK_PREVIEW_START_CHIP_RADIUS,
    COMPONENT_LINK_PREVIEW_TEXT_GAP, COMPONENT_LINK_PREVIEW_THUMBNAIL_SIZE,
    COMPONENT_LINK_PREVIEW_VIDEO_PLAYER_WIDTH, FONT_WEIGHT_SEMIBOLD, MOTION_DURATION_SETTLE,
    MOTION_EASING_SETTLE, RADIUS_FULL, RADIUS_XS, SIZE_FOCUS_RING_WIDTH, SIZE_HIT_DESKTOP_MIN,
    SPACE_2XS, SPACE_3XS, SPACE_MD, SPACE_XS, TYPE_BADGE, TYPE_BODY, TYPE_BODY_LG, TYPE_BODY_SM,
    TYPE_BUTTON, ThemeTokens,
};
use crate::theme::{
    YOUTUBE_MARK_ASPECT_RATIO, YOUTUBE_MARK_PATH, animation_duration, color, length, theme,
};
use crate::youtube_player::{
    close_youtube_player, play_in_youtube_player, youtube_player_fallback, youtube_player_link,
};

/// A video's poster box, 16:9; a Short's, 9:16.
const VIDEO_ASPECT_RATIO: f32 = 16.0 / 9.0;
const SHORT_ASPECT_RATIO: f32 = 9.0 / 16.0;
/// A Short's title wraps one line further: its card is narrow.
const SHORT_TITLE_LINES: usize = 3;
const VIDEO_TITLE_LINES: usize = 2;
/// Groups the poster with its key, so the key shows the press anywhere on the poster.
const POSTER_GROUP: &str = "link-card-poster";

/// What one preview needs to draw itself.
pub(super) struct Shown<'a> {
    pub(super) id: ElementId,
    pub(super) preview: &'a LinkPreview,
    /// The page's `theme-color` when usable, `0xRRGGBB`, for the seal.
    pub(super) page_colour: Option<u32>,
    /// The title, else the site's host name: every card has its link.
    pub(super) title: SharedString,
    pub(super) picture: Option<DownloadableFile>,
}

// ---- Page cards ----

/// A card's first line (`SiteLabel.svelte`): the site's seal (its colour and initial, decoration
/// only) and its name, `detail` after it, then `trailing` at the end.
pub(super) fn site_label(
    page_colour: Option<u32>,
    label: &str,
    detail: Option<String>,
    trailing: Option<AnyElement>,
    tokens: &ThemeTokens,
) -> AnyElement {
    let ink = page_colour.and_then(|page| {
        site_seal_ink_on(
            page,
            tokens.component_link_preview_seal_ink_dark >> 8,
            tokens.component_link_preview_seal_ink_light >> 8,
        )
        .map(|ink| (page, ink))
    });
    let (fill, text) = match ink {
        Some((page, SiteSealInk::Dark)) => (
            color((page << 8) | 0xFF),
            tokens.component_link_preview_seal_ink_dark,
        ),
        Some((page, SiteSealInk::Light)) => (
            color((page << 8) | 0xFF),
            tokens.component_link_preview_seal_ink_light,
        ),
        None => (
            color(tokens.component_link_preview_seal_fallback),
            tokens.component_link_preview_seal_fallback_text,
        ),
    };
    let label_text = match detail {
        Some(detail) => format!("{label} {detail}"),
        None => label.to_owned(),
    };
    div()
        .flex()
        .min_w_0()
        .items_center()
        .gap(length(COMPONENT_LINK_PREVIEW_GAP))
        .child(
            with_type(div(), TYPE_BADGE)
                .flex_none()
                .size(length(COMPONENT_LINK_PREVIEW_SEAL_SIZE))
                .flex()
                .items_center()
                .justify_center()
                .rounded(length(COMPONENT_LINK_PREVIEW_SEAL_RADIUS))
                .bg(fill)
                .text_color(color(text))
                .child(site_initial_of(label)),
        )
        .child(
            with_type(div(), TYPE_BODY_SM)
                .flex_1()
                .min_w_0()
                .truncate()
                .text_color(color(tokens.component_link_preview_site_text))
                .child(label_text),
        )
        .children(trailing)
        .into_any_element()
}

/// The frame provider cards share (`EmbedCard.svelte`): the ground, its border and padding, the
/// kind's content under the seal line. `label` names the card for screen readers.
pub(super) fn page_frame(
    id: &ElementId,
    label: Option<SharedString>,
    content: Vec<AnyElement>,
    tokens: &ThemeTokens,
) -> AnyElement {
    div()
        .id(child_id(id, "card"))
        .when_some(label, |card, label| {
            card.role(Role::Group).aria_label(label)
        })
        .flex()
        .flex_col()
        .gap(length(COMPONENT_LINK_PREVIEW_GAP))
        .w_full()
        .max_w(length(COMPONENT_LINK_PREVIEW_MAX_WIDTH))
        .p(length(COMPONENT_LINK_PREVIEW_PADDING))
        .rounded(length(COMPONENT_LINK_PREVIEW_RADIUS))
        .border(length(COMPONENT_LINK_PREVIEW_BORDER_WIDTH))
        .border_color(color(tokens.component_link_preview_border))
        .bg(color(tokens.component_link_preview_background))
        .children(content)
        .into_any_element()
}

pub(super) fn description(text: &str, lines: f32, tokens: &ThemeTokens) -> AnyElement {
    // Token line counts are small whole numbers.
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let lines = lines as usize;
    with_type(div(), TYPE_BODY)
        .line_clamp(lines)
        .text_color(color(tokens.component_link_preview_description_text))
        .child(text.to_owned())
        .into_any_element()
}

/// A preview's picture in a box of `width` × `height` token pixels, cropped to fill it; a
/// picture the instance cannot serve leaves the box's placeholder.
pub(super) fn picture(
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
        .rounded(length(radius))
        .bg(color(tokens.component_link_preview_picture_placeholder))
        .child(
            // Rounded itself: a parent's rounding does not clip its children's corners.
            img(source)
                .size_full()
                .rounded(length(radius))
                .object_fit(ObjectFit::Cover)
                .with_fallback(|| div().into_any_element()),
        )
        .into_any_element()
}

/// The picture's width in the thumbnail square, never above its own pixels (`thumbnailWidth`).
pub(super) fn thumbnail_box(width: u32, height: u32) -> (f32, f32) {
    #[allow(clippy::cast_precision_loss)] // Picture sides are far below 2^24.
    let (width, height) = (width as f32, height as f32);
    let shown_width = COMPONENT_LINK_PREVIEW_THUMBNAIL_SIZE
        .min(COMPONENT_LINK_PREVIEW_THUMBNAIL_SIZE * width / height)
        .min(width);
    (shown_width, shown_width * height / width)
}

/// The link's full address, shown before it opens.
pub(super) fn link_address_tooltip(
    url: &str,
) -> impl Fn(&mut Window, &mut App) -> gpui::AnyView + 'static {
    let address = SharedString::from(url.to_owned());
    move |_, cx| Tooltip::view(address.clone(), cx)
}

/// A picture that repeats the title link for the pointer only (`tabindex="-1"`, `aria-hidden`).
pub(super) fn picture_link(url: &str, picture: AnyElement, id: ElementId) -> AnyElement {
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

/// A page (`PageCard.svelte`): the seal line, the title, the description and the page's picture,
/// a hero across the top when wide, a thumbnail at the trailing top otherwise. The whole card is
/// the link and one Tab stop; it lights like a row under the pointer.
#[allow(clippy::too_many_lines)] // One card's element tree, in the order it is drawn.
pub(super) fn page_card(
    shown: &Shown<'_>,
    site: &str,
    layout: LinkPreviewLayout,
    window: &mut Window,
    cx: &mut App,
) -> AnyElement {
    let tokens = theme(cx);
    let preview = shown.preview;
    let card_id = child_id(&shown.id, "card");
    let focus = remembered_focus_handle(&card_id, window, cx);
    let hover = remembered_hover(&card_id, window, cx);
    let is_hovered = *hover.read(cx);
    let background = colour_transition(
        child_id(&card_id, "background"),
        color(if is_hovered {
            tokens.component_link_preview_card_background_hover
        } else {
            tokens.component_link_preview_background
        }),
        window,
        cx,
    );
    let image = shown.picture.zip(
        preview
            .image
            .filter(|image| image.width > 0 && image.height > 0),
    );
    let thumbnail = image
        .filter(|_| layout == LinkPreviewLayout::Thumbnail)
        .map(|(file, image)| {
            let (width, height) = thumbnail_box(image.width, image.height);
            picture(
                file,
                width,
                height,
                COMPONENT_LINK_PREVIEW_PICTURE_RADIUS,
                window,
                cx,
            )
        });
    let hero = image
        .filter(|_| layout == LinkPreviewLayout::LargePicture)
        .map(|(file, image)| {
            #[allow(clippy::cast_precision_loss)] // Picture sides are far below 2^24.
            let (width, height) = (image.width as f32, image.height as f32);
            let shown_width = width.min(COMPONENT_LINK_PREVIEW_MAX_WIDTH);
            let source = picture_for_box(
                file,
                shown_width,
                shown_width * height / width,
                picture_animation(cx),
                window,
            );
            let inner_radius =
                length(COMPONENT_LINK_PREVIEW_RADIUS - COMPONENT_LINK_PREVIEW_BORDER_WIDTH);
            // Full-bleed at its own ratio, clipped by the card's top corners.
            div()
                .w_full()
                .aspect_ratio(width / height)
                .rounded_t(inner_radius)
                .bg(color(tokens.component_link_preview_picture_placeholder))
                .child(
                    img(source)
                        .size_full()
                        .rounded_t(inner_radius)
                        .object_fit(ObjectFit::Cover)
                        .with_fallback(|| div().into_any_element()),
                )
        });
    let external = thumbnail.is_none().then(|| {
        icon(
            IconName::ArrowSquareOut,
            COMPONENT_LINK_PREVIEW_ACTION_ICON_SIZE,
            color(tokens.component_link_preview_external_icon),
        )
        .into_any_element()
    });
    let mut texts = vec![
        site_label(shown.page_colour, site, None, external, tokens),
        with_type(div(), TYPE_BODY_LG)
            .mt(length(COMPONENT_LINK_PREVIEW_TEXT_GAP))
            .font_weight(FontWeight(f32::from(FONT_WEIGHT_SEMIBOLD)))
            .text_color(color(tokens.component_link_preview_page_title_text))
            .when(is_hovered, gpui::Styled::underline)
            .child(shown.title.clone())
            .into_any_element(),
    ];
    if let Some(text) = &preview.description {
        texts.push(description(
            text,
            COMPONENT_LINK_PREVIEW_DESCRIPTION_LINES,
            tokens,
        ));
    }
    let url = preview.url.clone();
    div()
        .id(card_id)
        .role(Role::Link)
        .aria_label(shown.title.clone())
        .track_focus(&focus)
        .relative()
        .flex()
        .flex_col()
        .w_full()
        .max_w(length(COMPONENT_LINK_PREVIEW_MAX_WIDTH))
        .rounded(length(COMPONENT_LINK_PREVIEW_RADIUS))
        .border(length(COMPONENT_LINK_PREVIEW_BORDER_WIDTH))
        .border_color(color(tokens.component_link_preview_border))
        .bg(background)
        .cursor_pointer()
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
        .children(hero)
        .child(
            div()
                .flex()
                .items_start()
                .gap(length(COMPONENT_LINK_PREVIEW_GAP))
                .p(length(COMPONENT_LINK_PREVIEW_PADDING))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .flex()
                        .flex_col()
                        .gap(length(COMPONENT_LINK_PREVIEW_TEXT_GAP))
                        .children(texts),
                )
                .children(thumbnail),
        )
        .when(is_focus_visible(&focus, window), |card| {
            card.child(focus_ring(COMPONENT_LINK_PREVIEW_RADIUS, cx))
        })
        .into_any_element()
}

/// A title as a link of its own (provider cards, the screen's band), opening the browser.
pub(super) fn title_link(
    id: ElementId,
    url: &str,
    title: SharedString,
    text: Hsla,
    lines: Option<usize>,
    window: &mut Window,
    cx: &mut App,
) -> AnyElement {
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
        .text_color(text)
        .cursor_pointer()
        .when_some(lines, gpui::Styled::line_clamp)
        .when(is_hovered, gpui::Styled::underline)
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

// ---- Screen cards ----

/// What a screen's poster key does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ScreenKey {
    /// Plays the video in the player window.
    Play,
    /// The window plays this card's video: the key brings it to the front.
    ShowPlayer,
}

/// The poster key of the card for `link` while the player window shows `player_link`. The whole
/// link counts: the same video from another start or in a playlist is another request. Pressing
/// the key then sends the very request the window plays, which it brings to the front without
/// loading it again (`mellow-player`'s `page_to_load`).
pub(super) fn screen_key_of(link: &str, player_link: Option<&str>) -> ScreenKey {
    if player_link == Some(link) {
        ScreenKey::ShowPlayer
    } else {
        ScreenKey::Play
    }
}

/// The play key's name, which says everything the chips show (DESIGN.md 5.4): "Play the YouTube
/// video: {title}, by {channel}, 14 minutes 48 seconds long, starting at…, in a playlist".
pub(super) fn play_key_label(
    strings: Strings,
    video: &YouTubeEmbed,
    title: &str,
    channel: Option<&str>,
    duration_seconds: Option<u32>,
) -> String {
    let words = strings.link_previews();
    let spoken = |seconds: u32| {
        let parts = duration_parts(seconds);
        words.spoken_duration(
            i64::from(parts.hours),
            i64::from(parts.minutes),
            i64::from(parts.seconds),
        )
    };
    let mut label = if video.is_short {
        words.play_you_tube_short(title)
    } else {
        words.play_you_tube_video(title)
    };
    if let Some(channel) = channel {
        label += &words.by_channel(channel);
    }
    if let Some(duration) = duration_seconds {
        label += &words.lasting(&spoken(duration));
    }
    if let Some(start) = video.start_seconds {
        label += &words.starting_at(&spoken(start));
    }
    if video.playlist_id.is_some() {
        label += words.in_playlist();
    }
    label
}

/// A chip on the poster (playlist, start, length): decorative, the key's name says it.
fn poster_chip(text: String, glyph: Option<IconName>, tokens: &ThemeTokens) -> gpui::Div {
    let tint = color(tokens.component_link_preview_start_chip_text);
    with_type(div(), TYPE_BODY_SM)
        .absolute()
        .flex()
        .items_center()
        .gap(length(SPACE_2XS))
        .px(length(SPACE_XS))
        .py(length(SPACE_3XS))
        .rounded(length(COMPONENT_LINK_PREVIEW_START_CHIP_RADIUS))
        .bg(color(tokens.component_link_preview_start_chip_background))
        .text_color(tint)
        .children(glyph.map(|glyph| icon(glyph, COMPONENT_LINK_PREVIEW_ACTION_ICON_SIZE, tint)))
        .child(text)
}

/// The 56 px key at rest and lit: a circle settling into an apricot squircle under the pointer
/// or the focus (DESIGN.md 5.4), at once under reduced motion.
fn play_disc(
    id: &ElementId,
    is_lit: bool,
    has_poster: bool,
    window: &mut Window,
    cx: &mut App,
) -> AnyElement {
    let tokens = theme(cx);
    let settle = Transition::new(animation_duration(MOTION_DURATION_SETTLE, cx))
        .easing(easing(MOTION_EASING_SETTLE));
    let radius = gpui_base::motion::transition(
        child_id(id, "radius"),
        if is_lit {
            COMPONENT_LINK_PREVIEW_PLAY_BUTTON_RADIUS_LIT
        } else {
            COMPONENT_LINK_PREVIEW_PLAY_BUTTON_SIZE / 2.0
        },
        settle.clone(),
        window,
        cx,
    );
    // Without a poster the resting disc takes the stage's lifted tone, or it would vanish.
    let resting = if has_poster {
        tokens.component_link_preview_play_button_background
    } else {
        tokens.component_link_preview_play_button_background_hover
    };
    let fill = gpui_base::motion::transition(
        child_id(id, "fill"),
        color(if is_lit {
            tokens.component_link_preview_play_button_background_lit
        } else {
            resting
        }),
        settle.clone(),
        window,
        cx,
    );
    let glyph = gpui_base::motion::transition(
        child_id(id, "glyph"),
        color(if is_lit {
            tokens.component_link_preview_play_button_icon_lit
        } else {
            tokens.component_link_preview_play_button_icon
        }),
        settle,
        window,
        cx,
    );
    let pressed = color(tokens.component_link_preview_play_button_background_pressed);
    div()
        .id(child_id(id, "disc"))
        .size(length(COMPONENT_LINK_PREVIEW_PLAY_BUTTON_SIZE))
        .flex()
        .items_center()
        .justify_center()
        .rounded(length(radius.max(0.0)))
        .bg(fill)
        // shortcut: GPUI cannot scale a div, so the press darkens without shrinking to
        // `play-button-pressed-scale`; scale it once GPUI has element transforms.
        .group_active(POSTER_GROUP, move |style| style.bg(pressed))
        .child(icon(
            IconName::Play,
            COMPONENT_LINK_PREVIEW_PLAY_BUTTON_ICON_SIZE,
            glyph,
        ))
        .into_any_element()
}

/// "Playing in the player": the apricot pill that replaces the key while the window plays it.
fn now_playing_pill(is_hovered: bool, strings: Strings, tokens: &ThemeTokens) -> AnyElement {
    let text = color(tokens.component_link_preview_now_playing_text);
    with_type(div(), TYPE_BUTTON)
        .flex()
        .items_center()
        .gap(length(SPACE_XS))
        .h(length(COMPONENT_LINK_PREVIEW_NOW_PLAYING_HEIGHT))
        .px(length(SPACE_MD))
        .rounded(length(RADIUS_FULL))
        .bg(color(if is_hovered {
            tokens.component_link_preview_now_playing_background_hover
        } else {
            tokens.component_link_preview_now_playing_background
        }))
        .text_color(text)
        .child(icon(
            IconName::PictureInPicture,
            COMPONENT_LINK_PREVIEW_ACTION_ICON_SIZE,
            text,
        ))
        .child(strings.desktop().you_tube_playing_in_player_window())
        .into_any_element()
}

/// YouTube's mark (`YouTubeMark.svelte`): its own icon, white on the stage, a link to the video
/// for the pointer only (the title already is one for keyboards and screen readers).
fn youtube_mark(id: ElementId, url: &str, tooltip: String, tokens: &ThemeTokens) -> AnyElement {
    let url = url.to_owned();
    let tooltip = SharedString::from(tooltip);
    div()
        .id(id)
        .flex_none()
        .p(length(SPACE_2XS))
        .m(-length(SPACE_2XS))
        .cursor_pointer()
        .tooltip(move |_, cx| Tooltip::view(tooltip.clone(), cx))
        .tooltip_show_delay(Tooltip::show_delay())
        .on_click(move |_, _, cx| open_message_link(&url, cx))
        .child(
            svg()
                .path(YOUTUBE_MARK_PATH)
                .h(length(COMPONENT_LINK_PREVIEW_BRAND_MARK_HEIGHT))
                .w(length(
                    COMPONENT_LINK_PREVIEW_BRAND_MARK_HEIGHT * YOUTUBE_MARK_ASPECT_RATIO,
                ))
                .text_color(color(tokens.component_link_preview_brand_mark_color)),
        )
        .into_any_element()
}

/// "Close player" on the stage's band, in the stage's own control colours; focus goes back to the
/// play key.
fn close_player_key(
    id: ElementId,
    label: &'static str,
    play_key: gpui::FocusHandle,
    window: &mut Window,
    cx: &mut App,
) -> AnyElement {
    let tokens = theme(cx);
    let focus = remembered_focus_handle(&id, window, cx);
    let hover = remembered_hover(&id, window, cx);
    let is_hovered = *hover.read(cx);
    div()
        .id(id)
        .role(Role::Button)
        .aria_label(label)
        .track_focus(&focus)
        .relative()
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .size(length(COMPONENT_BUTTON_HEIGHT_SMALL))
        .rounded(length(RADIUS_XS))
        .when(is_hovered, |key| {
            key.bg(color(
                tokens.component_link_preview_screen_control_background_hover,
            ))
        })
        .cursor_pointer()
        .tooltip(move |_, cx| Tooltip::view(SharedString::new_static(label), cx))
        .tooltip_show_delay(Tooltip::show_delay())
        .on_hover(move |is_hovered, _, cx| {
            hover.update(cx, |remembered, cx| {
                *remembered = *is_hovered;
                cx.notify();
            });
        })
        .on_click(move |_, window, cx| {
            close_youtube_player(cx);
            play_key.focus(window, cx);
        })
        .child(icon(
            IconName::X,
            COMPONENT_LINK_PREVIEW_ACTION_ICON_SIZE,
            color(if is_hovered {
                tokens.component_link_preview_screen_control_icon_hover
            } else {
                tokens.component_link_preview_screen_control_icon
            }),
        ))
        .when(is_focus_visible(&focus, window), |key| {
            key.child(focus_ring(RADIUS_XS, cx))
        })
        .into_any_element()
}

/// A YouTube video or Short as a small screen (`VideoEmbed.svelte`): the instance's poster on the
/// media stage, fading into a band with the title, the channel and YouTube's mark. The poster is
/// the key: it plays the video in the Mellow player window, or brings that window forward while it
/// plays this video. Nothing comes from YouTube in this card.
#[allow(clippy::too_many_lines)] // One card's element tree, in the order it is drawn.
pub(super) fn screen_card(
    shown: &Shown<'_>,
    video: &YouTubeEmbed,
    channel: Option<&str>,
    duration_seconds: Option<u32>,
    strings: Strings,
    window: &mut Window,
    cx: &mut App,
) -> AnyElement {
    let tokens = theme(cx);
    let preview = shown.preview;
    let words = strings.link_previews();
    let key = screen_key_of(&preview.url, youtube_player_link(cx));
    let is_now_playing = key == ScreenKey::ShowPlayer;
    let key_label: SharedString = match key {
        ScreenKey::Play => play_key_label(strings, video, &shown.title, channel, duration_seconds),
        ScreenKey::ShowPlayer => strings.desktop().you_tube_show_player(&shown.title),
    }
    .into();
    let (box_width, aspect_ratio, title_lines) = if video.is_short {
        (
            COMPONENT_LINK_PREVIEW_SHORT_PLAYER_MAX_HEIGHT * SHORT_ASPECT_RATIO,
            SHORT_ASPECT_RATIO,
            SHORT_TITLE_LINES,
        )
    } else {
        (
            COMPONENT_LINK_PREVIEW_VIDEO_PLAYER_WIDTH,
            VIDEO_ASPECT_RATIO,
            VIDEO_TITLE_LINES,
        )
    };
    let inner_radius = length(COMPONENT_LINK_PREVIEW_RADIUS);
    let poster = shown.picture.map(|file| {
        let source = picture_for_box(
            file,
            box_width,
            box_width / aspect_ratio,
            picture_animation(cx),
            window,
        );
        // shortcut: GPUI cannot scale a picture inside its clip, so the poster does not grow to
        // `poster-hover-scale` under the pointer; add it once GPUI has element transforms.
        img(source)
            .absolute()
            .inset_0()
            .size_full()
            .rounded_t(inner_radius)
            .object_fit(ObjectFit::Cover)
            .with_fallback(|| div().into_any_element())
    });
    let has_poster = poster.is_some();
    let fade = color(tokens.component_link_preview_screen_fade);
    let key_id = child_id(&shown.id, "play");
    let focus = remembered_focus_handle(&key_id, window, cx);
    let hover = remembered_hover(&key_id, window, cx);
    let is_hovered = *hover.read(cx);
    let is_ring_shown = is_focus_visible(&focus, window);
    let is_lit = is_hovered || is_ring_shown;
    let centre = if is_now_playing {
        now_playing_pill(is_hovered, strings, tokens)
    } else {
        play_disc(&key_id, is_lit, has_poster, window, cx)
    };
    let inset = length(COMPONENT_LINK_PREVIEW_START_CHIP_INSET);
    let chips: Vec<gpui::Div> = if is_now_playing {
        Vec::new()
    } else {
        let mut chips = Vec::new();
        if video.playlist_id.is_some() {
            chips.push(
                poster_chip(
                    words.playlist().to_owned(),
                    Some(IconName::ListBullets),
                    tokens,
                )
                .left(inset)
                .top(inset),
            );
        }
        if let Some(start) = video.start_seconds {
            chips.push(
                poster_chip(words.starts_at(&clock_duration_label(start)), None, tokens)
                    .left(inset)
                    .bottom(inset),
            );
        }
        if let Some(duration) = duration_seconds {
            chips.push(
                poster_chip(clock_duration_label(duration), None, tokens)
                    .right(inset)
                    .bottom(inset),
            );
        }
        chips
    };
    let url = preview.url.clone();
    let played_video = video.clone();
    let played_title = shown.title.clone();
    let tooltip = key_label.clone();
    let poster_key = div()
        .id(key_id)
        .group(POSTER_GROUP)
        .role(Role::Button)
        .aria_label(key_label)
        .track_focus(&focus)
        .relative()
        .w_full()
        .aspect_ratio(aspect_ratio)
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
        // Enter or Space on the focused poster fire this too.
        .on_click(move |_, _, cx| play_in_youtube_player(&url, &played_video, &played_title, cx))
        .children(poster)
        // The picture sets into the stage instead of ending on a hard edge.
        .child(
            div()
                .absolute()
                .left_0()
                .right_0()
                .bottom_0()
                .h(gpui::relative(COMPONENT_LINK_PREVIEW_SCREEN_FADE_RATIO))
                .bg(linear_gradient(
                    180.0,
                    linear_color_stop(Hsla { a: 0.0, ..fade }, 0.0),
                    linear_color_stop(fade, 1.0),
                )),
        )
        .when(is_now_playing, |poster| {
            poster.child(
                div()
                    .absolute()
                    .inset_0()
                    .rounded_t(inner_radius)
                    .bg(color(tokens.component_link_preview_now_playing_scrim)),
            )
        })
        .children(chips)
        .child(centre)
        .when(is_ring_shown, |poster| {
            // Inside the poster's edge, in the stage's own focus colour.
            poster.child(
                div()
                    .absolute()
                    .inset_0()
                    .rounded_t(inner_radius)
                    .border(length(SIZE_FOCUS_RING_WIDTH))
                    .border_color(color(tokens.component_link_preview_screen_focus_ring)),
            )
        });
    let close = is_now_playing.then(|| {
        close_player_key(
            child_id(&shown.id, "close"),
            words.close_player(),
            focus.clone(),
            window,
            cx,
        )
    });
    let title_row = div()
        .flex()
        .items_start()
        .gap(length(COMPONENT_LINK_PREVIEW_GAP))
        .child(div().flex_1().min_w_0().child(title_link(
            child_id(&shown.id, "title"),
            &preview.url,
            shown.title.clone(),
            color(tokens.component_link_preview_screen_title_text),
            Some(title_lines),
            window,
            cx,
        )))
        .children(close);
    let channel_row = div()
        .flex()
        .items_center()
        .gap(length(COMPONENT_LINK_PREVIEW_GAP))
        .min_h(length(COMPONENT_LINK_PREVIEW_SCREEN_AVATAR_SIZE))
        .child(
            with_type(div(), TYPE_BODY_SM)
                .flex_1()
                .min_w_0()
                .truncate()
                .text_color(color(tokens.component_link_preview_screen_meta_text))
                .child(channel.unwrap_or_default().to_owned()),
        )
        .child(youtube_mark(
            child_id(&shown.id, "mark"),
            &preview.url,
            words.open_on_you_tube(&shown.title),
            tokens,
        ));
    let unavailable = youtube_player_fallback(&video.video_id, cx).then(|| {
        let note: SharedString = strings.desktop().you_tube_player_unavailable().into();
        with_type(div(), TYPE_BODY_SM)
            .flex()
            .items_center()
            .gap(length(SPACE_2XS))
            .text_color(color(tokens.component_link_preview_screen_error_text))
            .child(icon(
                IconName::WarningCircle,
                COMPONENT_LINK_PREVIEW_ACTION_ICON_SIZE,
                color(tokens.component_link_preview_screen_error_text),
            ))
            .child(note.clone())
            .child(crate::components::polite_live_region(
                child_id(&shown.id, "player-note"),
                note,
            ))
    });
    let group_label: SharedString = match channel {
        Some(channel) => format!("{}, {channel}", shown.title).into(),
        None => shown.title.clone(),
    };
    div()
        .id(child_id(&shown.id, "card"))
        .role(Role::Group)
        .aria_label(group_label)
        .flex()
        .flex_col()
        .map(|card| {
            if video.is_short {
                card.w(length(box_width)).max_w_full()
            } else {
                card.w_full().max_w(length(box_width))
            }
        })
        .rounded(length(COMPONENT_LINK_PREVIEW_RADIUS))
        .bg(color(tokens.component_link_preview_screen_background))
        .child(poster_key)
        .child(
            div()
                .flex()
                .flex_col()
                .gap(length(COMPONENT_LINK_PREVIEW_TEXT_GAP))
                .px(length(COMPONENT_LINK_PREVIEW_SCREEN_PADDING))
                .pb(length(COMPONENT_LINK_PREVIEW_SCREEN_PADDING))
                .child(title_row)
                .child(channel_row)
                .children(unavailable),
        )
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::i18n::Language;

    fn video() -> YouTubeEmbed {
        YouTubeEmbed {
            video_id: "dQw4w9WgXcQ".to_owned(),
            playlist_id: None,
            start_seconds: None,
            is_short: false,
        }
    }

    #[test]
    fn a_thumbnail_fits_the_square_without_upscaling() {
        assert_eq!(thumbnail_box(64, 32), (64.0, 32.0));
        assert_eq!(thumbnail_box(400, 200), (80.0, 40.0));
        assert_eq!(thumbnail_box(200, 400), (40.0, 80.0));
    }

    #[test]
    fn only_the_card_whose_very_link_the_window_plays_brings_it_forward() {
        const LINK: &str = "https://www.youtube.com/watch?v=R6MlUcmOul8";
        assert_eq!(screen_key_of(LINK, None), ScreenKey::Play);
        assert_eq!(screen_key_of(LINK, Some(LINK)), ScreenKey::ShowPlayer);
        // The same video from another start, or in a playlist, is another request.
        for other in [
            "https://www.youtube.com/watch?v=R6MlUcmOul8&t=95",
            "https://www.youtube.com/watch?v=R6MlUcmOul8&list=PLFgquLnL59alCl_2TQvOiD5Vgm1hCaGSI",
        ] {
            assert_eq!(screen_key_of(LINK, Some(other)), ScreenKey::Play);
        }
    }

    #[test]
    fn the_play_key_says_everything_the_chips_show() {
        let english = Strings::new(Language::English);
        assert_eq!(
            play_key_label(english, &video(), "Sintel", None, None),
            "Play YouTube video: Sintel"
        );
        let full = YouTubeEmbed {
            start_seconds: Some(95),
            playlist_id: Some("PLFgquLnL59alCl_2TQvOiD5Vgm1hCaGSI".to_owned()),
            ..video()
        };
        assert_eq!(
            play_key_label(english, &full, "Sintel", Some("Blender"), Some(888)),
            "Play YouTube video: Sintel, by Blender, 14 minutes 48 seconds long, \
             starting at 1 minute 35 seconds, in a playlist"
        );
        let short = YouTubeEmbed {
            is_short: true,
            ..video()
        };
        assert!(
            play_key_label(Strings::new(Language::French), &short, "Pli", None, None)
                .starts_with("Lire le Short YouTube")
        );
    }
}
