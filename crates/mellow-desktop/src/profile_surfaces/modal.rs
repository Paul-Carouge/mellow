//! The profile modal (`ProfileModal.svelte`, `ProfileCard.svelte`, `ProfileDetails.svelte`,
//! DESIGN.md §5.7.1): the person's own card on the left, as they framed it in Profile settings,
//! beside what you share with them (About me, links, roles, member since); a sheet rising from the
//! bottom in a narrow window. Over a dimmed window: GPUI cannot blur what lies behind a layer, and
//! the plain scrim is the design there, not a degraded one. Tab stays inside; Esc, Close or the
//! scrim close it, and focus goes back to the name or row that opened it.

use gpui::{
    Animation, AnimationExt as _, AnyElement, App, AppContext as _, Bounds, Context, ElementId,
    Entity, FocusHandle, InteractiveElement as _, IntoElement, KeyDownEvent, MouseButton,
    ObjectFit, ParentElement as _, Pixels, Role, SharedString, StatefulInteractiveElement as _,
    Styled as _, StyledImage as _, Window, anchored, deferred, div, img,
    prelude::FluentBuilder as _,
};
use mellow_client_core::shared_logic::format::display_name;
use mellow_client_core::shared_logic::profiles::{
    ShownProfileLink, UserProfileLoad, shown_profile_links,
};
use mellow_client_core::{ClientError, DownloadableFile};
use mellow_protocol::{PublicUser, RelationshipKind, SendFriendRequest, SetRelationshipKind};

use super::facts::{ProfileFacts, profile_facts};
use super::{ProfileSurfaces, ProfileSurfacesEvent};
use crate::calls::CallStartFailure;
use crate::components::{
    Avatar, AvatarPerson, Button, ButtonVariant, FocusNextControl, FocusPreviousControl,
    IconButton, LiveRegion as _, MenuAlignment, Tooltip, avatar_tint, bounds_recorder, easing,
    focus_next_inside, focus_previous_inside, icon, inset_focus_ring, is_focus_visible,
    menu_origin_beside, remembered_focus_handle, remembered_hover, role_icon, shadow, with_type,
};
use crate::home::{PersonMenuRequest, shown_presence};
use crate::i18n::strings;
use crate::messages::{
    MessageMarkdown, day_divider_label, local_day_of, picture_animation, picture_for_box,
};
use crate::pictures::Animation as PictureAnimation;
use crate::sign_in_form::describe_error;
use crate::theme::icons::IconName;
use crate::theme::tokens::{
    COMPONENT_CHAT_HEADER_ICON_SIZE, COMPONENT_DIALOG_ENTER_DURATION,
    COMPONENT_PROFILE_MODAL_ACTION_GAP, COMPONENT_PROFILE_MODAL_BORDER_WIDTH,
    COMPONENT_PROFILE_MODAL_CLOSE_INSET, COMPONENT_PROFILE_MODAL_CLOSE_SIZE,
    COMPONENT_PROFILE_MODAL_DATES_GAP, COMPONENT_PROFILE_MODAL_DETAILS_PADDING,
    COMPONENT_PROFILE_MODAL_IDENTITY_PADDING, COMPONENT_PROFILE_MODAL_IDENTITY_WIDTH,
    COMPONENT_PROFILE_MODAL_MARGIN, COMPONENT_PROFILE_MODAL_MIN_HEIGHT,
    COMPONENT_PROFILE_MODAL_RADIUS, COMPONENT_PROFILE_MODAL_SECTION_GAP,
    COMPONENT_PROFILE_MODAL_SECTION_LABEL_GAP, COMPONENT_PROFILE_MODAL_SHEET_RADIUS,
    COMPONENT_PROFILE_MODAL_SHEET_TOP_GAP, COMPONENT_PROFILE_MODAL_STATUS_PADDING_BLOCK,
    COMPONENT_PROFILE_MODAL_STATUS_PADDING_INLINE, COMPONENT_PROFILE_MODAL_STATUS_RADIUS,
    COMPONENT_PROFILE_MODAL_WIDTH, COMPONENT_PROFILE_POPOVER_AVATAR_RING_WIDTH,
    COMPONENT_PROFILE_POPOVER_AVATAR_SIZE, COMPONENT_PROFILE_POPOVER_BANNER_HEIGHT,
    COMPONENT_PROFILE_POPOVER_BANNER_IMAGE_HEIGHT, COMPONENT_PROFILE_POPOVER_LINK_GAP,
    COMPONENT_PROFILE_POPOVER_LINK_ICON_PLATE_SIZE, COMPONENT_PROFILE_POPOVER_LINK_ICON_RADIUS,
    COMPONENT_PROFILE_POPOVER_LINK_ICON_SIZE, COMPONENT_PROFILE_POPOVER_LINK_ROW_HEIGHT,
    COMPONENT_PROFILE_POPOVER_TEXT_GAP, COMPONENT_ROLE_ICON_GAP, COMPONENT_ROLE_ICON_MESSAGE_SIZE,
    COMPONENT_STATUS_SIZE_LARGE, COMPONENT_USER_PANEL_BUTTON_ICON_SIZE, DARK_THEME,
    MOTION_DURATION_BASE, MOTION_DURATION_SLOW, MOTION_EASING_ENTER, MOTION_EASING_SETTLE,
    MOTION_EASING_STANDARD, RADIUS_SM, RADIUS_XS, SIZE_HIT_WEB_MIN, SPACE_XS, TYPE_BODY,
    TYPE_BODY_SM, TYPE_CATEGORY, TYPE_HEADING_LG,
};
use crate::theme::{animation_duration, color, length, theme, window_width_at_default_zoom};

