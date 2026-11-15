//! `MessageLinkPreviews.svelte` (ADR-0041, ADR-0044, DESIGN.md 5.4 "Link cards"): the previews
//! the instance fetched for a message's links, under its files. A picture link alone, a YouTube
//! video as a screen, a provider's card by its kind (YouTube Music, GitHub, Wikipedia), or the
//! page card with the site's seal, title, description and picture (`link_cards`).
//!
//! Every title opens the link in the system browser, through the messages' own opener (http and
//! https only); videos play in the Mellow player window. Every text is plain; pictures come from
//! the instance through the picture pipeline, in boxes sized before they load.

use gpui::{
    AnyElement, App, ElementId, FontWeight, InteractiveElement as _, IntoElement,
    ParentElement as _, RenderOnce, Role, SharedString, StatefulInteractiveElement as _,
    Styled as _, Window, div, prelude::FluentBuilder as _,
};
use mellow_client_core::DownloadableFile;
use mellow_client_core::shared_logic::format::format_medium_date_without_repeated_year;
use mellow_client_core::shared_logic::link_previews::{
    IssueStateGlyph, IssueStateTone, LinkPreviewLayout, MAXIMUM_SHOWN_TOPICS, compact_count,
    issue_state_badge, known_link_embed_of, language_colour_of, link_host_name,
    link_preview_bar_colour, link_preview_layout, site_label_of, wikipedia_language_name,
    youtube_channel_name_of, youtube_duration_seconds_of, youtube_embed_of,
};
use mellow_protocol::{
    GitHubIssueEmbed, GitHubIssueState, GitHubRepositoryEmbed, LinkEmbed, LinkPreview, Snowflake,
    WikipediaArticleEmbed,
};

use super::attachments::picture_box;
use super::link_cards::{
    Shown, description, link_address_tooltip, page_card, page_frame, picture, picture_link,
    screen_card, site_label, thumbnail_box, title_link,
};
use super::rich_text::open_message_link;
use super::time::{local_date_time, locale};
use crate::components::{
    Button, ButtonSize, ButtonVariant, Tooltip, child_id, focus_ring, icon, is_focus_visible,
    remembered_focus_handle, with_type,
};
use crate::i18n;
use crate::theme::icons::IconName;
use crate::theme::tokens::{
    COMPONENT_ATTACHMENT_GAP, COMPONENT_ATTACHMENT_IMAGE_RADIUS,
    COMPONENT_LINK_PREVIEW_BORDER_WIDTH, COMPONENT_LINK_PREVIEW_COVER_RADIUS,
    COMPONENT_LINK_PREVIEW_COVER_SIZE, COMPONENT_LINK_PREVIEW_DESCRIPTION_LINES,
    COMPONENT_LINK_PREVIEW_GAP, COMPONENT_LINK_PREVIEW_LANGUAGE_DOT_SIZE,
    COMPONENT_LINK_PREVIEW_OWNER_AVATAR_SIZE, COMPONENT_LINK_PREVIEW_PICTURE_RADIUS,
    COMPONENT_LINK_PREVIEW_PLAY_BUTTON_ICON_SIZE, COMPONENT_LINK_PREVIEW_STAT_GAP,
    COMPONENT_LINK_PREVIEW_STAT_ICON_SIZE, COMPONENT_LINK_PREVIEW_STATE_BADGE_HEIGHT,
    COMPONENT_LINK_PREVIEW_STATE_BADGE_ICON_SIZE, COMPONENT_LINK_PREVIEW_STATE_BADGE_RADIUS,
    COMPONENT_LINK_PREVIEW_TEXT_GAP, COMPONENT_LINK_PREVIEW_TOPIC_PADDING_BLOCK,
    COMPONENT_LINK_PREVIEW_TOPIC_PADDING_INLINE, COMPONENT_LINK_PREVIEW_TOPIC_RADIUS,
    COMPONENT_LINK_PREVIEW_WIKIPEDIA_EXTRACT_LINES, FONT_WEIGHT_SEMIBOLD, SPACE_2XS, SPACE_3XS,
    SPACE_XS, TYPE_BODY, TYPE_BODY_LG, TYPE_BODY_SM, ThemeTokens,
};
use crate::theme::{color, length, theme};

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

/// Which card a preview takes (`MessageLinkPreviews.svelte`'s branches, in their order).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CardKind {
    GitHubRepository,
    GitHubIssue { is_pull_request: bool },
    Wikipedia,
    PictureOnly,
    Music,
    Screen,
    Page,
}

/// `None` for a preview with nothing worth a card.
fn card_kind_of(
    embed: Option<&LinkEmbed>,
    layout: LinkPreviewLayout,
    is_youtube: bool,
) -> Option<CardKind> {
    Some(match (embed, layout) {
        (_, LinkPreviewLayout::None) => return None,
        (Some(LinkEmbed::GitHubRepository(_)), _) => CardKind::GitHubRepository,
        (Some(LinkEmbed::GitHubIssue(_)), _) => CardKind::GitHubIssue {
            is_pull_request: false,
        },
        (Some(LinkEmbed::GitHubPullRequest(_)), _) => CardKind::GitHubIssue {
            is_pull_request: true,
        },
        (Some(LinkEmbed::WikipediaArticle(_)), _) => CardKind::Wikipedia,
        (_, LinkPreviewLayout::PictureOnly) => CardKind::PictureOnly,
        (Some(LinkEmbed::YouTubeMusicTrack(_)), LinkPreviewLayout::YouTubeMusic) => CardKind::Music,
        (_, LinkPreviewLayout::YouTube) if is_youtube => CardKind::Screen,
        _ => CardKind::Page,
    })
}

