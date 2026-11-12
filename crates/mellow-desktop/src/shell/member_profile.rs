//! A member's profile in the side sheet (`MemberList.svelte`, `ProfileCard.svelte`,
//! `ProfileDetails.svelte`, DESIGN.md §5.7, ADR-0050): choosing a member shows the banner, or the
//! strip in their avatar tint, the avatar cut out of the sheet, their name, "About me" and links
//! (read when it opens, as `user-profile-loader.svelte.ts`), "Send message", then Add friend (or
//! Accept, or "Request sent") and Block; someone blocked shows Unblock alone (ADR-0025). Back
//! returns to the list, on the row it came from.

use std::rc::Rc;

use gpui::{
    AnyElement, App, Context, ElementId, FocusHandle, InteractiveElement as _, IntoElement,
    ObjectFit, ParentElement as _, Pixels, Point, Role, ScrollHandle, SharedString,
    StatefulInteractiveElement as _, Styled as _, StyledImage as _, Window, div, img,
    prelude::FluentBuilder as _,
};
use mellow_client_core::ClientError;
use mellow_client_core::DownloadableFile;
use mellow_client_core::shared_logic::format::display_name;
use mellow_client_core::shared_logic::markdown::parse_inline_markdown;
use mellow_client_core::shared_logic::profiles::{
    ShownProfileLink, UserProfileLoad, UserProfileLoader, shown_profile_links,
};
use mellow_protocol::{
    PublicUser, RelationshipKind, SendFriendRequest, SetRelationshipKind, Snowflake,
};

use super::DuskShell;
use crate::components::{
    Avatar, AvatarPerson, Button, ButtonVariant, LiveRegion as _, Tooltip, avatar_tint, icon,
    inset_focus_ring, is_focus_visible, remembered_focus_handle, remembered_hover, with_type,
};
use crate::i18n::strings;
use crate::messages::{MessageBody, MessageMarkdown, picture_animation, picture_for_box};
use crate::pictures::Animation;
use crate::sign_in_form::describe_error;
use crate::theme::icons::IconName;
use crate::theme::tokens::{
    COMPONENT_PROFILE_POPOVER_AVATAR_RING_WIDTH, COMPONENT_PROFILE_POPOVER_AVATAR_SIZE,
    COMPONENT_PROFILE_POPOVER_BANNER_HEIGHT, COMPONENT_PROFILE_POPOVER_BANNER_IMAGE_HEIGHT,
    COMPONENT_PROFILE_POPOVER_LINK_GAP, COMPONENT_PROFILE_POPOVER_LINK_ICON_PLATE_SIZE,
    COMPONENT_PROFILE_POPOVER_LINK_ICON_RADIUS, COMPONENT_PROFILE_POPOVER_LINK_ICON_SIZE,
    COMPONENT_PROFILE_POPOVER_LINK_ROW_HEIGHT, COMPONENT_PROFILE_POPOVER_PADDING,
    COMPONENT_PROFILE_POPOVER_SECTION_GAP, COMPONENT_PROFILE_POPOVER_TEXT_GAP, RADIUS_XS,
    SPACE_2XS, SPACE_XS, TYPE_BODY, TYPE_BODY_SM, TYPE_CATEGORY, TYPE_HEADING_LG,
};
use crate::theme::{color, length, theme};

/// Block, and Unblock in its place once blocked.
const PROFILE_BLOCK_ID: &str = "profile-block";

/// A friend or block change made from the profile without a dialog.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum RelationshipChange {
    AddFriend,
    Accept,
    Unblock,
}

/// Whose profile the sheet shows, and in which server's list it was chosen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ShownProfile {
    guild_id: Snowflake,
    user_id: Snowflake,
}

/// The side sheet's profile: what is shown, its details as they load, and its one action.
pub(super) struct MemberProfileState {
    shown: Option<ShownProfile>,
    loader: UserProfileLoader,
    /// "About me" parsed once per text.
    parsed_about_me: Option<(String, Rc<MessageBody>)>,
    is_opening_conversation: bool,
    profile_action_error: Option<String>,
    is_changing_relationship: bool,
    /// Read out by the profile's status region: what the last friend or block change did.
    relationship_announcement: String,
    back_focus: FocusHandle,
    /// The sheet's scroller, shared by the list and the profile.
    list_scroll: ScrollHandle,
    /// Where the list was scrolled when the profile opened, restored by Back: focus alone does
    /// not scroll the row into view.
    list_offset: Point<Pixels>,
    /// The row Back returns to, focused once the list is drawn again.
    row_to_focus: Option<Snowflake>,
}

