//! The search results side sheet (`MessageSearchPanel.svelte`, DESIGN.md 5.12): docked beside
//! the conversation like the member list, over it in a narrow window (with its own field). The
//! count titles it; then the sort, the filters applied, the facets that narrow, and the results as
//! slips of the conversation sheet, labelled by channel, with the matched words lit.

use std::collections::HashMap;
use std::rc::Rc;

use gpui::{
    AnyElement, App, Context, ElementId, FontWeight, InteractiveElement as _, IntoElement,
    KeyDownEvent, ParentElement as _, Role, SharedString, StatefulInteractiveElement as _,
    Styled as _, Toggled, Window, div, point, prelude::FluentBuilder as _, px,
};
use mellow_client_core::shared_logic::format::display_name;
use mellow_client_core::shared_logic::format::format_calendar_day;
use mellow_client_core::shared_logic::forward_messages::forward_origin_place;
use mellow_client_core::shared_logic::markdown::{
    MarkdownSources, RoleMentionContext, parse_message_markdown,
};
use mellow_client_core::shared_logic::mentions::mentionable_people;
use mellow_client_core::shared_logic::message_search::MessageSearchStatus;
use mellow_client_core::shared_logic::search_highlights::with_search_highlights;
use mellow_client_core::shared_logic::search_query::{
    SearchFilter, SearchFilterKey, SearchHasValue, SearchQueryPart, SearchQueryProblem,
    has_searched_words, search_query_parts, searched_words_of, without_search_query_part,
};
use mellow_client_core::shared_logic::search_results::{
    is_narrowed_by, query_narrowed_to_author, query_narrowed_to_channel, starts_channel_runs,
};
use mellow_client_core::shared_logic::search_text::fold_for_search;
use mellow_protocol::{
    MAXIMUM_REACHABLE_SEARCH_RESULTS, MAXIMUM_SEARCH_QUERY_LENGTH, MAXIMUM_SEARCH_QUERY_PARTS,
    Message, MessageSearchHit, MessageSearchSort, PublicUser, Snowflake,
};

use super::MessageSearchView;
use super::field::FieldPlacement;
use crate::components::{
    Avatar, AvatarPerson, Button, ButtonSize, ButtonVariant, IconButton, focus_ring, icon,
    is_focus_visible, remembered_focus_handle, with_type,
};
use crate::home::conversation_display_name;
use crate::i18n::{Strings, format_count, strings};
use crate::messages::{
    ForwardFrame, ForwardOrigin, MessageAttachments, MessageBody, MessageMarkdown,
    full_timestamp_label, local_date_time, locale, message_summary, message_timestamp_label,
    picture_animation, picture_for_box,
};
use crate::session::now_unix_milliseconds;
use crate::sign_in_form::describe_error;
use crate::theme::icons::IconName;
use crate::theme::tokens::{
    COMPONENT_CHAT_HEADER_BUTTON_SIZE, COMPONENT_CHAT_HEADER_HEIGHT,
    COMPONENT_CHAT_HEADER_ICON_SIZE, COMPONENT_MESSAGE_BLOCKED_ICON_SIZE,
    COMPONENT_MESSAGE_HISTORY_LOAD_DISTANCE, COMPONENT_MESSAGE_SKELETON_BAR_HEIGHT,
    COMPONENT_MESSAGE_STATUS_ICON_SIZE, COMPONENT_SEARCH_FACET_HEIGHT,
    COMPONENT_SEARCH_FIELD_ICON_SIZE, COMPONENT_SEARCH_RESULT_AVATAR_SIZE,
    COMPONENT_SEARCH_RESULT_GAP, COMPONENT_SEARCH_RESULT_PADDING, COMPONENT_SEARCH_RESULT_RADIUS,
    COMPONENT_SEARCH_SORT_HEIGHT, COMPONENT_SIDE_SHEET_RADIUS, RADIUS_SM, RADIUS_XS, SPACE_2XS,
    SPACE_3XS, SPACE_MD, SPACE_SM, SPACE_XS, TYPE_BODY, TYPE_BODY_SM, TYPE_HEADING_MD,
    TYPE_MESSAGE, TYPE_MESSAGE_AUTHOR, TYPE_MESSAGE_TIMESTAMP,
};
use crate::theme::{color, length, theme};

const SKELETON_RESULT_COUNT: usize = 3;
const MAXIMUM_FACETS_SHOWN: usize = 5;
/// The skeleton's bars, as shares of the text column (`w-2/5`, `w-4/5`, `w-3/5`).
const SKELETON_BAR_SHARES: [f32; 3] = [0.4, 0.8, 0.6];

fn count(value: impl TryInto<i64>) -> i64 {
    value.try_into().unwrap_or(i64::MAX)
}

/// Where a result was written, as the panel names it.
struct Place {
    name: String,
    is_conversation: bool,
    /// The name `in:` may use; `None` for a conversation, named by id.
    filter_name: Option<String>,
}

/// The people, emoji and roles results are read against, gathered once per page of results.
struct Sources {
    people: mellow_client_core::shared_logic::mentions::MentionablePeople,
    custom_emojis: HashMap<Snowflake, mellow_protocol::CustomEmoji>,
    roles: HashMap<Snowflake, mellow_protocol::Role>,
}