/// The layer of the modal: above the shell's drawers and overlays, below the menus and dialogs it
/// opens (its More menu, Block's confirmation), which come later at the same or a higher priority.
const MODAL_LAYER_PRIORITY: usize = 1;

/// A friend or block change made from the modal's buttons.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RelationshipChange {
    AddFriend,
    Accept,
    Unblock,
}

/// One open profile: whose, what opened it, its details as they load, and its actions' state.
pub(super) struct ProfileModal {
    pub user: PublicUser,
    pub opener: Option<FocusHandle>,
    is_opening_conversation: bool,
    is_changing_relationship: bool,
    error: Option<String>,
    /// Read out politely: what the last friend or block change did.
    announcement: String,
    container_focus: FocusHandle,
    /// "Send message", "Unblock" or "Edit profile": one handle, so focus stays put when one
    /// replaces the other.
    pub primary_focus: FocusHandle,
    friend_focus: FocusHandle,
    more_focus: FocusHandle,
    more_bounds: Option<Entity<Option<Bounds<Pixels>>>>,
}

impl ProfileModal {
    #[cfg(test)]
    pub(super) fn container_focus_for_tests(&self) -> &FocusHandle {
        &self.container_focus
    }

    pub(super) fn new(user: PublicUser, opener: Option<FocusHandle>, cx: &mut App) -> Self {
        Self {
            user,
            opener,
            is_opening_conversation: false,
            is_changing_relationship: false,
            error: None,
            announcement: String::new(),
            container_focus: cx.focus_handle(),
            primary_focus: cx.focus_handle().tab_stop(true),
            friend_focus: cx.focus_handle().tab_stop(true),
            more_focus: cx.focus_handle().tab_stop(true),
            more_bounds: None,
        }
    }
}

impl ProfileSurfaces {
    /// Why the last action failed, as the modal says it.
    #[cfg(test)]
    pub(crate) fn modal_error(&self) -> Option<&str> {
        self.modal.as_ref()?.error.as_deref()
    }

    #[cfg(test)]
    pub(crate) fn modal_details_for_tests(&self) -> &UserProfileLoad {
        self.modal_details.current()
    }

    #[cfg(test)]
    pub(crate) fn modal_primary_focus(&self) -> Option<FocusHandle> {
        Some(self.modal.as_ref()?.primary_focus.clone())
    }

    /// "Send message": opens the conversation, then the modal closes and the shell focuses its
    /// composer.
    fn send_message(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(modal) = self.modal.as_mut() else {
            return;
        };
        if modal.is_opening_conversation {
            return;
        }
        modal.is_opening_conversation = true;
        modal.error = None;
        let user_id = modal.user.id;
        let failure = strings(cx).members().open_conversation_failed().to_owned();
        let surfaces = cx.entity().downgrade();
        let window_handle = window.window_handle();
        self.session.update(cx, |session, cx| {
            session.open_direct_message(user_id, failure, cx, move |answer, cx| {
                // The session is still being updated: showing the conversation reads it.
                cx.defer(move |cx| {
                    let _ = window_handle.update(cx, |_, _, cx| {
                        let _ = surfaces.update(cx, |surfaces, cx| {
                            let Some(modal) = surfaces
                                .modal
                                .as_mut()
                                .filter(|modal| modal.user.id == user_id)
                            else {
                                return;
                            };
                            modal.is_opening_conversation = false;
                            match answer {
                                Ok(conversation_id) => {
                                    // The composer takes focus, not the opener.
                                    surfaces.modal = None;
                                    surfaces.modal_details.show(None);
                                    cx.emit(ProfileSurfacesEvent::OpenedConversation(
                                        conversation_id,
                                    ));
                                }
                                Err(message) => modal.error = Some(message),
                            }
                            cx.notify();
                        });
                    });
                });
            });
        });
        cx.notify();
    }