impl MemberProfileState {
    pub(super) fn new(cx: &mut App) -> Self {
        Self {
            shown: None,
            loader: UserProfileLoader::default(),
            parsed_about_me: None,
            is_opening_conversation: false,
            profile_action_error: None,
            is_changing_relationship: false,
            relationship_announcement: String::new(),
            back_focus: cx.focus_handle().tab_stop(true),
            list_scroll: ScrollHandle::new(),
            list_offset: Point::default(),
            row_to_focus: None,
        }
    }

    #[cfg(test)]
    pub(super) fn load(&self) -> &UserProfileLoad {
        self.loader.current()
    }

    /// Why the last action failed, as the card says it.
    #[cfg(test)]
    pub(super) fn profile_action_error(&self) -> Option<&str> {
        self.profile_action_error.as_deref()
    }

    #[cfg(test)]
    pub(super) fn is_changing_relationship(&self) -> bool {
        self.is_changing_relationship
    }

    pub(super) fn back_focus(&self) -> &FocusHandle {
        &self.back_focus
    }

    pub(super) fn list_scroll(&self) -> &ScrollHandle {
        &self.list_scroll
    }

    /// The person shown in `guild_id`'s sheet; another server's sheet shows its list.
    pub(super) fn shown_user_id(&self, guild_id: Snowflake) -> Option<Snowflake> {
        self.shown
            .filter(|shown| shown.guild_id == guild_id)
            .map(|shown| shown.user_id)
    }

    /// True once for the row Back returns to.
    pub(super) fn take_row_to_focus(&mut self, user_id: Snowflake) -> bool {
        if self.row_to_focus == Some(user_id) {
            self.row_to_focus = None;
            true
        } else {
            false
        }
    }

    fn forget(&mut self) {
        self.shown = None;
        self.loader.show(None);
        self.profile_action_error = None;
    }

    fn about_body(&mut self, about_me: &str) -> Option<Rc<MessageBody>> {
        if self
            .parsed_about_me
            .as_ref()
            .is_none_or(|(text, _)| text != about_me)
        {
            let body = MessageBody {
                blocks: parse_inline_markdown(about_me),
                is_jumbo: false,
                names_reader: false,
            };
            self.parsed_about_me = Some((about_me.to_owned(), Rc::new(body)));
        }
        self.parsed_about_me
            .as_ref()
            .filter(|(_, body)| !body.blocks.is_empty())
            .map(|(_, body)| Rc::clone(body))
    }
}

impl DuskShell {
    /// A member row was chosen: their profile replaces the list, focus on Back.
    pub(super) fn show_member_profile(
        &mut self,
        guild_id: Snowflake,
        user_id: Snowflake,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let profile = &mut self.member_profile;
        if profile.shown.is_none() {
            profile.list_offset = profile.list_scroll.offset();
        }
        profile.list_scroll.set_offset(Point::default());
        profile.shown = Some(ShownProfile { guild_id, user_id });
        profile.profile_action_error = None;
        profile.is_opening_conversation = false;
        if let Some(ticket) = profile.loader.show(Some(user_id)) {
            let shell = cx.entity().downgrade();
            self.session.update(cx, |session, cx| {
                session.load_user_profile(user_id, cx, move |answer, cx| {
                    let _ = shell.update(cx, |shell, cx| {
                        if shell.member_profile.loader.finish(ticket, answer) {
                            cx.notify();
                        }
                    });
                });
            });
        }
        window.focus(&self.member_profile.back_focus, cx);
        cx.notify();
    }

    /// Back: the list again, focus on the row the profile came from.
    pub(super) fn show_member_list(&mut self, cx: &mut Context<Self>) {
        let profile = &mut self.member_profile;
        profile.row_to_focus = profile.shown.map(|shown| shown.user_id);
        profile.list_scroll.set_offset(profile.list_offset);
        profile.forget();
        cx.notify();
    }

