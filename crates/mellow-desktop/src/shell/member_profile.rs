//! A member's profile in the side sheet (`MemberList.svelte`, `ProfileCard.svelte`,
//! `ProfileDetails.svelte`, DESIGN.md §5.7, ADR-0050): choosing a member shows the banner, or the
//! strip in their avatar tint, the avatar cut out of the sheet, their name, "About me" and links
//! (read when it opens, as `user-profile-loader.svelte.ts`), and "Send message". Back returns to
//! the list, on the row it came from.
//! shortcut: the web's friend and block buttons under "Send message" come with the friends work
//! of M3 step 4; a blocked person's profile shows no action until then.

use std::rc::Rc;

use gpui::{
    AnyElement, App, Context, ElementId, FocusHandle, InteractiveElement as _, IntoElement,
    ObjectFit, ParentElement as _, Role, SharedString, StatefulInteractiveElement as _,
    Styled as _, StyledImage as _, Window, div, img, prelude::FluentBuilder as _,
};
use mellow_client_core::DownloadableFile;
use mellow_client_core::shared_logic::format::display_name;
use mellow_client_core::shared_logic::markdown::parse_inline_markdown;
use mellow_client_core::shared_logic::profiles::{
    ShownProfileLink, UserProfileLoad, UserProfileLoader, shown_profile_links,
};
use mellow_protocol::{PublicUser, RelationshipKind, Snowflake};

use super::DuskShell;
use crate::components::{
    Avatar, AvatarPerson, Button, ButtonVariant, Tooltip, avatar_tint, icon, inset_focus_ring,
    is_focus_visible, remembered_focus_handle, remembered_hover, with_type,
};
use crate::i18n::strings;
use crate::messages::{MessageBody, MessageMarkdown, picture_animation, picture_for_box};
use crate::pictures::Animation;
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
    about: Option<(String, Rc<MessageBody>)>,
    is_opening_conversation: bool,
    error: Option<String>,
    back_focus: FocusHandle,
    /// The row Back returns to, focused once the list is drawn again.
    row_to_focus: Option<Snowflake>,
}

impl MemberProfileState {
    pub(super) fn new(cx: &mut App) -> Self {
        Self {
            shown: None,
            loader: UserProfileLoader::default(),
            about: None,
            is_opening_conversation: false,
            error: None,
            back_focus: cx.focus_handle().tab_stop(true),
            row_to_focus: None,
        }
    }

    #[cfg(test)]
    pub(super) fn load(&self) -> &UserProfileLoad {
        self.loader.current()
    }

    pub(super) fn back_focus(&self) -> &FocusHandle {
        &self.back_focus
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
        self.error = None;
    }

    fn about_body(&mut self, about_me: &str) -> Option<Rc<MessageBody>> {
        if self.about.as_ref().is_none_or(|(text, _)| text != about_me) {
            let body = MessageBody {
                blocks: parse_inline_markdown(about_me),
                is_jumbo: false,
                names_reader: false,
            };
            self.about = Some((about_me.to_owned(), Rc::new(body)));
        }
        self.about
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
        profile.shown = Some(ShownProfile { guild_id, user_id });
        profile.error = None;
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
        self.member_profile.row_to_focus = self.member_profile.shown.map(|shown| shown.user_id);
        self.member_profile.forget();
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
        self.member_profile.error = None;
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
                            Err(message) => shell.member_profile.error = Some(message),
                        }
                        cx.notify();
                    });
                });
            });
        });
        cx.notify();
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
        let banner = match user.banner {
            // A picture that fails leaves the banner colour in its place.
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
                    .object_fit(ObjectFit::Cover),
                ),
            // No banner picture: the strip takes the person's avatar tint, as in the Dusk mockup.
            None => div()
                .flex_none()
                .w_full()
                .h(length(COMPONENT_PROFILE_POPOVER_BANNER_HEIGHT))
                .bg(avatar_tint(tokens, person.tint_number)),
        };
        let name = display_name(user).to_owned();
        let state = self.session.read(cx).state();
        let is_self = state.current_user().is_some_and(|me| me.id == user.id);
        let is_blocked = state
            .relationship(user.id)
            .map(|relationship| relationship.kind)
            == Some(RelationshipKind::Blocked);

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
        let error = self.member_profile.error.clone().map(|error| {
            with_type(div(), TYPE_BODY)
                .id("profile-error")
                .role(Role::Alert)
                .aria_label(error.clone())
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
                .object_fit(ObjectFit::Contain),
            ),
        None => plate.child(icon(
            IconName::LinkSimple,
            COMPONENT_PROFILE_POPOVER_LINK_ICON_SIZE,
            color(tokens.component_profile_popover_link_icon_fallback),
        )),
    };
    let url = link.url.clone();
    let tooltip: SharedString = link.url.into();
    div()
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
        })
        .into_any_element()
}
