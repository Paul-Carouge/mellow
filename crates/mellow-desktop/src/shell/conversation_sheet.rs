//! The conversation sheet (`ChatPane.svelte`, ADR-0022 §2): the chat header (the place's name, a
//! channel's topic, "Go to…", the pinned messages and the member sheet toggle), then
//! [`DuskShell::conversation_slot`]: the conversation view of the open channel or conversation, or
//! the sentence a place without one opens with.

use gpui::{
    AnyElement, Context, InteractiveElement as _, IntoElement, ParentElement as _, Role,
    SharedString, StatefulInteractiveElement as _, Styled as _, Window, div,
    prelude::FluentBuilder as _,
};

use mellow_client_core::shared_logic::format::display_name;

use super::channel_sidebar::{can_send_in, conversation_display_name};
use super::{DuskShell, ShellLayout};
use crate::components::{IconButton, icon, with_type};
use crate::conversation::OpenConversation;
use crate::i18n::strings;
use crate::navigation::Place;
use crate::theme::icons::IconName;
use crate::theme::tokens::{
    COMPONENT_CHAT_HEADER_BUTTON_SIZE, COMPONENT_CHAT_HEADER_DIVIDER_HEIGHT,
    COMPONENT_CHAT_HEADER_HEIGHT, COMPONENT_CHAT_HEADER_ICON_SIZE,
    COMPONENT_CHAT_HEADER_PADDING_INLINE, COMPONENT_COMPOSER_MARGIN_BOTTOM,
    COMPONENT_COMPOSER_MARGIN_INLINE, COMPONENT_COMPOSER_MIN_HEIGHT, COMPONENT_COMPOSER_RADIUS,
    COMPONENT_SHEET_RADIUS, SPACE_2XS, SPACE_MD, SPACE_XS, TYPE_BODY, TYPE_HEADING_MD,
    TYPE_HEADING_XL,
};
use crate::theme::{color, length, theme};

/// What the sheet shows for the current place.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct SheetSubject {
    /// The header's name: "general", "Ana", "Direct messages".
    pub name: String,
    pub icon: Option<IconName>,
    pub topic: Option<String>,
    /// The sentence the empty conversation opens with, until the message list lands.
    pub welcome: Option<String>,
    /// The member sheet toggle shows for a server channel.
    pub has_member_sheet: bool,
}

impl DuskShell {
    /// The open channel or conversation as the conversation view names it.
    pub(super) fn open_conversation(&self, cx: &gpui::App) -> Option<OpenConversation> {
        let strings = strings(cx);
        let state = self.session.read(cx).state();
        let current_user_id = state.current_user().map(|user| user.id);
        match self.navigation.place()? {
            Place::Server {
                guild_id,
                channel_id: Some(channel_id),
            } => {
                let channel = state
                    .guild(guild_id)?
                    .channels
                    .iter()
                    .find(|channel| channel.id == channel_id)?;
                Some(OpenConversation {
                    channel_id,
                    guild_id: Some(guild_id),
                    label: format!("#{}", channel.name),
                    composer_label: strings.chat().composer_in_channel(&channel.name),
                    hero_icon: IconName::Hash,
                    hero_title: strings.chat().welcome_to_channel(&channel.name),
                    hero_sentence: strings.chat().channel_start(&channel.name),
                    topic: channel.topic.trim().to_owned(),
                })
            }
            Place::Home {
                conversation_id: Some(conversation_id),
            } => {
                let conversation = state.direct_message_channel(conversation_id)?;
                let name = conversation_display_name(conversation, current_user_id, strings);
                let is_group = conversation.owner_id.is_some();
                let participant_names: Vec<&str> = conversation
                    .recipients
                    .iter()
                    .filter(|recipient| Some(recipient.id) != current_user_id)
                    .map(display_name)
                    .collect();
                Some(OpenConversation {
                    channel_id: conversation_id,
                    guild_id: None,
                    label: name.clone(),
                    composer_label: if is_group {
                        strings.chat().composer_to_group(&name)
                    } else {
                        strings.chat().composer_to_person(&name)
                    },
                    hero_icon: if is_group {
                        IconName::Users
                    } else {
                        IconName::At
                    },
                    hero_sentence: if is_group {
                        strings
                            .chat()
                            .group_start(&strings.groups().participant_names(&participant_names))
                    } else {
                        strings.chat().conversation_start(&name)
                    },
                    hero_title: name,
                    topic: String::new(),
                })
            }
            _ => None,
        }
    }