    /// "Send message": opens the conversation with them, as the quick switcher does.
    fn send_message_from_profile(
        &mut self,
        user_id: Snowflake,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.member_profile.is_opening_conversation {
            return;
        }
        self.member_profile.is_opening_conversation = true;
        self.member_profile.profile_action_error = None;
        let failure = strings(cx).members().open_conversation_failed().to_owned();
        let shell = cx.entity().downgrade();
        let window_handle = window.window_handle();
        self.session.update(cx, |session, cx| {
            session.open_direct_message(user_id, failure, cx, move |answer, cx| {
                let _ = window_handle.update(cx, |_, window, cx| {
                    let _ = shell.update(cx, |shell, cx| {
                        shell.member_profile.is_opening_conversation = false;
                        match answer {
                            Ok(conversation_id) => {
                                shell.member_profile.forget();
                                shell.close_member_overlay(window, cx);
                                shell.open_place(None, Some(conversation_id), window, cx);
                                window.focus(&shell.root_focus, cx);
                                shell.conversation.update(cx, |conversation, cx| {
                                    conversation.focus_composer(window, cx);
                                });
                            }
                            Err(message) => {
                                shell.member_profile.profile_action_error = Some(message);
                            }
                        }
                        cx.notify();
                    });
                });
            });
        });
        cx.notify();
    }

    /// Add friend, Accept or Unblock (`changeRelationship`): the button pressed is replaced once
    /// the instance answers, so focus moves to Block if it was still on it.
    pub(super) fn change_relationship_from_profile(
        &mut self,
        user: &PublicUser,
        change: RelationshipChange,
        block_focus: FocusHandle,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.member_profile.is_changing_relationship {
            return;
        }
        self.member_profile.is_changing_relationship = true;
        self.member_profile.profile_action_error = None;
        let catalogue = strings(cx);
        let person = display_name(user);
        let (done, failure) = match change {
            RelationshipChange::AddFriend => (
                catalogue.friends().sent(person),
                catalogue.members().friend_request_failed().to_owned(),
            ),
            RelationshipChange::Accept => (
                catalogue.friends().accepted(person),
                catalogue.members().accept_failed().to_owned(),
            ),
            RelationshipChange::Unblock => (
                catalogue.friends().unblocked(person),
                catalogue.friends().unblock_failed(person),
            ),
        };
        let pressed = window.focused(cx);
        let shell = cx.entity().downgrade();
        let window_handle = window.window_handle();
        let on_done = move |answer: Result<(), ClientError>, cx: &mut App| {
            let _ = window_handle.update(cx, |_, window, cx| {
                let has_focus_stayed = window.focused(cx) == pressed;
                let _ = shell.update(cx, |shell, cx| {
                    let profile = &mut shell.member_profile;
                    profile.is_changing_relationship = false;
                    match answer {
                        Ok(()) => {
                            profile.relationship_announcement = done;
                            if has_focus_stayed {
                                window.focus(&block_focus, cx);
                            }
                        }
                        Err(error) => {
                            profile.profile_action_error = Some(describe_error(
                                &error,
                                &failure,
                                crate::strings_pending_i18n::strings(cx),
                            ));
                        }
                    }
                    cx.notify();
                });
            });
        };
        let user_id = user.id;
        let username = user.username.clone();
        self.session.update(cx, |session, cx| match change {
            RelationshipChange::AddFriend => session.send_friend_request(
                SendFriendRequest {
                    username,
                    friend_code: None,
                },
                cx,
                move |answer, cx| on_done(answer.map(|_| ()), cx),
            ),
            RelationshipChange::Accept => {
                session.set_relationship(user_id, SetRelationshipKind::Friend, cx, on_done);
            }
            RelationshipChange::Unblock => session.remove_relationship(user_id, cx, on_done),
        });
        cx.notify();
    }

