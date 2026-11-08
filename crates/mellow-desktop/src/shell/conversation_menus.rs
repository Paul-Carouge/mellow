//! The menus of Home's conversation rows and of a group's header (`openConversationMenu` in
//! `ChannelSidebar.svelte`, DESIGN.md §6.33): a one-to-one conversation's row opens the other
//! person's menu, which also copies the conversation's ID and mutes it; a group's row, and its
//! name in the header, open the group's menu. Home draws both.

use gpui::{
    AnyElement, App, Context, ElementId, FocusHandle, InteractiveElement as _, IntoElement,
    KeyDownEvent, MouseButton, MouseDownEvent, ParentElement as _, Pixels, Point, Styled as _,
    Window, div,
};
use mellow_protocol::Snowflake;

use super::DuskShell;
use crate::components::{
    MenuOrigin, bounds_recorder, is_context_menu_keystroke, menu_origin_at_pointer,
    menu_origin_beside_remembered, remembered_bounds, remembered_focus_handle,
};
use crate::home::PersonMenuRequest;

impl DuskShell {
    /// `row`, the sidebar row of conversation `conversation_id`, opening its menu on a
    /// right-click, or beside it on Shift+F10 and the Menu key while it has focus.
    pub(super) fn with_conversation_menu(
        row: AnyElement,
        conversation_id: Snowflake,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let row_id = ElementId::from(("conversation-row", conversation_id.raw()));
        let opener = remembered_focus_handle(&row_id, window, cx);
        let bounds = remembered_bounds(&row_id, window, cx);
        let recorder = bounds_recorder(bounds.clone());
        let shell = cx.entity();
        let open = move |at: Option<Point<Pixels>>, window: &mut Window, cx: &mut App| {
            let origin = match at {
                Some(position) => Some(menu_origin_at_pointer(position)),
                None => menu_origin_beside_remembered(&bounds, cx),
            };
            let Some(origin) = origin else {
                return false;
            };
            shell.update(cx, |shell, cx| {
                shell.open_conversation_menu(
                    conversation_id,
                    origin,
                    Some(opener.clone()),
                    window,
                    cx,
                );
            });
            true
        };
        let open_by_key = open.clone();
        div()
            .relative()
            .on_mouse_down(
                MouseButton::Right,
                move |event: &MouseDownEvent, window, cx| {
                    cx.stop_propagation();
                    open(Some(event.position), window, cx);
                },
            )
            .on_key_down(move |event: &KeyDownEvent, window, cx| {
                if is_context_menu_keystroke(&event.keystroke) && open_by_key(None, window, cx) {
                    cx.stop_propagation();
                }
            })
            .child(row)
            .child(recorder)
            .into_any_element()
    }

    /// A group's menu, or the other person's for a one-to-one conversation.
    pub(super) fn open_conversation_menu(
        &mut self,
        conversation_id: Snowflake,
        origin: MenuOrigin,
        opener: Option<FocusHandle>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let state = self.session.read(cx).state();
        let Some(conversation) = state.direct_message_channel(conversation_id) else {
            return;
        };
        if conversation.owner_id.is_some() {
            self.home.update(cx, |home, cx| {
                home.open_group_menu(conversation_id, origin, opener, window, cx);
            });
            return;
        }
        let current_user_id = state.current_user().map(|user| user.id);
        let Some(recipient) = conversation
            .recipients
            .iter()
            .find(|recipient| Some(recipient.id) != current_user_id)
            .cloned()
        else {
            return;
        };
        let request = PersonMenuRequest {
            opener,
            conversation_id: Some(conversation_id),
            ..PersonMenuRequest::new(recipient, origin)
        };
        self.home
            .update(cx, |home, cx| home.open_person_menu(request, window, cx));
    }

    /// The open group's id, for its header's menu; `None` elsewhere.
    pub(super) fn open_group_id(&self, cx: &App) -> Option<Snowflake> {
        self.navigation.selected_channel_id().filter(|channel_id| {
            self.session
                .read(cx)
                .state()
                .direct_message_channel(*channel_id)
                .is_some_and(|conversation| conversation.owner_id.is_some())
        })
    }

    /// A right-click on the group's name in the header: its menu at the pointer. The row in the
    /// sidebar is the keyboard's way to it.
    pub(super) fn group_header_menu_listener(
        group_id: Snowflake,
        cx: &Context<Self>,
    ) -> impl Fn(&MouseDownEvent, &mut Window, &mut App) + 'static {
        let shell = cx.entity();
        move |event, window, cx| {
            cx.stop_propagation();
            shell.update(cx, |shell, cx| {
                let origin = menu_origin_at_pointer(event.position);
                shell.open_conversation_menu(group_id, origin, None, window, cx);
            });
        }
    }
}