impl MessageSearchView {
    fn place_of(&self, channel_id: Snowflake, cx: &App) -> Place {
        let catalogue = strings(cx);
        let state = self.session.read(cx).state();
        let guild = self
            .search
            .scope_guild_id()
            .and_then(|guild_id| state.guild(guild_id));
        if let Some(channel) = guild.and_then(|guild| {
            guild
                .channels
                .iter()
                .find(|channel| channel.id == channel_id)
        }) {
            return Place {
                name: format!("#{}", channel.name),
                is_conversation: false,
                filter_name: Some(channel.name.clone()),
            };
        }
        if let Some(conversation) = state.direct_message_channel(channel_id) {
            let current_user_id = state.current_user().map(|user| user.id);
            return Place {
                name: conversation_display_name(conversation, current_user_id, catalogue),
                is_conversation: true,
                filter_name: None,
            };
        }
        Place {
            name: catalogue.search().unknown_channel().to_owned(),
            is_conversation: guild.is_none(),
            filter_name: None,
        }
    }

    fn known_people(&self, cx: &App) -> Vec<PublicUser> {
        let state = self.session.read(cx).state();
        let mut people: Vec<PublicUser> = state.current_user_as_public_user().into_iter().collect();
        match self.search.scope_guild_id() {
            Some(guild_id) => people.extend(
                state
                    .members(guild_id)
                    .iter()
                    .map(|member| member.user.clone()),
            ),
            None => people.extend(
                state
                    .direct_message_channels()
                    .iter()
                    .flat_map(|conversation| conversation.recipients.iter().cloned()),
            ),
        }
        people
    }

    fn sources(&self, cx: &App) -> Sources {
        let state = self.session.read(cx).state();
        let mut custom_emojis = HashMap::new();
        let mut roles = HashMap::new();
        for guild in state.guilds() {
            custom_emojis.extend(guild.emojis.iter().map(|emoji| (emoji.id, emoji.clone())));
            roles.extend(guild.roles.iter().map(|role| (role.id, role.clone())));
        }
        Sources {
            people: mentionable_people(&self.known_people(cx)),
            custom_emojis,
            roles,
        }
    }

    /// Each shown result's text, parsed with its matched words, for the results not parsed yet.
    fn parse_missing_bodies(&mut self, cx: &App) {
        if self
            .search
            .hits()
            .iter()
            .all(|hit| self.bodies.contains_key(&hit.message.id))
        {
            return;
        }
        let sources = self.sources(cx);
        let scope_guild_id = self.search.scope_guild_id();
        for hit in self.search.hits() {
            if self.bodies.contains_key(&hit.message.id) {
                continue;
            }
            let message = &hit.message;
            let parsed = parse_message_markdown(
                &message.content,
                MarkdownSources {
                    people: &sources.people,
                    custom_emojis: &sources.custom_emojis,
                    role_mentions: RoleMentionContext {
                        roles: &sources.roles,
                        is_everyone_mentioned: scope_guild_id
                            .is_some_and(|guild_id| message.mention_role_ids.contains(&guild_id)),
                    },
                },
            );
            let blocks = with_search_highlights(&parsed, &message.content, &hit.highlights);
            self.bodies.insert(
                message.id,
                Rc::new(MessageBody {
                    blocks,
                    is_jumbo: false,
                    names_reader: false,
                }),
            );
        }
    }

    fn filter_chip_label(&self, filter: &SearchFilter, cx: &App) -> String {
        let catalogue = strings(cx);
        let chips = catalogue.search().filter_chip();
        let state = self.session.read(cx).state();
        let value = filter.value.as_str();
        match filter.key {
            SearchFilterKey::From | SearchFilterKey::Mentions => {
                let typed_name = value.trim_start_matches('@').trim();
                // Folded as the instance compares names (ADR-0048): "élodie" names Élodie.
                let name_key =
                    fold_for_search(&typed_name.split_whitespace().collect::<Vec<_>>().join(" "));
                let person = self
                    .search
                    .author_facets()
                    .iter()
                    .map(|facet| &facet.author)
                    .find(|author| fold_for_search(&author.username) == name_key)
                    .cloned()
                    .or_else(|| {
                        self.known_people(cx)
                            .into_iter()
                            .find(|candidate| fold_for_search(&candidate.username) == name_key)
                    });
                let shown = person
                    .as_ref()
                    .map_or(typed_name, |person| display_name(person));
                if filter.key == SearchFilterKey::From {
                    chips.from(shown)
                } else {
                    chips.mentions(shown)
                }
            }
            SearchFilterKey::In => {
                let reference = value.trim_start_matches('#');
                let guild = self
                    .search
                    .scope_guild_id()
                    .and_then(|guild_id| state.guild(guild_id));
                let channel = guild.and_then(|guild| {
                    guild.channels.iter().find(|candidate| {
                        candidate.id.raw().to_string() == reference
                            || candidate.name.to_lowercase() == reference.to_lowercase()
                    })
                });
                let conversation = state
                    .direct_message_channels()
                    .iter()
                    .find(|candidate| candidate.id.raw().to_string() == reference);
                let name = match (channel, conversation) {
                    (Some(channel), _) => format!("#{}", channel.name),
                    (None, Some(conversation)) => conversation_display_name(
                        conversation,
                        state.current_user().map(|user| user.id),
                        catalogue,
                    ),
                    (None, None) => format!("#{reference}"),
                };
                chips.r#in(&name)
            }
            SearchFilterKey::Has => {
                let has = catalogue.search().has_chip();
                match SearchHasValue::ALL
                    .into_iter()
                    .find(|candidate| candidate.as_str() == value.to_lowercase())
                {
                    Some(SearchHasValue::Link) => has.link().to_owned(),
                    Some(SearchHasValue::File) => has.file().to_owned(),
                    Some(SearchHasValue::Image) => has.image().to_owned(),
                    Some(SearchHasValue::Pinned) => has.pinned().to_owned(),
                    None => value.to_owned(),
                }
            }
            SearchFilterKey::Before | SearchFilterKey::After | SearchFilterKey::During => {
                let day = format_calendar_day(locale(catalogue), value)
                    .unwrap_or_else(|| value.to_owned());
                match filter.key {
                    SearchFilterKey::Before => chips.before(&day),
                    SearchFilterKey::After => chips.after(&day),
                    _ => chips.during(&day),
                }
            }
        }
    }