impl RenderOnce for MessageLinkPreviews {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let strings = i18n::strings(cx);
        let cards: Vec<AnyElement> = self
            .previews
            .iter()
            .filter_map(|preview| {
                let youtube = youtube_embed_of(&preview.url);
                let embed = known_link_embed_of(preview);
                let layout = link_preview_layout(preview, youtube.as_ref(), embed);
                let kind = card_kind_of(embed, layout, youtube.is_some())?;
                let shown = Shown {
                    id: ElementId::NamedChild(
                        std::sync::Arc::new(self.id.clone()),
                        preview.id.to_string().into(),
                    ),
                    preview,
                    page_colour: link_preview_bar_colour(preview.theme_color),
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
                Some(match (kind, embed) {
                    (CardKind::GitHubRepository, Some(LinkEmbed::GitHubRepository(repository))) => {
                        github_repository_card(&shown, repository, window, cx)
                    }
                    (
                        CardKind::GitHubIssue { is_pull_request },
                        Some(LinkEmbed::GitHubIssue(issue) | LinkEmbed::GitHubPullRequest(issue)),
                    ) => github_issue_card(&shown, issue, is_pull_request, window, cx),
                    (CardKind::Wikipedia, Some(LinkEmbed::WikipediaArticle(article))) => {
                        wikipedia_card(&shown, article, window, cx)
                    }
                    (CardKind::PictureOnly, _) => picture_only(&shown, window, cx),
                    (CardKind::Music, Some(LinkEmbed::YouTubeMusicTrack(track))) => {
                        music_card(&shown, &track.artist, window, cx)
                    }
                    (CardKind::Screen, _) => match youtube.as_ref() {
                        Some(video) => screen_card(
                            &shown,
                            video,
                            youtube_channel_name_of(preview, embed),
                            youtube_duration_seconds_of(embed, video),
                            strings,
                            window,
                            cx,
                        ),
                        None => page_card(&shown, &site_label_of(preview), layout, window, cx),
                    },
                    _ => page_card(&shown, &site_label_of(preview), layout, window, cx),
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
            color(tokens.component_link_preview_title_text),
            None,
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
    page_frame(
        &shown.id,
        Some(label),
        vec![
            site_label(shown.page_colour, &service, None, None, tokens),
            row,
        ],
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
                    color(tokens.component_link_preview_title_text),
                    None,
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
    let mut content = vec![
        site_label(
            shown.page_colour,
            &site_label_of(preview),
            None,
            None,
            tokens,
        ),
        heading,
    ];
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
    page_frame(
        &shown.id,
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
    content.push(site_label(
        shown.page_colour,
        &site_label_of(preview),
        Some(repository_path.clone()),
        None,
        tokens,
    ));
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
        color(tokens.component_link_preview_title_text),
        None,
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
    page_frame(
        &shown.id,
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
        site_label(shown.page_colour, &wiki, None, None, tokens),
        title_link(
            child_id(&shown.id, "title"),
            &preview.url,
            preview
                .title
                .clone()
                .unwrap_or_else(|| preview.url.clone())
                .into(),
            color(tokens.component_link_preview_title_text),
            None,
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
    page_frame(
        &shown.id,
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
    use mellow_protocol::{GitHubIssueEmbed, YouTubeMusicTrackEmbed};

    use super::*;

    fn issue() -> GitHubIssueEmbed {
        GitHubIssueEmbed {
            owner: "rust-lang".to_owned(),
            repository: "rust".to_owned(),
            number: 1,
            state: GitHubIssueState::Open,
            author: String::new(),
            opened_at_unix_milliseconds: None,
            merged_at_unix_milliseconds: None,
            comments: 0,
        }
    }

    #[test]
    fn each_preview_takes_its_card_in_the_web_s_order() {
        use LinkPreviewLayout as Layout;
        assert_eq!(card_kind_of(None, Layout::None, false), None);
        assert_eq!(
            card_kind_of(
                Some(&LinkEmbed::GitHubPullRequest(issue())),
                Layout::TextOnly,
                false
            ),
            Some(CardKind::GitHubIssue {
                is_pull_request: true
            })
        );
        assert_eq!(
            card_kind_of(
                Some(&LinkEmbed::GitHubIssue(issue())),
                Layout::Thumbnail,
                false
            ),
            Some(CardKind::GitHubIssue {
                is_pull_request: false
            })
        );
        assert_eq!(
            card_kind_of(None, Layout::PictureOnly, false),
            Some(CardKind::PictureOnly)
        );
        let track = LinkEmbed::YouTubeMusicTrack(YouTubeMusicTrackEmbed {
            video_id: "dQw4w9WgXcQ".to_owned(),
            artist: "Blender".to_owned(),
            album: None,
            duration_seconds: None,
        });
        assert_eq!(
            card_kind_of(Some(&track), Layout::YouTubeMusic, true),
            Some(CardKind::Music)
        );
        // A YouTube link is a screen, whatever its embed says; a music embed on a video's
        // layout too.
        assert_eq!(
            card_kind_of(None, Layout::YouTube, true),
            Some(CardKind::Screen)
        );
        assert_eq!(
            card_kind_of(Some(&track), Layout::YouTube, true),
            Some(CardKind::Screen)
        );
        for layout in [Layout::LargePicture, Layout::Thumbnail, Layout::TextOnly] {
            assert_eq!(card_kind_of(None, layout, false), Some(CardKind::Page));
        }
    }
}