    pub(super) fn sheet_subject(&self, cx: &gpui::App) -> SheetSubject {
        let strings = strings(cx);
        let state = self.session.read(cx).state();
        let current_user_id = state.current_user().map(|user| user.id);
        match self.navigation.place() {
            None => SheetSubject {
                name: String::new(),
                icon: None,
                topic: None,
                welcome: Some(strings.chat().loading_workspace().to_owned()),
                has_member_sheet: false,
            },
            Some(Place::Server {
                guild_id,
                channel_id: None,
            }) => SheetSubject {
                name: state
                    .guild(guild_id)
                    .map(|guild| guild.name.clone())
                    .unwrap_or_default(),
                icon: None,
                topic: None,
                welcome: Some(
                    state
                        .guild(guild_id)
                        .map(|guild| strings.chat().no_visible_channels_body(&guild.name))
                        .unwrap_or_default(),
                ),
                has_member_sheet: false,
            },
            Some(Place::Server {
                guild_id,
                channel_id: Some(channel_id),
            }) => {
                let channel = state.guild(guild_id).and_then(|guild| {
                    guild
                        .channels
                        .iter()
                        .find(|channel| channel.id == channel_id)
                });
                let name = channel
                    .map(|channel| channel.name.clone())
                    .unwrap_or_default();
                SheetSubject {
                    welcome: Some(strings.chat().welcome_to_channel(&name)),
                    topic: channel
                        .map(|channel| channel.topic.trim().to_owned())
                        .filter(|topic| !topic.is_empty()),
                    icon: Some(if can_send_in(state, channel_id) {
                        IconName::Hash
                    } else {
                        IconName::Lock
                    }),
                    name,
                    has_member_sheet: true,
                }
            }
            Some(Place::Home {
                conversation_id: Some(conversation_id),
            }) => {
                let conversation = state.direct_message_channel(conversation_id);
                let name = conversation
                    .map(|conversation| {
                        conversation_display_name(conversation, current_user_id, strings)
                    })
                    .unwrap_or_default();
                let is_group =
                    conversation.is_some_and(|conversation| conversation.owner_id.is_some());
                SheetSubject {
                    welcome: Some(if is_group {
                        strings.chat().group_start(&name)
                    } else {
                        strings.chat().conversation_start(&name)
                    }),
                    icon: Some(if is_group {
                        IconName::Users
                    } else {
                        IconName::At
                    }),
                    topic: None,
                    name,
                    has_member_sheet: false,
                }
            }
            // shortcut: the Friends space (`FriendsPane.svelte`) comes with friends (M3 step 4).
            Some(Place::Home {
                conversation_id: None,
            }) => SheetSubject {
                name: strings.sidebar().friends().to_owned(),
                icon: Some(IconName::Users),
                topic: None,
                welcome: None,
                has_member_sheet: false,
            },
        }
    }

    pub(super) fn render_conversation_sheet(
        &mut self,
        layout: ShellLayout,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let tokens = theme(cx);
        let is_full_bleed = layout == ShellLayout::SheetAlone;
        let subject = self.sheet_subject(cx);
        let header = self.render_chat_header(&subject, is_full_bleed, window, cx);
        let slot = if self.navigation.selected_channel_id().is_some() {
            self.conversation.clone().into_any_element()
        } else {
            Self::conversation_slot(&subject, cx)
        };
        div()
            .debug_selector(|| "sheet".to_owned())
            .id("sheet")
            .role(Role::Main)
            .flex_1()
            .min_w_0()
            .flex()
            .flex_col()
            .bg(color(tokens.color_surface_chat))
            .when(!is_full_bleed, |sheet| {
                sheet.rounded(length(COMPONENT_SHEET_RADIUS))
            })
            .overflow_hidden()
            .child(header)
            .child(slot)
    }