    /// A chip's ✕: the query without it, searched again; the last one empties the search.
    fn remove_filter(
        &mut self,
        start: usize,
        end: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let query = without_search_query_part(self.search.query(), start, end);
        if query.is_empty() {
            self.search.clear();
            self.after_cleared(window, cx);
            self.search.is_panel_open = true;
            self.focus_field(window, cx);
        } else {
            self.set_field_text(&query, None, window, cx);
            self.search_now(query, cx);
        }
    }

    /// A facet or a chip: the query `query`, put in the field and searched.
    fn search_from_panel(&mut self, query: String, window: &mut Window, cx: &mut Context<Self>) {
        self.set_field_text(&query, None, window, cx);
        self.search_now(query, cx);
    }

    /// The results, `width` wide when docked (`None`: as wide as the overlay gives).
    #[allow(clippy::too_many_lines)] // One panel and its states, as the web's markup.
    pub fn render_panel(
        &mut self,
        is_docked: bool,
        width: Option<f32>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let tokens = theme(cx);
        let catalogue = strings(cx);
        let search_strings = catalogue.search();
        let status = self.search.status();
        let hits_count = self.search.hits().len();
        let query = self.search.query().to_owned();
        let state = self.session.read(cx).state();
        let guild_name = self
            .search
            .scope_guild_id()
            .and_then(|guild_id| state.guild(guild_id))
            .map(|guild| guild.name.clone());

        let total = count(self.search.total_count());
        let is_lower_bound = self.search.is_total_lower_bound();
        let title: SharedString = if status == MessageSearchStatus::Loaded && hits_count > 0 {
            search_strings.result_count(total, is_lower_bound).into()
        } else {
            search_strings.title().into()
        };
        let announcement = match status {
            MessageSearchStatus::Loaded if hits_count == 0 => {
                search_strings.no_results_title().to_owned()
            }
            MessageSearchStatus::Loaded => search_strings.results_announcement(
                total,
                is_lower_bound,
                &searched_words_of(&query),
            ),
            _ => String::new(),
        };

        // A new search starts at the top of the list.
        let scrolled_for = (query.clone(), self.search.effective_sort());
        if scrolled_for != self.scrolled_for {
            self.scrolled_for = scrolled_for;
            self.results_scroll.set_offset(point(px(0.0), px(0.0)));
        }
        // The next page loads as the end of the list comes into view (and at once when it is).
        if self.search.can_load_more() && !self.search.is_loading_more() {
            let offset = self.results_scroll.offset();
            let max_offset = self.results_scroll.max_offset();
            let distance_to_end = f32::from(max_offset.y + offset.y);
            if distance_to_end <= COMPONENT_MESSAGE_HISTORY_LOAD_DISTANCE {
                cx.defer_in(window, |view, _, cx| view.load_more(cx));
            }
        }

        let body: AnyElement = match status {
            MessageSearchStatus::Idle => Self::render_idle(guild_name.as_deref(), cx),
            MessageSearchStatus::TooLarge(problem) => with_type(div(), TYPE_BODY)
                .id("search-too-large")
                .role(Role::Alert)
                .pt(length(SPACE_XS))
                .text_color(color(tokens.color_text_secondary))
                .child(match problem {
                    SearchQueryProblem::TooLong => {
                        search_strings.too_long(count(MAXIMUM_SEARCH_QUERY_LENGTH))
                    }
                    SearchQueryProblem::TooManyParts => {
                        search_strings.too_many_parts(count(MAXIMUM_SEARCH_QUERY_PARTS))
                    }
                })
                .into_any_element(),
            MessageSearchStatus::RateLimited | MessageSearchStatus::Failed => {
                let view = cx.entity().downgrade();
                div()
                    .id("search-failed")
                    .role(Role::Alert)
                    .flex()
                    .flex_col()
                    .items_start()
                    .gap(length(SPACE_SM))
                    .pt(length(SPACE_XS))
                    .when(status == MessageSearchStatus::RateLimited, |alert| {
                        alert
                            .child(
                                with_type(div(), TYPE_HEADING_MD)
                                    .text_color(color(tokens.color_text_heading))
                                    .child(search_strings.rate_limited_title()),
                            )
                            .child(
                                with_type(div(), TYPE_BODY)
                                    .text_color(color(tokens.color_text_secondary))
                                    .child(search_strings.rate_limited_body()),
                            )
                    })
                    .when(status == MessageSearchStatus::Failed, |alert| {
                        let reason = self.search.failure().map_or_else(
                            || search_strings.failed().to_owned(),
                            |error| {
                                describe_error(
                                    error,
                                    search_strings.failed(),
                                    crate::strings_pending_i18n::strings(cx),
                                )
                            },
                        );
                        alert.child(
                            with_type(div(), TYPE_BODY)
                                .text_color(color(tokens.color_text_secondary))
                                .child(reason),
                        )
                    })
                    .child(
                        Button::new("search-try-again", catalogue.common().try_again()).on_click(
                            move |_, cx| {
                                let _ = view.update(cx, MessageSearchView::retry);
                            },
                        ),
                    )
                    .into_any_element()
            }
            MessageSearchStatus::Loading => div()
                .id("search-loading")
                .role(Role::Status)
                .aria_label(search_strings.searching())
                .flex()
                .flex_col()
                .gap(length(COMPONENT_SEARCH_RESULT_GAP))
                .pt(length(SPACE_XS))
                .children((0..SKELETON_RESULT_COUNT).map(|_| skeleton_slip(cx)))
                .into_any_element(),
            MessageSearchStatus::Loaded if hits_count == 0 => {
                let filters = self.render_filters(&query, cx);
                div()
                    .flex()
                    .flex_col()
                    .gap(length(SPACE_XS))
                    .pt(length(SPACE_XS))
                    .child(
                        with_type(div(), TYPE_HEADING_MD)
                            .text_color(color(tokens.color_text_heading))
                            .child(search_strings.no_results_title()),
                    )
                    .child(
                        with_type(div(), TYPE_BODY)
                            .text_color(color(tokens.color_text_secondary))
                            .child(search_strings.no_results_body()),
                    )
                    .children(filters)
                    .into_any_element()
            }
            MessageSearchStatus::Loaded => self.render_results(&query, window, cx),
        };

        let close_view = cx.entity().downgrade();
        let field = (!is_docked).then(|| {
            div()
                .flex_none()
                .px(length(SPACE_SM))
                .pb(length(SPACE_XS))
                .child(self.render_field(FieldPlacement::Sheet, window, cx))
        });
        div()
            .id("search-results-panel")
            .role(Role::Region)
            .aria_label(search_strings.results())
            .on_key_down(cx.listener(move |view, event: &KeyDownEvent, _, cx| {
                if is_docked
                    && event.keystroke.key == "escape"
                    && !event.keystroke.modifiers.modified()
                {
                    cx.stop_propagation();
                    view.hide_panel(cx);
                }
            }))
            .h_full()
            .when_some(width, |panel, width| panel.w(length(width)))
            .when(width.is_none(), gpui::Styled::w_full)
            .flex()
            .flex_col()
            .overflow_hidden()
            .bg(color(tokens.component_side_sheet_background))
            .rounded(length(COMPONENT_SIDE_SHEET_RADIUS))
            .child(
                div()
                    .flex_none()
                    .h(length(COMPONENT_CHAT_HEADER_HEIGHT))
                    .pl(length(SPACE_MD))
                    .pr(length(SPACE_2XS))
                    .flex()
                    .items_center()
                    .gap(length(SPACE_2XS))
                    .text_color(color(tokens.color_text_heading))
                    .child(
                        with_type(div(), TYPE_HEADING_MD)
                            .id("search-results-title")
                            .role(Role::Heading)
                            .aria_level(2)
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .child(title),
                    )
                    .child(
                        IconButton::new("close-search", IconName::X, catalogue.common().close())
                            .sizes(
                                COMPONENT_CHAT_HEADER_BUTTON_SIZE,
                                COMPONENT_CHAT_HEADER_ICON_SIZE,
                            )
                            .on_click(move |window, cx| {
                                let _ = close_view.update(cx, |view, cx| view.close(window, cx));
                            }),
                    ),
            )
            .children(field)
            .child(
                div()
                    .id("search-results-announcement")
                    .role(Role::Status)
                    .aria_label(announcement)
                    .size_0()
                    .overflow_hidden(),
            )
            .when_some(self.notice.clone(), |panel, notice| {
                panel.child(
                    with_type(div(), TYPE_BODY)
                        .id("search-notice")
                        .role(Role::Status)
                        .aria_label(notice.clone())
                        .flex_none()
                        .px(length(SPACE_MD))
                        .pb(length(SPACE_XS))
                        .text_color(color(tokens.color_text_secondary))
                        .child(notice),
                )
            })
            .child(
                div()
                    .id("search-results-scroll")
                    .track_scroll(&self.results_scroll)
                    .on_scroll(cx.listener(|_, _, _, cx| cx.notify()))
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .px(length(SPACE_SM))
                    .pb(length(SPACE_MD))
                    .child(body),
            )
            .into_any_element()
    }