    /// The friend and block buttons under "Send message", stacked: the sheet is too narrow for two
    /// labels side by side in French. Block and Unblock share a focus handle, so focus stays put
    /// when one replaces the other.
    fn relationship_buttons(
        &self,
        user: &PublicUser,
        relationship: Option<RelationshipKind>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        let catalogue = strings(cx);
        let is_busy = self.member_profile.is_changing_relationship;
        let block_focus = remembered_focus_handle(&ElementId::from(PROFILE_BLOCK_ID), window, cx);
        let entity = cx.entity();
        let change_button = |id: &'static str, label: &'static str, change: RelationshipChange| {
            let entity = entity.clone();
            let user = user.clone();
            let block_focus = block_focus.clone();
            Button::new(id, label)
                .full_width()
                .disabled(is_busy)
                .on_click(move |window, cx| {
                    entity.update(cx, |shell, cx| {
                        shell.change_relationship_from_profile(
                            &user,
                            change,
                            block_focus.clone(),
                            window,
                            cx,
                        );
                    });
                })
        };
        if relationship == Some(RelationshipKind::Blocked) {
            return vec![
                change_button(
                    "profile-unblock",
                    catalogue.friends().unblock(),
                    RelationshipChange::Unblock,
                )
                .track_focus(&block_focus)
                .into_any_element(),
            ];
        }
        let request = match relationship {
            None => Some(
                change_button(
                    "profile-add-friend",
                    catalogue.members().add_friend(),
                    RelationshipChange::AddFriend,
                )
                .into_any_element(),
            ),
            Some(RelationshipKind::IncomingRequest) => Some(
                change_button(
                    "profile-accept",
                    catalogue.members().accept_request(),
                    RelationshipChange::Accept,
                )
                .into_any_element(),
            ),
            Some(RelationshipKind::OutgoingRequest) => Some(
                Button::new("profile-request-sent", catalogue.members().request_sent())
                    .full_width()
                    .disabled(true)
                    .into_any_element(),
            ),
            Some(RelationshipKind::Friend | RelationshipKind::Blocked) => None,
        };
        let home = self.home.clone();
        let user = user.clone();
        let block = Button::new(PROFILE_BLOCK_ID, catalogue.friends().block())
            .full_width()
            .track_focus(&block_focus)
            .on_click(move |window, cx| {
                home.update(cx, |home, cx| home.ask_to_block(&user, window, cx));
            })
            .into_any_element();
        request.into_iter().chain([block]).collect()
    }