    /// "Appeler" (ADR-0062): the one-to-one conversation opens and rings. A refusal stays here,
    /// where the reader asked.
    fn call(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(modal) = self.modal.as_mut() else {
            return;
        };
        if modal.is_opening_conversation {
            return;
        }
        modal.is_opening_conversation = true;
        modal.error = None;
        let user_id = modal.user.id;
        let person = display_name(&modal.user).to_owned();
        let failure = strings(cx).calls().could_not_call(&person);
        let surfaces = cx.entity().downgrade();
        let window_handle = window.window_handle();
        self.session.update(cx, |session, cx| {
            session.open_direct_message(user_id, failure.clone(), cx, move |answer, cx| {
                let conversation_id = match answer {
                    Ok(conversation_id) => conversation_id,
                    Err(message) => {
                        let _ = surfaces.update(cx, |surfaces, cx| {
                            if let Some(modal) = surfaces.modal.as_mut() {
                                modal.is_opening_conversation = false;
                                modal.error = Some(message);
                            }
                            cx.notify();
                        });
                        return;
                    }
                };
                let Some(calls) = crate::calls::calls(cx) else {
                    return;
                };
                calls.update(cx, |calls, cx| {
                    calls.start_call(conversation_id, cx, move |outcome, cx| {
                        let _ = window_handle.update(cx, |_, _, cx| {
                            let _ = surfaces.update(cx, |surfaces, cx| {
                                let Some(modal) = surfaces
                                    .modal
                                    .as_mut()
                                    .filter(|modal| modal.user.id == user_id)
                                else {
                                    return;
                                };
                                modal.is_opening_conversation = false;
                                match outcome {
                                    Ok(()) => {
                                        surfaces.modal = None;
                                        surfaces.modal_details.show(None);
                                        cx.emit(ProfileSurfacesEvent::OpenedConversation(
                                            conversation_id,
                                        ));
                                        // Opening the conversation closes the call view: it
                                        // opens again over it.
                                        cx.defer(|cx| {
                                            if let Some(voice) = crate::voice::voice(cx) {
                                                voice.update(
                                                    cx,
                                                    crate::voice::Voice::open_call_view,
                                                );
                                            }
                                        });
                                    }
                                    // The start already under way ends this wait too.
                                    Err(CallStartFailure::AlreadyStarting) => {}
                                    Err(start_failure) => {
                                        modal.error = Some(format!(
                                            "{failure}. {}",
                                            crate::calls::words::call_start_failure_sentence(
                                                start_failure,
                                                strings(cx),
                                            )
                                        ));
                                        if let Some(calls) = crate::calls::calls(cx) {
                                            calls.update(
                                                cx,
                                                crate::calls::Calls::dismiss_start_problem,
                                            );
                                        }
                                    }
                                }
                                cx.notify();
                            });
                        });
                    });
                });
            });
        });
        cx.notify();
    }