    fn render_idle(guild_name: Option<&str>, cx: &App) -> AnyElement {
        let tokens = theme(cx);
        let search_strings = strings(cx).search();
        let title = match guild_name {
            Some(name) => search_strings.idle_title_in_server(name),
            None => search_strings.idle_title_in_conversations().to_owned(),
        };
        div()
            .flex()
            .flex_col()
            .gap(length(SPACE_SM))
            .pt(length(SPACE_XS))
            .child(
                with_type(div(), TYPE_HEADING_MD)
                    .text_color(color(tokens.color_text_heading))
                    .child(title),
            )
            .child(
                with_type(div(), TYPE_BODY)
                    .text_color(color(tokens.color_text_secondary))
                    .child(search_strings.idle_body()),
            )
            .child(div().flex().flex_col().gap(length(SPACE_XS)).children(
                search_strings.tips().iter().map(|tip| {
                    div()
                        .flex()
                        .flex_col()
                        .items_start()
                        .gap(length(SPACE_3XS))
                        .child(
                            with_type(div(), TYPE_BODY_SM)
                                .font_family(crate::theme::MONOSPACE_FONT_FAMILY)
                                .px(length(SPACE_2XS))
                                .rounded(length(RADIUS_XS))
                                .bg(color(tokens.component_message_code_background))
                                .text_color(color(tokens.color_text_heading))
                                .child(tip.example),
                        )
                        .child(
                            with_type(div(), TYPE_BODY_SM)
                                .text_color(color(tokens.color_text_muted))
                                .child(tip.meaning),
                        )
                }),
            ))
            .into_any_element()
    }