    /// The card under the sheet's header; `sheet_width` sizes the banner picture.
    #[allow(clippy::too_many_lines)] // One element tree, in the order it is drawn.
    pub(super) fn render_member_profile(
        &mut self,
        user: &PublicUser,
        sheet_width: f32,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let tokens = theme(cx);
        let catalogue = strings(cx);
        let surface = color(tokens.component_side_sheet_background);
        let animation = picture_animation(cx);
        let person = AvatarPerson::from_user(
            user,
            user.avatar.map(|image| {
                picture_for_box(
                    DownloadableFile::ProfileImage(image.id),
                    COMPONENT_PROFILE_POPOVER_AVATAR_SIZE,
                    COMPONENT_PROFILE_POPOVER_AVATAR_SIZE,
                    animation,
                    window,
                )
            }),
        );
        let tint = avatar_tint(tokens, person.tint_number);
        let banner = match user.banner {
            // A picture that fails gives way to the avatar tint, as on the web.
            Some(banner) => div()
                .flex_none()
                .w_full()
                .h(length(COMPONENT_PROFILE_POPOVER_BANNER_IMAGE_HEIGHT))
                .bg(color(tokens.component_profile_popover_banner))
                .child(
                    img(picture_for_box(
                        DownloadableFile::ProfileImage(banner.id),
                        sheet_width,
                        COMPONENT_PROFILE_POPOVER_BANNER_IMAGE_HEIGHT,
                        animation,
                        window,
                    ))
                    .size_full()
                    .object_fit(ObjectFit::Cover)
                    .with_fallback(move || div().size_full().bg(tint).into_any_element()),
                ),
            // No banner picture: the strip takes the person's avatar tint, as in the Dusk mockup.
            None => div()
                .flex_none()
                .w_full()
                .h(length(COMPONENT_PROFILE_POPOVER_BANNER_HEIGHT))
                .bg(tint),
        };
        let name = display_name(user).to_owned();
        let state = self.session.read(cx).state();
        let is_self = state.current_user().is_some_and(|me| me.id == user.id);
        let relationship = state
            .relationship(user.id)
            .map(|relationship| relationship.kind);
        let is_blocked = relationship == Some(RelationshipKind::Blocked);

        let details: Vec<AnyElement> = match self.member_profile.loader.current().clone() {
            UserProfileLoad::Loaded { user_id, profile } if user_id == user.id => {
                let mut sections = Vec::new();
                if let Some(body) = self.member_profile.about_body(&profile.about_me) {
                    sections.push(
                        section("profile-about", catalogue.profile_details().about_me(), cx)
                            .child(
                                with_type(div(), TYPE_BODY)
                                    .text_color(color(tokens.component_profile_popover_about_text))
                                    .child(MessageMarkdown::new(
                                        ("profile-about-text", user.id.raw()),
                                        body,
                                    )),
                            )
                            .into_any_element(),
                    );
                }
                let links = shown_profile_links(&profile.links);
                if !links.is_empty() {
                    let rows: Vec<AnyElement> = links
                        .into_iter()
                        .enumerate()
                        .map(|(index, link)| link_row(index, link, window, cx))
                        .collect();
                    sections.push(
                        section("profile-links", catalogue.profile_details().links(), cx)
                            .child(
                                div()
                                    .id("profile-link-list")
                                    .role(Role::List)
                                    .children(rows),
                            )
                            .into_any_element(),
                    );
                }
                sections
            }
            UserProfileLoad::Failed { user_id } if user_id == user.id => {
                vec![
                    with_type(div(), TYPE_BODY_SM)
                        .mt(length(COMPONENT_PROFILE_POPOVER_SECTION_GAP))
                        .text_color(color(tokens.component_profile_popover_username_text))
                        .child(catalogue.profile_details().load_failed())
                        .into_any_element(),
                ]
            }
            _ => Vec::new(),
        };

        let is_opening = self.member_profile.is_opening_conversation;
        let user_id = user.id;
        let entity = cx.entity();
        let relationship_buttons = if is_self {
            Vec::new()
        } else {
            self.relationship_buttons(user, relationship, window, cx)
        };
        // No conversation while blocked: unblocking comes first.
        let action = (!is_self && !is_blocked).then(|| {
            div().mt(length(SPACE_XS)).child(
                Button::new(
                    "profile-send-message",
                    if is_opening {
                        catalogue.members().opening()
                    } else {
                        catalogue.members().send_message()
                    },
                )
                .variant(ButtonVariant::Primary)
                .full_width()
                .disabled(is_opening)
                .on_click(move |window, cx| {
                    entity.update(cx, |shell, cx| {
                        shell.send_message_from_profile(user_id, window, cx);
                    });
                }),
            )
        });
        let error = self
            .member_profile
            .profile_action_error
            .clone()
            .map(|error| {
                with_type(div(), TYPE_BODY)
                    .id("profile-error")
                    .live_assertive(error.clone())
                    .text_color(color(tokens.component_profile_popover_error_text))
                    .child(error)
            });

        // The ring is drawn outside the avatar, so the avatar keeps the card's padding and sits
        // one gap below the banner's middle line.
        let ring = COMPONENT_PROFILE_POPOVER_AVATAR_RING_WIDTH;
        let avatar = div()
            .flex_none()
            .size(length(COMPONENT_PROFILE_POPOVER_AVATAR_SIZE + 2.0 * ring))
            .mt(length(
                COMPONENT_PROFILE_POPOVER_TEXT_GAP
                    - COMPONENT_PROFILE_POPOVER_AVATAR_SIZE / 2.0
                    - ring,
            ))
            .ml(-length(ring))
            .p(length(ring))
            .rounded_full()
            .bg(surface)
            .child(Avatar::new(person, COMPONENT_PROFILE_POPOVER_AVATAR_SIZE));

        div()
            .debug_selector(|| "member-profile".to_owned())
            .flex()
            .flex_col()
            .child(banner)
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(length(COMPONENT_PROFILE_POPOVER_TEXT_GAP))
                    .px(length(COMPONENT_PROFILE_POPOVER_PADDING))
                    .pb(length(COMPONENT_PROFILE_POPOVER_PADDING))
                    .child(avatar)
                    .child(
                        with_type(div(), TYPE_HEADING_LG)
                            .id("profile-name")
                            .role(Role::Heading)
                            .aria_level(3)
                            .aria_label(name.clone())
                            .mt(length(SPACE_XS))
                            .text_color(color(tokens.component_profile_popover_name_text))
                            .child(name),
                    )
                    .children(details)
                    .children(action)
                    .when(!relationship_buttons.is_empty(), |card| {
                        card.child(
                            div()
                                .when(is_blocked, |buttons| buttons.mt(length(SPACE_XS)))
                                .flex()
                                .flex_col()
                                .gap(length(SPACE_XS))
                                .children(relationship_buttons),
                        )
                    })
                    .child(
                        div()
                            .id("profile-relationship-announcement")
                            .live_polite(SharedString::from(
                                self.member_profile.relationship_announcement.clone(),
                            ))
                            .h_0()
                            .overflow_hidden(),
                    )
                    .children(error),
            )
            .into_any_element()
    }
}