    #[allow(clippy::too_many_lines)] // One element tree, in the order it is drawn.
    fn render_chat_header(
        &mut self,
        subject: &SheetSubject,
        has_navigation_drawer: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let tokens = theme(cx);
        let strings = strings(cx);
        let entity = cx.entity();
        let header_button = |id: &'static str, icon_name: IconName, label: &'static str| {
            IconButton::new(id, icon_name, label).sizes(
                COMPONENT_CHAT_HEADER_BUTTON_SIZE,
                COMPONENT_CHAT_HEADER_ICON_SIZE,
            )
        };
        let is_member_sheet_shown = self.is_member_sheet_shown(window, cx);
        let divider = || {
            div()
                .flex_none()
                .mx(length(SPACE_XS))
                .w(length(1.0))
                .h(length(COMPONENT_CHAT_HEADER_DIVIDER_HEIGHT))
                .bg(color(tokens.color_border_divider))
        };
        // No line under the header: the sheet's tone and space separate it (ADR-0022 §2).
        div()
            .debug_selector(|| "chat-header".to_owned())
            .flex_none()
            .h(length(COMPONENT_CHAT_HEADER_HEIGHT))
            .pl(length(COMPONENT_CHAT_HEADER_PADDING_INLINE))
            .pr(length(SPACE_XS))
            .flex()
            .items_center()
            .gap(length(SPACE_XS))
            .when(has_navigation_drawer, |header| {
                let entity = entity.clone();
                header.child(
                    header_button(
                        "open-navigation",
                        IconName::List,
                        strings.chat().open_navigation(),
                    )
                    .on_click(move |window, cx| {
                        entity.update(cx, |shell, cx| shell.open_navigation_drawer(window, cx));
                    }),
                )
            })
            .child(
                div()
                    .min_w_0()
                    .flex()
                    .items_center()
                    .gap(length(SPACE_2XS))
                    .when(subject.topic.is_some(), |name| {
                        name.max_w(gpui::relative(0.5))
                    })
                    .text_color(color(tokens.component_chat_header_name_text))
                    .when_some(subject.icon, |name, icon_name| {
                        name.child(icon(
                            icon_name,
                            COMPONENT_CHAT_HEADER_ICON_SIZE,
                            color(tokens.color_text_muted),
                        ))
                    })
                    .child(
                        with_type(div(), TYPE_HEADING_MD)
                            .id("chat-header-name")
                            .role(Role::Heading)
                            .aria_level(1)
                            .aria_label(SharedString::from(subject.name.clone()))
                            .min_w_0()
                            .truncate()
                            .child(subject.name.clone()),
                    ),
            )
            .when_some(
                subject.topic.clone().filter(|_| !has_navigation_drawer),
                |header, topic| {
                    header.child(divider()).child(
                        with_type(div(), TYPE_BODY)
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_color(color(tokens.component_chat_header_topic_text))
                            .child(topic),
                    )
                },
            )
            .when(subject.topic.is_none() || has_navigation_drawer, |header| {
                header.child(div().flex_1())
            })
            .child({
                let entity = entity.clone();
                header_button(
                    "go-to",
                    IconName::MagnifyingGlass,
                    strings.chat().go_to_label(),
                )
                .on_click(move |window, cx| {
                    entity.update(cx, |shell, cx| shell.toggle_quick_switcher(window, cx));
                })
            })
            .when(self.navigation.selected_channel_id().is_some(), |header| {
                let conversation = self.conversation.clone();
                let is_open = conversation.read(cx).is_pins_panel_open();
                header.child(
                    header_button(
                        "pinned-messages",
                        IconName::PushPin,
                        strings.pins().pinned_messages(),
                    )
                    .toggled(is_open)
                    .on_click(move |window, cx| {
                        conversation.update(cx, |conversation, cx| {
                            conversation.toggle_pins_panel(window, cx);
                        });
                    }),
                )
            })
            .when(subject.has_member_sheet, |header| {
                header.child(
                    header_button(
                        "member-list-toggle",
                        IconName::Users,
                        strings.chat().member_list(),
                    )
                    .toggled(is_member_sheet_shown)
                    .on_click(move |window, cx| {
                        entity.update(cx, |shell, cx| shell.toggle_member_sheet(window, cx));
                    }),
                )
            })
    }

    /// A place without a conversation (loading, a server without channels, the Friends space):
    /// the sentence it opens with, and the composer's frame, so the dock lines up with the user
    /// card (ADR-0022 §4).
    fn conversation_slot(subject: &SheetSubject, cx: &gpui::App) -> AnyElement {
        let tokens = theme(cx);
        let composer_frame = div()
            .debug_selector(|| "composer".to_owned())
            .flex_none()
            .h(length(COMPONENT_COMPOSER_MIN_HEIGHT))
            .mx(length(COMPONENT_COMPOSER_MARGIN_INLINE))
            .mb(length(COMPONENT_COMPOSER_MARGIN_BOTTOM))
            .rounded(length(COMPONENT_COMPOSER_RADIUS))
            .bg(color(tokens.component_composer_background));
        div()
            .debug_selector(|| "conversation-slot".to_owned())
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .flex_col()
                    .justify_end()
                    .px(length(SPACE_MD))
                    .pb(length(SPACE_MD))
                    .when_some(subject.welcome.clone(), |body, welcome| {
                        body.child(
                            with_type(div(), TYPE_HEADING_XL)
                                .text_color(color(tokens.color_text_heading))
                                .child(welcome),
                        )
                    }),
            )
            .child(composer_frame)
            .into_any_element()
    }
}