    /// The filters applied, each with its ✕.
    fn render_filters(&self, query: &str, cx: &mut Context<Self>) -> Option<AnyElement> {
        let tokens = theme(cx);
        let search_strings = strings(cx).search();
        let applied: Vec<(SearchQueryPart, SearchFilter)> = search_query_parts(query)
            .into_iter()
            .filter_map(|part| part.filter.clone().map(|filter| (part, filter)))
            .collect();
        if applied.is_empty() {
            return None;
        }
        let chips = applied.into_iter().map(|(part, filter)| {
            let label = self.filter_chip_label(&filter, cx);
            let remove_label = search_strings.remove_filter(&label);
            let (start, end) = (part.start, part.end);
            div()
                .flex()
                .items_center()
                .max_w_full()
                .min_h(length(COMPONENT_SEARCH_FACET_HEIGHT))
                .gap(length(SPACE_2XS))
                .pl(length(SPACE_XS))
                .rounded_full()
                .bg(color(tokens.component_search_filter_background))
                .text_color(color(tokens.component_search_filter_text))
                .child(
                    with_type(div(), TYPE_BODY_SM)
                        .min_w_0()
                        .truncate()
                        .child(label),
                )
                .child(
                    IconButton::new(
                        ElementId::named_usize("search-remove-filter", start),
                        IconName::X,
                        remove_label,
                    )
                    .sizes(
                        COMPONENT_SEARCH_FACET_HEIGHT,
                        COMPONENT_SEARCH_FIELD_ICON_SIZE,
                    )
                    .on_click({
                        let view = cx.entity().downgrade();
                        move |window, cx| {
                            let _ = view.update(cx, |view, cx| {
                                view.remove_filter(start, end, window, cx);
                            });
                        }
                    }),
                )
        });
        Some(
            div()
                .id("search-applied-filters")
                .role(Role::List)
                .aria_label(search_strings.applied_filters())
                .mt(length(SPACE_SM))
                .flex()
                .flex_wrap()
                .gap(length(SPACE_2XS))
                .children(chips)
                .into_any_element(),
        )
    }

    fn render_sort(&self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let tokens = theme(cx);
        let search_strings = strings(cx).search();
        let options = [
            (MessageSearchSort::Relevance, search_strings.relevance()),
            (MessageSearchSort::Newest, search_strings.newest()),
        ];
        let chosen = self.search.effective_sort();
        let radios = options
            .into_iter()
            .enumerate()
            .map(|(index, (sort, label))| {
                let is_chosen = chosen == sort;
                let id = ElementId::named_usize("search-sort", index);
                let focus_handle = remembered_focus_handle(&id, window, cx);
                let is_ring_shown = is_focus_visible(&focus_handle, window);
                let other = options[(index + 1) % options.len()].0;
                div()
                    .id(id)
                    .role(Role::RadioButton)
                    .aria_label(label)
                    .aria_toggled(if is_chosen {
                        Toggled::True
                    } else {
                        Toggled::False
                    })
                    .aria_position_in_set(index + 1)
                    .aria_size_of_set(options.len())
                    .track_focus(&focus_handle)
                    .relative()
                    .h(length(COMPONENT_SEARCH_SORT_HEIGHT))
                    .flex()
                    .items_center()
                    .px(length(SPACE_XS))
                    .rounded(length(RADIUS_XS))
                    .cursor_pointer()
                    .when(is_chosen, |radio| {
                        radio
                            .bg(color(tokens.component_search_sort_selected_background))
                            .text_color(color(tokens.component_search_sort_selected_text))
                    })
                    .when(!is_chosen, |radio| {
                        radio.text_color(color(tokens.component_search_sort_text))
                    })
                    .child(
                        with_type(div(), TYPE_BODY_SM)
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(label),
                    )
                    .on_click(cx.listener(move |view, _, _, cx| view.set_sort(sort, cx)))
                    // As native radios: the arrows choose the other one.
                    .on_key_down(cx.listener(move |view, event: &KeyDownEvent, _, cx| {
                        if matches!(
                            event.keystroke.key.as_str(),
                            "left" | "right" | "up" | "down"
                        ) && !event.keystroke.modifiers.modified()
                        {
                            cx.stop_propagation();
                            view.set_sort(other, cx);
                        }
                    }))
                    .when(is_ring_shown, |radio| {
                        radio.child(focus_ring(RADIUS_XS, cx))
                    })
            });
        div()
            .id("search-sort")
            .role(Role::RadioGroup)
            .aria_label(search_strings.sort_label())
            .flex()
            .w_auto()
            .self_start()
            .gap(length(SPACE_3XS))
            .p(length(SPACE_3XS))
            .rounded(length(RADIUS_SM))
            .bg(color(tokens.component_search_facet_background))
            .children(radios)
            .into_any_element()
    }