    /// Add friend, Accept or Unblock: the button pressed is replaced once the instance answers,
    /// so focus moves on to the next control if it was still on it.
    pub(crate) fn change_relationship(
        &mut self,
        change: RelationshipChange,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(modal) = self.modal.as_mut() else {
            return;
        };
        if modal.is_changing_relationship {
            return;
        }
        modal.is_changing_relationship = true;
        modal.error = None;
        let user = modal.user.clone();
        let catalogue = strings(cx);
        let person = display_name(&user).to_owned();
        let (done, failure) = match change {
            RelationshipChange::AddFriend => (
                catalogue.friends().sent(&person),
                catalogue.members().friend_request_failed().to_owned(),
            ),
            RelationshipChange::Accept => (
                catalogue.friends().accepted(&person),
                catalogue.members().accept_failed().to_owned(),
            ),
            RelationshipChange::Unblock => (
                catalogue.friends().unblocked(&person),
                catalogue.friends().unblock_failed(&person),
            ),
        };
        // Unblock becomes Send message on the same handle; the friend button goes, so More next.
        let next_focus = match change {
            RelationshipChange::Unblock => modal.primary_focus.clone(),
            RelationshipChange::AddFriend | RelationshipChange::Accept => modal.more_focus.clone(),
        };
        let pressed = window.focused(cx);
        let surfaces = cx.entity().downgrade();
        let window_handle = window.window_handle();
        let user_id = user.id;
        let on_done = move |answer: Result<(), ClientError>, cx: &mut App| {
            let _ = window_handle.update(cx, |_, window, cx| {
                let has_focus_stayed = window.focused(cx) == pressed;
                let _ = surfaces.update(cx, |surfaces, cx| {
                    let Some(modal) = surfaces
                        .modal
                        .as_mut()
                        .filter(|modal| modal.user.id == user_id)
                    else {
                        return;
                    };
                    modal.is_changing_relationship = false;
                    match answer {
                        Ok(()) => {
                            modal.announcement = done;
                            if has_focus_stayed {
                                window.focus(&next_focus, cx);
                            }
                        }
                        Err(error) => {
                            modal.error = Some(describe_error(
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

    /// More: the person menu beside the button, without what the modal's buttons already do.
    fn open_more_menu(&mut self, cx: &mut Context<Self>) {
        let Some(modal) = self.modal.as_ref() else {
            return;
        };
        let Some(bounds) = modal
            .more_bounds
            .as_ref()
            .and_then(|bounds| *bounds.read(cx))
        else {
            return;
        };
        let request = PersonMenuRequest {
            opener: Some(modal.more_focus.clone()),
            is_in_profile: true,
            ..PersonMenuRequest::new(
                modal.user.clone(),
                menu_origin_beside(bounds, MenuAlignment::End),
            )
        };
        cx.emit(ProfileSurfacesEvent::OpenPersonMenu(request));
    }

    fn on_modal_key_down(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if event.keystroke.key == "escape" && !event.keystroke.modifiers.modified() {
            cx.stop_propagation();
            self.close_modal(window, cx);
        }
    }

    /// The actions under the status: the primary button, Call and More, then the friend button.
    #[allow(clippy::too_many_lines)] // One element tree, in the order it is drawn.
    fn modal_actions(&self, facts: &ProfileFacts, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let Some(modal) = self.modal.as_ref() else {
            return Vec::new();
        };
        let catalogue = strings(cx);
        let entity = cx.entity();
        let is_busy = modal.is_changing_relationship;
        let person = display_name(&facts.user).to_owned();
        let change_button = |id: &'static str, label: &'static str, change: RelationshipChange| {
            let entity = entity.clone();
            Button::new(id, label)
                .disabled(is_busy)
                .on_click(move |window, cx| {
                    entity.update(cx, |surfaces, cx| {
                        surfaces.change_relationship(change, window, cx);
                    });
                })
        };
        let is_blocked = facts.relationship == Some(RelationshipKind::Blocked);
        let primary = if facts.is_self {
            let entity = entity.clone();
            Button::new("profile-edit", catalogue.profile_card().edit_profile()).on_click(
                move |window, cx| {
                    entity.update(cx, |surfaces, cx| {
                        surfaces.close_modal(window, cx);
                        cx.emit(ProfileSurfacesEvent::EditProfile);
                    });
                },
            )
        } else if is_blocked {
            // No conversation while blocked: unblocking comes first.
            change_button(
                "profile-unblock",
                catalogue.friends().unblock(),
                RelationshipChange::Unblock,
            )
        } else {
            let entity = entity.clone();
            let is_opening = modal.is_opening_conversation;
            Button::new(
                "profile-send-message",
                if is_opening {
                    catalogue.members().opening()
                } else {
                    catalogue.members().send_message()
                },
            )
            .variant(ButtonVariant::Primary)
            .disabled(is_opening)
            .on_click(move |window, cx| {
                entity.update(cx, |surfaces, cx| surfaces.send_message(window, cx));
            })
        }
        .full_width()
        .track_focus(&modal.primary_focus);
        let more_entity = entity.clone();
        let more = Button::new(
            "profile-more",
            catalogue.profile_card().more_actions_for(&person),
        )
        .icon_only(IconName::DotsThree, COMPONENT_CHAT_HEADER_ICON_SIZE)
        .track_focus(&modal.more_focus)
        .on_click(move |_, cx| more_entity.update(cx, Self::open_more_menu));
        let more_bounds = modal.more_bounds.clone();
        // Calling: hidden where the instance has no calls, and for someone blocked.
        let can_call =
            !facts.is_self && !is_blocked && self.session.read(cx).state().is_calls_enabled();
        let call = can_call.then(|| {
            let entity = entity.clone();
            Button::new("profile-call", catalogue.calls().call_person(&person))
                .icon_only(IconName::Phone, COMPONENT_CHAT_HEADER_ICON_SIZE)
                .disabled(modal.is_opening_conversation)
                .on_click(move |window, cx| {
                    entity.update(cx, |surfaces, cx| surfaces.call(window, cx));
                })
        });
        let row_one = div()
            .flex()
            .gap(length(COMPONENT_PROFILE_MODAL_ACTION_GAP))
            .child(div().flex_1().min_w_0().child(primary))
            .children(call)
            .child(
                div()
                    .relative()
                    .children(more_bounds.map(bounds_recorder))
                    .child(more),
            )
            .into_any_element();
        let friend = match facts.relationship {
            _ if facts.is_self => None,
            None => Some(change_button(
                "profile-add-friend",
                catalogue.members().add_friend(),
                RelationshipChange::AddFriend,
            )),
            Some(RelationshipKind::IncomingRequest) => Some(change_button(
                "profile-accept",
                catalogue.members().accept_request(),
                RelationshipChange::Accept,
            )),
            // The reason is its label.
            Some(RelationshipKind::OutgoingRequest) => Some(
                Button::new("profile-request-sent", catalogue.members().request_sent())
                    .disabled(true),
            ),
            Some(RelationshipKind::Friend | RelationshipKind::Blocked) => None,
        };
        std::iter::once(row_one)
            .chain(friend.map(|button| {
                button
                    .full_width()
                    .track_focus(&modal.friend_focus)
                    .into_any_element()
            }))
            .collect()
    }

    /// The identity column: the person's card, as framed in Profile settings (`ProfileCard`).
    #[allow(clippy::too_many_lines)] // One card, in the order it is drawn.
    fn modal_identity(
        &self,
        facts: &ProfileFacts,
        banner_width: f32,
        // The card's top corners at the banner: GPUI does not clip children to rounded corners.
        (top_left_radius, top_right_radius): (f32, f32),
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let tokens = theme(cx);
        let catalogue = strings(cx);
        let surface = color(tokens.component_profile_modal_background);
        let animation = picture_animation(cx);
        let user = &facts.user;
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
                .rounded_tl(length(top_left_radius))
                .rounded_tr(length(top_right_radius))
                .bg(color(tokens.component_profile_popover_banner))
                .child(
                    img(picture_for_box(
                        DownloadableFile::ProfileImage(banner.id),
                        banner_width,
                        COMPONENT_PROFILE_POPOVER_BANNER_IMAGE_HEIGHT,
                        animation,
                        window,
                    ))
                    .size_full()
                    .rounded_tl(length(top_left_radius))
                    .rounded_tr(length(top_right_radius))
                    .object_fit(ObjectFit::Cover)
                    .with_fallback(move || div().size_full().bg(tint).into_any_element()),
                ),
            // No banner picture: the strip takes the person's avatar tint.
            None => div()
                .flex_none()
                .w_full()
                .h(length(COMPONENT_PROFILE_POPOVER_BANNER_HEIGHT))
                .rounded_tl(length(top_left_radius))
                .rounded_tr(length(top_right_radius))
                .bg(tint),
        };
        let name = display_name(user).to_owned();
        // The ring is drawn outside the avatar, cut from the card, one gap below the banner's
        // middle line.
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
            .child(
                Avatar::new(person, COMPONENT_PROFILE_POPOVER_AVATAR_SIZE)
                    .presence(shown_presence(facts.presence), surface)
                    .status_size(COMPONENT_STATUS_SIZE_LARGE),
            );
        let role_icon_element = facts.role_icon.as_ref().and_then(|role| {
            let role_name: SharedString = role.name.clone().into();
            role_icon(role, COMPONENT_ROLE_ICON_MESSAGE_SIZE, window, cx).map(|icon| {
                div()
                    .id("profile-modal-role-icon")
                    .role(Role::Image)
                    .aria_label(role_name.clone())
                    .tooltip(move |_, cx| Tooltip::view(role_name.clone(), cx))
                    .tooltip_show_delay(Tooltip::show_delay())
                    .child(icon)
            })
        });
        let actions = self.modal_actions(facts, cx);
        let (announcement, error) = self.modal.as_ref().map_or_else(
            || (String::new(), None),
            |modal| (modal.announcement.clone(), modal.error.clone()),
        );
        let status = facts.custom_status.clone().map(|status| {
            with_type(div(), TYPE_BODY)
                .id("profile-modal-status")
                .aria_label(format!("{}, {status}", catalogue.custom_status().field()))
                .max_w_full()
                .px(length(COMPONENT_PROFILE_MODAL_STATUS_PADDING_INLINE))
                .py(length(COMPONENT_PROFILE_MODAL_STATUS_PADDING_BLOCK))
                .rounded(length(COMPONENT_PROFILE_MODAL_STATUS_RADIUS))
                .bg(color(tokens.component_profile_modal_status_background))
                .text_color(color(tokens.component_profile_modal_status_text))
                .child(status)
        });

        div()
            .flex()
            .flex_col()
            .child(banner)
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(length(COMPONENT_PROFILE_POPOVER_TEXT_GAP))
                    .px(length(COMPONENT_PROFILE_MODAL_IDENTITY_PADDING))
                    .pb(length(COMPONENT_PROFILE_MODAL_IDENTITY_PADDING))
                    .child(avatar)
                    .child(
                        with_type(div(), TYPE_HEADING_LG)
                            .id("profile-modal-name")
                            .role(Role::Heading)
                            .aria_level(2)
                            .aria_label(name.clone())
                            .mt(length(SPACE_XS))
                            .flex()
                            .items_center()
                            .gap(length(COMPONENT_ROLE_ICON_GAP))
                            .text_color(color(tokens.component_profile_popover_name_text))
                            .child(div().min_w_0().child(name))
                            .children(role_icon_element),
                    )
                    // The avatar's presence is said by the dialog's description.
                    .child(
                        with_type(div(), TYPE_BODY_SM)
                            .text_color(color(tokens.component_profile_modal_presence_text))
                            .child(crate::home::presence_name(catalogue, facts.presence)),
                    )
                    .children(status)
                    .child(
                        div()
                            .mt(length(SPACE_XS))
                            .flex()
                            .flex_col()
                            .gap(length(COMPONENT_PROFILE_MODAL_ACTION_GAP))
                            .children(actions),
                    )
                    .child(
                        div()
                            .id("profile-modal-announcement")
                            .live_polite(SharedString::from(announcement))
                            .h_0()
                            .overflow_hidden(),
                    )
                    .children(error.map(|error| {
                        with_type(div(), TYPE_BODY)
                            .id("profile-modal-error")
                            .live_assertive(error.clone())
                            .text_color(color(tokens.component_profile_popover_error_text))
                            .child(error)
                    })),
            )
            .into_any_element()
    }

    /// The details column: About me, links, roles, member since; a section only when it has
    /// something.
    #[allow(clippy::too_many_lines)] // One column, in the order it is drawn.
    fn modal_details(
        &mut self,
        facts: &ProfileFacts,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        let tokens = theme(cx);
        let catalogue = strings(cx);
        let mut sections = Vec::new();
        let load = self
            .modal
            .as_ref()
            .map(|_| self.modal_details.current().clone());
        match load {
            Some(UserProfileLoad::Loaded { user_id, profile }) if user_id == facts.user.id => {
                if let Some(body) = self.about_body(&profile.about_me) {
                    sections.push(
                        section("profile-about", catalogue.profile_details().about_me(), cx)
                            .child(
                                with_type(div(), TYPE_BODY)
                                    .text_color(color(tokens.component_profile_popover_about_text))
                                    .child(MessageMarkdown::new(
                                        ("profile-modal-about", user_id.raw()),
                                        body,
                                    )),
                            )
                            .into_any_element(),
                    );
                }
                let links = shown_profile_links(&profile.links);
                let has_links = !links.is_empty();
                if has_links {
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
                if facts.is_self && profile.about_me.trim().is_empty() && !has_links {
                    sections.push(
                        with_type(div(), TYPE_BODY)
                            .text_color(color(tokens.component_profile_modal_date_label_text))
                            .child(catalogue.profile_card().no_details_yet())
                            .into_any_element(),
                    );
                }
            }
            Some(UserProfileLoad::Failed { user_id }) if user_id == facts.user.id => {
                sections.push(
                    with_type(div(), TYPE_BODY_SM)
                        .text_color(color(tokens.component_profile_modal_date_label_text))
                        .child(catalogue.profile_details().load_failed())
                        .into_any_element(),
                );
            }
            // Nothing while it loads: one small request, a skeleton would flash longer.
            _ => {}
        }
        let person = display_name(&facts.user).to_owned();
        if let Some(chips) = facts
            .server
            .as_ref()
            .and_then(|server| server.role_chips(&person, None, catalogue))
        {
            sections.push(
                section("profile-roles", catalogue.profile_card().roles(), cx)
                    .child(chips)
                    .into_any_element(),
            );
        }
        let date_figure = |id: &'static str, label: String, unix_milliseconds: u64| {
            let date = day_divider_label(local_day_of(unix_milliseconds), catalogue);
            div()
                .id(id)
                .aria_label(format!("{label}, {date}"))
                .flex()
                .flex_col()
                .child(
                    with_type(div(), TYPE_BODY_SM)
                        .text_color(color(tokens.component_profile_modal_date_label_text))
                        .child(label),
                )
                .child(
                    with_type(div(), TYPE_BODY)
                        .text_color(color(tokens.component_profile_modal_date_text))
                        .child(date),
                )
        };
        sections.push(
            section("profile-since", catalogue.profile_card().member_since(), cx)
                .child(
                    div()
                        .flex()
                        .flex_wrap()
                        .gap_x(length(COMPONENT_PROFILE_MODAL_DATES_GAP))
                        .gap_y(length(SPACE_XS))
                        .child(date_figure(
                            "profile-since-mellow",
                            catalogue.profile_card().mellow().to_owned(),
                            facts.joined_mellow_at_unix_milliseconds,
                        ))
                        .children(facts.server.as_ref().map(|server| {
                            date_figure(
                                "profile-since-server",
                                server.name.clone(),
                                server.joined_at_unix_milliseconds,
                            )
                        })),
                )
                .into_any_element(),
        );
        sections
    }

    #[allow(clippy::too_many_lines)] // One layer, in the order it is drawn.
    pub(super) fn render_modal(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let modal = self.modal.as_mut()?;
        modal
            .more_bounds
            .get_or_insert_with(|| cx.new(|_| None::<Bounds<Pixels>>));
        let container_focus = modal.container_focus.clone();
        let user = modal.user.clone();
        let tokens = theme(cx);
        let catalogue = strings(cx);
        let state = self.session.read(cx).state();
        let facts = profile_facts(&user, self.guild_id, state);
        let name = display_name(&facts.user).to_owned();
        // Under the title band and inside the window's frame (DESIGN.md §4.2, Window chrome).
        let content_bounds = crate::window_chrome::client_content_bounds(window);
        let viewport_height =
            crate::theme::pixels_at_default_zoom(content_bounds.size.height, window);
        let window_width = window_width_at_default_zoom(window);
        // The card and its margins need the room; below, a sheet from the bottom.
        let is_wide =
            window_width >= COMPONENT_PROFILE_MODAL_WIDTH + 2.0 * COMPONENT_PROFILE_MODAL_MARGIN;
        let banner_width = if is_wide {
            COMPONENT_PROFILE_MODAL_IDENTITY_WIDTH
        } else {
            window_width
        };
        let is_dark = *tokens == DARK_THEME;
        // Inside the dark theme's outline, the inner corners follow it concentrically.
        let inner_radius = COMPONENT_PROFILE_MODAL_RADIUS
            - if is_dark {
                COMPONENT_PROFILE_MODAL_BORDER_WIDTH
            } else {
                0.0
            };
        let top_corners = if is_wide {
            (inner_radius, 0.0)
        } else {
            (
                COMPONENT_PROFILE_MODAL_SHEET_RADIUS,
                COMPONENT_PROFILE_MODAL_SHEET_RADIUS,
            )
        };
        let identity = self.modal_identity(&facts, banner_width, top_corners, window, cx);
        let details = self.modal_details(&facts, window, cx);
        let entity = cx.entity();
        let close_entity = entity.clone();
        let scrim_entity = entity.clone();

        let close = IconButton::new(
            "profile-modal-close",
            IconName::X,
            catalogue.common().close(),
        )
        .sizes(
            if is_wide {
                COMPONENT_PROFILE_MODAL_CLOSE_SIZE
            } else {
                SIZE_HIT_WEB_MIN
            },
            COMPONENT_USER_PANEL_BUTTON_ICON_SIZE,
        )
        .on_click(move |window, cx| {
            close_entity.update(cx, |surfaces, cx| surfaces.close_modal(window, cx));
        });
        let close = div()
            .absolute()
            .top(length(COMPONENT_PROFILE_MODAL_CLOSE_INSET))
            .right(length(COMPONENT_PROFILE_MODAL_CLOSE_INSET))
            // On a sheet it sits on the banner: a plate reads over any picture.
            .when(!is_wide, |close| {
                close.rounded(length(RADIUS_SM)).bg(color(
                    tokens.component_profile_modal_close_on_picture_background,
                ))
            })
            .child(close);

        let details_column = div()
            .id("profile-modal-details")
            .flex_1()
            .min_w_0()
            .flex()
            .flex_col()
            .gap(length(COMPONENT_PROFILE_MODAL_SECTION_GAP))
            .p(length(COMPONENT_PROFILE_MODAL_DETAILS_PADDING))
            .when(is_wide, gpui::StatefulInteractiveElement::overflow_y_scroll)
            .children(details);

        let card = div()
            .id("profile-modal")
            .debug_selector(|| "profile-modal".to_owned())
            .role(Role::Dialog)
            .aria_label(name)
            .aria_description(crate::home::presence_name(catalogue, facts.presence))
            .track_focus(&container_focus)
            .tab_group()
            .key_context("ProfileModal")
            .on_key_down(cx.listener(Self::on_modal_key_down))
            .on_action(cx.listener(|surfaces, _: &FocusNextControl, window, cx| {
                if let Some(modal) = surfaces.modal.as_ref() {
                    focus_next_inside(&modal.container_focus, window, cx);
                }
            }))
            .on_action(
                cx.listener(|surfaces, _: &FocusPreviousControl, window, cx| {
                    if let Some(modal) = surfaces.modal.as_ref() {
                        focus_previous_inside(&modal.container_focus, window, cx);
                    }
                }),
            )
            // Clicks on the card stay on the card; only the scrim closes.
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .relative()
            .flex()
            .overflow_hidden()
            .bg(color(tokens.component_profile_modal_details_background))
            .text_color(color(tokens.component_profile_popover_about_text))
            .shadow(shadow(tokens.component_profile_modal_shadow))
            .map(|card| {
                if is_wide {
                    let most = viewport_height - 2.0 * COMPONENT_PROFILE_MODAL_MARGIN;
                    card.flex_row()
                        .w(length(COMPONENT_PROFILE_MODAL_WIDTH))
                        .min_h(length(COMPONENT_PROFILE_MODAL_MIN_HEIGHT.min(most)))
                        .max_h(length(most))
                        .rounded(length(COMPONENT_PROFILE_MODAL_RADIUS))
                        .when(is_dark, |card| {
                            card.border(length(COMPONENT_PROFILE_MODAL_BORDER_WIDTH))
                                .border_color(color(tokens.component_profile_modal_border))
                        })
                        .child(
                            div()
                                .id("profile-modal-identity")
                                .flex_none()
                                .w(length(COMPONENT_PROFILE_MODAL_IDENTITY_WIDTH))
                                .rounded_l(length(inner_radius))
                                .bg(color(tokens.component_profile_modal_background))
                                .overflow_y_scroll()
                                .child(identity),
                        )
                        .child(details_column)
                } else {
                    // One column in one scroll, from the top gap to the bottom edge.
                    card.w_full()
                        .h(length(
                            viewport_height - COMPONENT_PROFILE_MODAL_SHEET_TOP_GAP,
                        ))
                        .rounded_t(length(COMPONENT_PROFILE_MODAL_SHEET_RADIUS))
                        .child(
                            div()
                                .id("profile-modal-sheet")
                                .size_full()
                                .flex()
                                .flex_col()
                                .overflow_y_scroll()
                                .child(
                                    div()
                                        .flex_none()
                                        .bg(color(tokens.component_profile_modal_background))
                                        .child(identity),
                                )
                                .child(details_column),
                        )
                }
            })
            .child(close);
        let card = if is_wide {
            // GPUI cannot scale a box yet, so the card settles in by fading (as the dialogs).
            card.with_animation(
                "profile-modal-enter",
                Animation::new(layer_duration(COMPONENT_DIALOG_ENTER_DURATION, cx))
                    .with_easing(|progress| easing(MOTION_EASING_SETTLE).sample(progress)),
                |card, progress| card.opacity(progress.min(1.0)),
            )
            .into_any_element()
        } else {
            // Rises from the bottom.
            let rise = viewport_height - COMPONENT_PROFILE_MODAL_SHEET_TOP_GAP;
            card.with_animation(
                "profile-sheet-enter",
                Animation::new(layer_duration(MOTION_DURATION_SLOW, cx))
                    .with_easing(|progress| easing(MOTION_EASING_ENTER).sample(progress)),
                move |card, progress| card.top(length(rise * (1.0 - progress.min(1.0)))),
            )
            .into_any_element()
        };

        let scrim = div()
            .id("profile-modal-scrim")
            .occlude()
            .map(|scrim| crate::window_chrome::sized_to_window_content(scrim, window))
            .flex()
            .flex_col()
            .items_center()
            .map(|scrim| {
                if is_wide {
                    scrim.justify_center()
                } else {
                    scrim.justify_end()
                }
            })
            .bg(color(tokens.component_profile_modal_scrim))
            .on_mouse_down(MouseButton::Left, move |_, window, cx| {
                scrim_entity.update(cx, |surfaces, cx| surfaces.close_modal(window, cx));
            })
            .child(card)
            .with_animation(
                "profile-modal-scrim",
                Animation::new(layer_duration(MOTION_DURATION_BASE, cx))
                    .with_easing(|progress| easing(MOTION_EASING_STANDARD).sample(progress)),
                gpui::Styled::opacity,
            );
        Some(
            deferred(anchored().position(content_bounds.origin).child(scrim))
                .with_priority(MODAL_LAYER_PRIORITY)
                .into_any_element(),
        )
    }
}

/// A layer's entrance: at once under reduced motion, where GPUI needs a non-zero length.
fn layer_duration(token: crate::theme::tokens::AnimationDuration, cx: &App) -> std::time::Duration {
    animation_duration(token, cx).max(std::time::Duration::from_millis(1))
}

/// A titled part of the details ("About me", "Links", "Roles", "Member since").
fn section(id: &'static str, title: &'static str, cx: &App) -> gpui::Stateful<gpui::Div> {
    let tokens = theme(cx);
    div()
        .id(id)
        .flex()
        .flex_col()
        .gap(length(COMPONENT_PROFILE_MODAL_SECTION_LABEL_GAP))
        .child(
            with_type(div(), TYPE_CATEGORY)
                .id(ElementId::NamedChild(
                    std::sync::Arc::new(id.into()),
                    "heading".into(),
                ))
                .role(Role::Heading)
                .aria_level(3)
                .aria_label(title)
                .text_color(color(tokens.component_profile_popover_section_label_text))
                .child(title),
        )
}

/// A link with its site's icon, stored by the instance, or a link glyph until it is fetched; its
/// label, or its host, names it. It opens in the browser.
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
                    PictureAnimation::StillFrame,
                    window,
                ))
                .size(length(COMPONENT_PROFILE_POPOVER_LINK_ICON_SIZE))
                // Its plate's corners: GPUI does not clip children to rounded corners.
                .rounded(length(COMPONENT_PROFILE_POPOVER_LINK_ICON_RADIUS))
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

/// The link glyph: a link without a stored icon, or whose icon failed to load.
fn link_glyph(colour: gpui::Hsla) -> impl IntoElement {
    icon(
        IconName::LinkSimple,
        COMPONENT_PROFILE_POPOVER_LINK_ICON_SIZE,
        colour,
    )
}