/// A titled part of the details ("About me", "Links").
fn section(id: &'static str, title: &'static str, cx: &App) -> gpui::Stateful<gpui::Div> {
    let tokens = theme(cx);
    div()
        .id(id)
        .mt(length(COMPONENT_PROFILE_POPOVER_SECTION_GAP))
        .flex()
        .flex_col()
        .gap(length(SPACE_2XS))
        .child(
            with_type(div(), TYPE_CATEGORY)
                .id(ElementId::NamedChild(
                    std::sync::Arc::new(id.into()),
                    "heading".into(),
                ))
                .role(Role::Heading)
                .aria_level(4)
                .aria_label(title)
                .text_color(color(tokens.component_profile_popover_section_label_text))
                .child(title),
        )
}

/// A link with its site's icon, stored by the instance, or a link glyph until it is fetched;
/// its label, or its host, names it. It opens in the browser.
fn link_row(index: usize, link: ShownProfileLink, window: &mut Window, cx: &mut App) -> AnyElement {
    let tokens = theme(cx);
    let catalogue = strings(cx);
    let id = ElementId::NamedInteger("profile-link".into(), index as u64);
    let focus_handle = remembered_focus_handle(&id, window, cx);
    let hover = remembered_hover(&id, window, cx);
    let is_hovered = *hover.read(cx);
    let is_ring_shown = is_focus_visible(&focus_handle, window);
    let name: SharedString = match &link.label {
        Some(label) => catalogue
            .profile_details()
            .link_label(label, &link.host)
            .into(),
        None => link.host.clone().into(),
    };
    let accessible_name = format!(
        "{name} ({})",
        catalogue.profile_details().opens_in_new_tab()
    );
    let fallback_glyph_colour = color(tokens.component_profile_popover_link_icon_fallback);
    let plate = div()
        .flex_none()
        .size(length(COMPONENT_PROFILE_POPOVER_LINK_ICON_PLATE_SIZE))
        .flex()
        .items_center()
        .justify_center()
        .rounded(length(COMPONENT_PROFILE_POPOVER_LINK_ICON_RADIUS));
    let plate = match link.icon_id {
        // On a light plate: many sites draw dark marks on transparency.
        Some(icon_id) => plate
            .bg(color(tokens.component_profile_popover_link_icon_plate))
            .child(
                img(picture_for_box(
                    DownloadableFile::LinkIcon(icon_id),
                    COMPONENT_PROFILE_POPOVER_LINK_ICON_SIZE,
                    COMPONENT_PROFILE_POPOVER_LINK_ICON_SIZE,
                    Animation::StillFrame,
                    window,
                ))
                .size(length(COMPONENT_PROFILE_POPOVER_LINK_ICON_SIZE))
                .object_fit(ObjectFit::Contain)
                .with_fallback(move || link_glyph(fallback_glyph_colour).into_any_element()),
            ),
        None => plate.child(link_glyph(fallback_glyph_colour)),
    };
    let url = link.url.clone();
    let tooltip: SharedString = link.url.into();
    let item_id = ElementId::NamedInteger("profile-link-item".into(), index as u64);
    let row = div()
        .id(id)
        .role(Role::Link)
        .aria_label(accessible_name)
        .track_focus(&focus_handle)
        .relative()
        .min_h(length(COMPONENT_PROFILE_POPOVER_LINK_ROW_HEIGHT))
        .flex()
        .items_center()
        .gap(length(COMPONENT_PROFILE_POPOVER_LINK_GAP))
        .rounded(length(RADIUS_XS))
        .cursor_pointer()
        .text_color(color(tokens.component_profile_popover_link_text))
        .tooltip(move |_, cx| Tooltip::view(tooltip.clone(), cx))
        .tooltip_show_delay(Tooltip::show_delay())
        .on_hover(move |is_hovered, _, cx| {
            hover.update(cx, |remembered, cx| {
                *remembered = *is_hovered;
                cx.notify();
            });
        })
        .on_click(move |_, _, cx| cx.open_url(&url))
        .child(plate)
        .child(
            with_type(div(), TYPE_BODY)
                .min_w_0()
                .truncate()
                .when(is_hovered, gpui::Styled::underline)
                .child(name),
        )
        .when(is_ring_shown, |row| {
            row.child(inset_focus_ring(RADIUS_XS, cx))
        });
    div()
        .id(item_id)
        .role(Role::ListItem)
        .child(row)
        .into_any_element()
}

/// The link glyph: a link without a stored icon, or whose icon failed to load (the web's
/// `onerror`).
fn link_glyph(colour: gpui::Hsla) -> impl IntoElement {
    icon(
        IconName::LinkSimple,
        COMPONENT_PROFILE_POPOVER_LINK_ICON_SIZE,
        colour,
    )
}