    fn render_facet_group(
        id: &'static str,
        label: &'static str,
        facets: Vec<(String, u32, String)>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let tokens = theme(cx);
        let catalogue = strings(cx);
        let buttons = facets
            .into_iter()
            .enumerate()
            .map(|(index, (name, facet_count, query))| {
                let view = cx.entity().downgrade();
                div()
                    .id(ElementId::named_usize(id, index))
                    .role(Role::Button)
                    .aria_label(
                        catalogue
                            .search()
                            .narrow_to(&name, count(facet_count), false),
                    )
                    .flex()
                    .items_center()
                    .max_w_full()
                    .min_h(length(COMPONENT_SEARCH_FACET_HEIGHT))
                    .gap(length(SPACE_2XS))
                    .px(length(SPACE_XS))
                    .rounded_full()
                    .cursor_pointer()
                    .bg(color(tokens.component_search_facet_background))
                    .hover(|facet| facet.bg(color(tokens.component_search_facet_background_hover)))
                    .text_color(color(tokens.component_search_facet_text))
                    .child(
                        with_type(div(), TYPE_BODY_SM)
                            .min_w_0()
                            .truncate()
                            .child(name),
                    )
                    .child(
                        with_type(div(), TYPE_BODY_SM)
                            .text_color(color(tokens.component_search_facet_count_text))
                            .child(format_count(catalogue.language(), f64::from(facet_count))),
                    )
                    .on_click(move |_, window, cx| {
                        let _ = view.update(cx, |view, cx| {
                            view.search_from_panel(query.clone(), window, cx);
                        });
                    })
            });
        div()
            .id(id)
            .role(Role::Group)
            .aria_label(label)
            .mt(length(SPACE_SM))
            .flex()
            .flex_col()
            .gap(length(SPACE_2XS))
            .child(
                with_type(div(), TYPE_BODY_SM)
                    .text_color(color(tokens.color_text_muted))
                    .child(label),
            )
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .gap(length(SPACE_2XS))
                    .children(buttons),
            )
            .into_any_element()
    }

    #[allow(clippy::too_many_lines)] // The sort, filters, facets, slips and the list's end.
    fn render_results(
        &mut self,
        query: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let tokens = theme(cx);
        let catalogue = strings(cx);
        let search_strings = catalogue.search();
        self.parse_missing_bodies(cx);
        let is_in_server = self.search.scope_guild_id().is_some();

        let sort = has_searched_words(query).then(|| self.render_sort(window, cx));
        let filters = self.render_filters(query, cx);
        let channel_facets: Vec<(String, u32, String)> = if !is_narrowed_by(
            query,
            SearchFilterKey::In,
        ) && self.search.channel_facets().len()
            > 1
        {
            self.search
                .channel_facets()
                .iter()
                .take(MAXIMUM_FACETS_SHOWN)
                .map(|facet| {
                    let place = self.place_of(facet.channel_id, cx);
                    let narrowed = query_narrowed_to_channel(
                        query,
                        facet.channel_id,
                        place.filter_name.as_deref(),
                    );
                    (place.name, facet.count, narrowed)
                })
                .collect()
        } else {
            Vec::new()
        };
        let author_facets: Vec<(String, u32, String)> =
            if !is_narrowed_by(query, SearchFilterKey::From)
                && self.search.author_facets().len() > 1
            {
                self.search
                    .author_facets()
                    .iter()
                    .take(MAXIMUM_FACETS_SHOWN)
                    .map(|facet| {
                        (
                            display_name(&facet.author).to_owned(),
                            facet.count,
                            query_narrowed_to_author(query, &facet.author.username),
                        )
                    })
                    .collect()
            } else {
                Vec::new()
            };
        let channel_facet_group = (!channel_facets.is_empty()).then(|| {
            Self::render_facet_group(
                "search-channel-facets",
                if is_in_server {
                    search_strings.narrow_by_channel()
                } else {
                    search_strings.narrow_by_conversation()
                },
                channel_facets,
                cx,
            )
        });
        let author_facet_group = (!author_facets.is_empty()).then(|| {
            Self::render_facet_group(
                "search-author-facets",
                search_strings.narrow_by_person(),
                author_facets,
                cx,
            )
        });

        let hits: Vec<MessageSearchHit> = self.search.hits().to_vec();
        let run_starts = starts_channel_runs(&hits);
        let now = now_unix_milliseconds();
        let mut rows: Vec<AnyElement> = Vec::with_capacity(hits.len());
        for (index, (hit, starts_run)) in hits.iter().zip(run_starts).enumerate() {
            let place = self.place_of(hit.message.channel_id, cx);
            let slip = self.render_slip(hit, &place, now, window, cx);
            rows.push(
                div()
                    .flex()
                    .flex_col()
                    .when(index > 0, |row| {
                        row.mt(length(if starts_run {
                            SPACE_MD
                        } else {
                            COMPONENT_SEARCH_RESULT_GAP
                        }))
                    })
                    .when(starts_run, |row| {
                        row.child(
                            with_type(div(), TYPE_BODY_SM)
                                .mb(length(SPACE_2XS))
                                .pl(length(SPACE_2XS))
                                .flex()
                                .items_center()
                                .gap(length(SPACE_2XS))
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(color(tokens.component_search_channel_label_text))
                                .child(icon(
                                    if place.is_conversation {
                                        IconName::At
                                    } else {
                                        IconName::Hash
                                    },
                                    COMPONENT_SEARCH_FIELD_ICON_SIZE,
                                    color(tokens.component_search_channel_label_text),
                                ))
                                .child(div().min_w_0().truncate().child(
                                    if place.is_conversation {
                                        place.name.clone()
                                    } else {
                                        place.name.trim_start_matches('#').to_owned()
                                    },
                                )),
                        )
                    })
                    .child(slip)
                    .into_any_element(),
            );
        }

        let list_end: Option<AnyElement> = if self.search.is_loading_more() {
            Some(
                div()
                    .id("search-loading-more")
                    .role(Role::Status)
                    .aria_label(search_strings.loading_more())
                    .mt(length(COMPONENT_SEARCH_RESULT_GAP))
                    .child(skeleton_slip(cx))
                    .into_any_element(),
            )
        } else if let Some(error) = self.search.load_more_error() {
            let view = cx.entity().downgrade();
            Some(
                div()
                    .id("search-load-more-failed")
                    .role(Role::Alert)
                    .mt(length(SPACE_SM))
                    .flex()
                    .flex_col()
                    .items_start()
                    .gap(length(SPACE_XS))
                    .child(
                        with_type(div(), TYPE_BODY)
                            .text_color(color(tokens.color_text_secondary))
                            .child(describe_error(
                                error,
                                search_strings.load_more_failed(),
                                crate::strings_pending_i18n::strings(cx),
                            )),
                    )
                    .child(
                        Button::new("search-load-more-again", catalogue.common().try_again())
                            .on_click(move |_, cx| {
                                let _ = view.update(cx, MessageSearchView::retry);
                            }),
                    )
                    .into_any_element(),
            )
        } else if self.search.has_reached_limit() {
            Some(
                with_type(div(), TYPE_BODY_SM)
                    .mt(length(SPACE_SM))
                    .text_color(color(tokens.color_text_muted))
                    .child(search_strings.limit_reached(count(MAXIMUM_REACHABLE_SEARCH_RESULTS)))
                    .into_any_element(),
            )
        } else {
            None
        };

        div()
            .flex()
            .flex_col()
            .children(sort)
            .children(filters)
            .children(channel_facet_group)
            .children(author_facet_group)
            .child(
                div()
                    .id("search-results-list")
                    .role(Role::List)
                    .aria_label(search_strings.results())
                    .mt(length(SPACE_SM))
                    .flex()
                    .flex_col()
                    .children(rows),
            )
            .children(list_end)
            .into_any_element()
    }

    #[allow(clippy::too_many_lines)] // One slip: folded, or the message as the list shows it.
    fn render_slip(
        &self,
        hit: &MessageSearchHit,
        place: &Place,
        now: u64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let tokens = theme(cx);
        let catalogue = strings(cx);
        let message = &hit.message;
        let message_id = message.id;
        let channel_id = message.channel_id;
        let state = self.session.read(cx).state();
        if state.is_blocked(message.author.id) && !self.revealed_blocked_ids.contains(&message_id) {
            return div()
                .flex()
                .flex_wrap()
                .items_center()
                .gap_x(length(SPACE_XS))
                .rounded(length(COMPONENT_SEARCH_RESULT_RADIUS))
                .bg(color(tokens.component_search_result_background))
                .px(length(COMPONENT_SEARCH_RESULT_PADDING))
                .py(length(SPACE_XS))
                .text_color(color(tokens.component_message_blocked_text))
                .child(icon(
                    IconName::Prohibit,
                    COMPONENT_MESSAGE_BLOCKED_ICON_SIZE,
                    color(tokens.component_message_blocked_text),
                ))
                .child(with_type(div(), TYPE_BODY).child(catalogue.messages().blocked_run(1)))
                .child(
                    Button::new(
                        ElementId::named_usize(
                            "search-show-blocked",
                            usize::try_from(message_id.raw()).unwrap_or_default(),
                        ),
                        catalogue.messages().show_blocked(),
                    )
                    .variant(ButtonVariant::Ghost)
                    .size(ButtonSize::Small)
                    .on_click({
                        let view = cx.entity().downgrade();
                        move |_, cx| {
                            let _ = view.update(cx, |view, cx| {
                                view.revealed_blocked_ids.insert(message_id);
                                cx.notify();
                            });
                        }
                    }),
                )
                .into_any_element();
        }

        let author_name = display_name(&message.author).to_owned();
        let sent = local_date_time(message_id.unix_milliseconds());
        let full_time: SharedString = full_timestamp_label(sent, catalogue).into();
        let picture = message.author.avatar.as_ref().map(|avatar| {
            picture_for_box(
                mellow_client_core::DownloadableFile::ProfileImage(avatar.id),
                COMPONENT_SEARCH_RESULT_AVATAR_SIZE,
                COMPONENT_SEARCH_RESULT_AVATAR_SIZE,
                picture_animation(cx),
                window,
            )
        });
        let content = self.slip_content(message, cx);
        let content: Vec<AnyElement> = match &message.forwarded_from {
            // A forward as the list shows it (ADR-0039): its words are never read as the forwarder's.
            Some(origin) => {
                let current_user_id = state.current_user().map(|user| user.id);
                let place = forward_origin_place(
                    origin.channel_id,
                    state.guilds(),
                    state.direct_message_channels(),
                    |conversation| {
                        conversation_display_name(conversation, current_user_id, catalogue)
                    },
                );
                vec![
                    ForwardFrame::new(
                        ElementId::Name(format!("search-forward-{}", message_id.raw()).into()),
                        ForwardOrigin {
                            place: place.map(|place| place.label.into()),
                            sent_at_unix_milliseconds: origin.message_id.unix_milliseconds(),
                        },
                        now,
                        content,
                        None,
                    )
                    .into_any_element(),
                ]
            }
            None => content,
        };
        let jump_view = cx.entity().downgrade();
        let slip_view = cx.entity().downgrade();
        let slip_id = SharedString::from(format!("search-result-{}", message_id.raw()));
        div()
            .id(ElementId::Name(slip_id))
            .role(Role::Article)
            .aria_label(catalogue.messages().author_said(&author_name, ""))
            .flex()
            .gap(length(SPACE_SM))
            .p(length(COMPONENT_SEARCH_RESULT_PADDING))
            .rounded(length(COMPONENT_SEARCH_RESULT_RADIUS))
            .bg(color(tokens.component_search_result_background))
            .cursor_pointer()
            // A click on the slip itself opens it; its links, spoilers and buttons keep their own.
            .on_click(move |_, _, cx| {
                let _ =
                    slip_view.update(cx, |view, cx| view.open_result(channel_id, message_id, cx));
            })
            .child(Avatar::new(
                AvatarPerson::from_user(&message.author, picture),
                COMPONENT_SEARCH_RESULT_AVATAR_SIZE,
            ))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_w_0()
                    .gap(length(SPACE_3XS))
                    .child(
                        div()
                            .flex()
                            .items_start()
                            .gap(length(SPACE_XS))
                            .child(
                                div()
                                    .flex()
                                    .flex_1()
                                    .min_w_0()
                                    .flex_wrap()
                                    .items_baseline()
                                    .gap_x(length(SPACE_XS))
                                    .child(
                                        with_type(div(), TYPE_MESSAGE_AUTHOR)
                                            .text_color(color(tokens.component_message_author_text))
                                            .child(author_name.clone()),
                                    )
                                    .child(
                                        with_type(div(), TYPE_MESSAGE_TIMESTAMP)
                                            .id(SharedString::from(format!(
                                                "search-time-{}",
                                                message_id.raw()
                                            )))
                                            .aria_label(SharedString::from(format!(
                                                "{full_time}, {}",
                                                place.name
                                            )))
                                            .text_color(color(
                                                tokens.component_message_timestamp_text,
                                            ))
                                            .child(message_timestamp_label(
                                                sent,
                                                local_date_time(now),
                                                catalogue,
                                            )),
                                    ),
                            )
                            // The keyboard's way in; the slip itself answers a click.
                            .child(
                                Button::new(
                                    SharedString::from(format!("search-jump-{}", message_id.raw())),
                                    catalogue.search().jump(),
                                )
                                .variant(ButtonVariant::Secondary)
                                .size(ButtonSize::Small)
                                .accessible_label(catalogue.search().jump_to(&author_name))
                                .on_click(move |_, cx| {
                                    let _ = jump_view.update(cx, |view, cx| {
                                        view.open_result(channel_id, message_id, cx);
                                    });
                                }),
                            ),
                    )
                    .children(content),
            )
            .into_any_element()
    }

    /// The words with their matches lit, a poll's or tracker's title, and the files.
    fn slip_content(&self, message: &Message, cx: &App) -> Vec<AnyElement> {
        let tokens = theme(cx);
        let catalogue: Strings = strings(cx);
        let mut content = Vec::new();
        if !message.content.is_empty()
            && let Some(body) = self.bodies.get(&message.id)
        {
            content.push(
                with_type(div(), TYPE_MESSAGE)
                    .text_color(color(tokens.component_message_body_text))
                    .child(MessageMarkdown::new(
                        ElementId::Name(format!("search-body-{}", message.id.raw()).into()),
                        Rc::clone(body),
                    ))
                    .into_any_element(),
            );
        }
        // A poll's question or a tracker's title, never their votes or ticks: the slip opens the
        // message for those (ADR-0043, ADR-0047).
        let tracker_icon = if message.poll.is_some() {
            Some(IconName::ChartBarHorizontal)
        } else if message.checklist.is_some() {
            Some(IconName::ListChecks)
        } else if message.progress_bar.is_some() {
            Some(IconName::Gauge)
        } else {
            None
        };
        if let Some(tracker_icon) = tracker_icon {
            let summary = message_summary(
                message,
                &HashMap::<Snowflake, mellow_protocol::Role>::new(),
                catalogue,
            );
            content.push(
                with_type(div(), TYPE_MESSAGE)
                    .flex()
                    .items_center()
                    .gap(length(SPACE_2XS))
                    .text_color(color(tokens.component_message_body_text))
                    .child(icon(
                        tracker_icon,
                        COMPONENT_MESSAGE_STATUS_ICON_SIZE,
                        color(tokens.color_text_muted),
                    ))
                    .child(div().min_w_0().child(summary))
                    .into_any_element(),
            );
        }
        if !message.attachments.is_empty() {
            content.push(
                MessageAttachments::new(
                    ElementId::Name(format!("search-files-{}", message.id.raw()).into()),
                    message.attachments.clone(),
                )
                .into_any_element(),
            );
        }
        content
    }
}

/// A result's shape while results load.
fn skeleton_slip(cx: &App) -> AnyElement {
    let tokens = theme(cx);
    let fill = color(tokens.component_message_skeleton_fill);
    div()
        .flex()
        .gap(length(SPACE_SM))
        .p(length(COMPONENT_SEARCH_RESULT_PADDING))
        .rounded(length(COMPONENT_SEARCH_RESULT_RADIUS))
        .bg(color(tokens.component_search_result_background))
        .child(
            div()
                .flex_none()
                .size(length(COMPONENT_SEARCH_RESULT_AVATAR_SIZE))
                .rounded_full()
                .bg(fill),
        )
        .child(
            div()
                .flex()
                .flex_col()
                .flex_1()
                .gap(length(SPACE_XS))
                .pt(length(SPACE_2XS))
                .children(SKELETON_BAR_SHARES.iter().map(|share| {
                    div()
                        .h(length(COMPONENT_MESSAGE_SKELETON_BAR_HEIGHT))
                        .w(gpui::relative(*share))
                        .rounded_full()
                        .bg(fill)
                })),
        )
        .into_any_element()
}
