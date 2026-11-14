//! Who voted for one answer of a poll (`PollVotersDialog.svelte`, DESIGN.md §6.83), listed as who
//! reacted is: avatar and name, a page at a time, "Show more" while pages come full.

use gpui::{
    Context, ElementId, Entity, InteractiveElement as _, IntoElement, ParentElement as _, Render,
    Role, StatefulInteractiveElement as _, Styled as _, Window, div, prelude::FluentBuilder as _,
};
use mellow_client_core::DownloadableFile;
use mellow_client_core::shared_logic::format::display_name;
use mellow_protocol::{PublicUser, Snowflake};

use crate::components::{
    Avatar, AvatarPerson, Button, ButtonVariant, LiveRegion as _, is_picture_row_pointed_at,
    track_picture_row_pointer, with_type,
};
use crate::i18n::strings;
use crate::messages::picture_for_box;
use crate::pictures::chosen_profile_image_animation;
use crate::session::{POLL_VOTERS_PAGE_SIZE, Session};
use crate::theme::tokens::{
    COMPONENT_DIALOG_SECTION_GAP, COMPONENT_MEMBER_ROW_AVATAR_SIZE, COMPONENT_MEMBER_ROW_HEIGHT,
    SPACE_SM, TYPE_BODY, TYPE_MEMBER_NAME,
};
use crate::theme::{color, length, theme};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LoadState {
    Loading,
    Loaded,
    Failed,
}

pub(super) struct PollVotersBody {
    session: Entity<Session>,
    channel_id: Snowflake,
    message_id: Snowflake,
    answer_id: u32,
    voters: Vec<PublicUser>,
    load_state: LoadState,
    has_more: bool,
    is_loading_more: bool,
    /// "Couldn't load more", under the list.
    more_failed: bool,
}

impl PollVotersBody {
    pub(super) fn new(
        session: Entity<Session>,
        channel_id: Snowflake,
        message_id: Snowflake,
        answer_id: u32,
        cx: &mut Context<Self>,
    ) -> Self {
        let mut body = Self {
            session,
            channel_id,
            message_id,
            answer_id,
            voters: Vec::new(),
            load_state: LoadState::Loading,
            has_more: false,
            is_loading_more: false,
            more_failed: false,
        };
        body.load_page(cx);
        body
    }

    /// The first page, or the one after the last voter shown.
    fn load_page(&mut self, cx: &mut Context<Self>) {
        let after = self.voters.last().map(|voter| voter.id);
        let failure = strings(cx).polls().voters_failed().to_owned();
        let body = cx.entity().downgrade();
        let (channel_id, message_id, answer_id) =
            (self.channel_id, self.message_id, self.answer_id);
        self.session.update(cx, |session, cx| {
            session.list_poll_voters(
                channel_id,
                message_id,
                answer_id,
                after,
                failure,
                move |answer, cx| {
                    let _ = body.update(cx, |body, cx| {
                        body.is_loading_more = false;
                        match answer {
                            Ok(page) => {
                                body.has_more = page.len() == POLL_VOTERS_PAGE_SIZE as usize;
                                body.voters.extend(page);
                                body.load_state = LoadState::Loaded;
                            }
                            Err(_) if after.is_some() => body.more_failed = true,
                            Err(_) => body.load_state = LoadState::Failed,
                        }
                        cx.notify();
                    });
                },
                cx,
            );
        });
    }

    fn load_more(&mut self, cx: &mut Context<Self>) {
        if self.is_loading_more || self.voters.is_empty() {
            return;
        }
        self.is_loading_more = true;
        self.more_failed = false;
        self.load_page(cx);
        cx.notify();
    }
}

impl Render for PollVotersBody {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let tokens = theme(cx);
        let polls = strings(cx).polls();
        let note = |text: &'static str| {
            with_type(div(), TYPE_BODY)
                .text_color(color(tokens.color_text_muted))
                .child(text)
        };
        match self.load_state {
            LoadState::Loading => {
                return note(polls.loading_voters())
                    .id("poll-voters-loading")
                    .live_polite(polls.loading_voters())
                    .into_any_element();
            }
            LoadState::Failed => return note(polls.voters_failed()).into_any_element(),
            LoadState::Loaded if self.voters.is_empty() => {
                return note(polls.no_voters()).into_any_element();
            }
            LoadState::Loaded => {}
        }
        let count = self.voters.len();
        let people = self.voters.iter().enumerate().map(|(index, user)| {
            let row_id = ElementId::from(("poll-voter", index));
            let is_row_pointed_at = is_picture_row_pointed_at(&row_id, cx);
            let picture = user.avatar.as_ref().map(|avatar| {
                picture_for_box(
                    DownloadableFile::ProfileImage(avatar.id),
                    COMPONENT_MEMBER_ROW_AVATAR_SIZE,
                    COMPONENT_MEMBER_ROW_AVATAR_SIZE,
                    chosen_profile_image_animation(avatar, is_row_pointed_at, cx),
                    window,
                )
            });
            let name = display_name(user).to_owned();
            track_picture_row_pointer(div().id(row_id.clone()), row_id, window)
                .role(Role::ListItem)
                .aria_label(name.clone())
                .aria_position_in_set(index + 1)
                .aria_size_of_set(count)
                .flex()
                .items_center()
                .gap(length(SPACE_SM))
                .min_h(length(COMPONENT_MEMBER_ROW_HEIGHT))
                .child(Avatar::new(
                    AvatarPerson::from_user(user, picture),
                    COMPONENT_MEMBER_ROW_AVATAR_SIZE,
                ))
                .child(
                    with_type(div(), TYPE_MEMBER_NAME)
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .text_color(color(tokens.color_text_normal))
                        .child(name),
                )
        });
        let entity = cx.entity();
        div()
            .flex()
            .flex_col()
            .gap(length(COMPONENT_DIALOG_SECTION_GAP))
            .child(
                div()
                    .id("poll-voters")
                    .role(Role::List)
                    .flex()
                    .flex_col()
                    .children(people),
            )
            .when(self.more_failed, |body| {
                body.child(
                    with_type(div(), TYPE_BODY)
                        .id("poll-voters-more-failed")
                        .live_assertive(polls.more_voters_failed())
                        .text_color(color(tokens.component_dialog_error_text))
                        .child(polls.more_voters_failed()),
                )
            })
            .when(self.has_more, |body| {
                body.child(
                    div().flex().child(
                        Button::new("poll-voters-more", polls.more_voters())
                            .variant(ButtonVariant::Secondary)
                            .disabled(self.is_loading_more)
                            .on_click(move |_, cx| {
                                entity.update(cx, PollVotersBody::load_more);
                            }),
                    ),
                )
            })
            .into_any_element()
    }
}
