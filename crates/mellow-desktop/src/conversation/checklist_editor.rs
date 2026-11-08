//! A checklist in editing (`ChecklistCard.svelte`'s editing mode, ADR-0047): each item is a field
//! (Enter or leaving it renames; blank or unchanged keeps it) with a Remove button, never on the
//! last item, and a field at the end adds one. Nothing is guessed: the card waits for the
//! instance's answer, and a refusal is said above the composer.

use gpui::{
    AppContext as _, Context, Entity, Focusable as _, IntoElement, ParentElement as _, Render,
    Styled as _, Subscription, WeakEntity, Window, div, prelude::FluentBuilder as _,
};
use gpui_component::input::{InputEvent, InputState};
use mellow_client_core::shared_logic::tracker_form::checklist_item_text;
use mellow_protocol::{Checklist, MAXIMUM_CHECKLIST_ITEMS, Snowflake};

use super::ConversationView;
use crate::components::{Button, ButtonVariant, IconButton, IconButtonTone, TextField, with_type};
use crate::i18n::strings;
use crate::session::{ChecklistItemChange, Session};
use crate::theme::icons::IconName;
use crate::theme::tokens::{
    COMPONENT_POLL_ANSWER_GAP, COMPONENT_POLL_MARK_GLYPH_SIZE, SIZE_HIT_WEB_MIN, SPACE_2XS,
    TYPE_BODY_SM,
};
use crate::theme::{color, length, theme};

/// One item's field, and the text the instance holds for it.
struct ItemField {
    item_id: u32,
    saved_text: String,
    input: Entity<InputState>,
    _events: Subscription,
}

pub(super) struct ChecklistEditor {
    session: Entity<Session>,
    view: WeakEntity<ConversationView>,
    channel_id: Snowflake,
    message_id: Snowflake,
    items: Vec<ItemField>,
    new_item: Entity<InputState>,
    _subscriptions: Vec<Subscription>,
}

impl ChecklistEditor {
    pub(super) fn new(
        session: Entity<Session>,
        view: WeakEntity<ConversationView>,
        channel_id: Snowflake,
        message_id: Snowflake,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let placeholder = strings(cx).checklists().new_item();
        let new_item = cx.new(|cx| InputState::new(window, cx).placeholder(placeholder));
        let subscriptions = vec![
            // Items others add, rename or remove meanwhile show at once.
            cx.observe(&session, |_, _, cx| cx.notify()),
            cx.subscribe_in(
                &new_item,
                window,
                |editor: &mut Self, _, event: &InputEvent, window, cx| {
                    if matches!(event, InputEvent::PressEnter { .. }) {
                        editor.add(window, cx);
                    }
                },
            ),
        ];
        Self {
            session,
            view,
            channel_id,
            message_id,
            items: Vec::new(),
            new_item,
            _subscriptions: subscriptions,
        }
    }

    pub(super) fn focus_new_item(&self, window: &mut Window, cx: &mut Context<Self>) {
        let focus = self.new_item.read(cx).focus_handle(cx);
        window.focus(&focus, cx);
    }

    fn checklist(&self, cx: &gpui::App) -> Option<Checklist> {
        self.session
            .read(cx)
            .state()
            .cached_message(self.channel_id, self.message_id)
            .and_then(|message| message.checklist.clone())
    }

    /// Keeps one field per item, by id: a renamed item's field takes its new text unless it is
    /// being typed in.
    fn follow_items(&mut self, checklist: &Checklist, window: &mut Window, cx: &mut Context<Self>) {
        let mut fields = std::mem::take(&mut self.items);
        for item in &checklist.items {
            let field = match fields.iter().position(|field| field.item_id == item.id) {
                Some(index) => {
                    let mut field = fields.swap_remove(index);
                    if field.saved_text != item.text {
                        field.saved_text.clone_from(&item.text);
                        let is_typing = field.input.read(cx).focus_handle(cx).is_focused(window);
                        if !is_typing {
                            let text = item.text.clone();
                            field
                                .input
                                .update(cx, |input, cx| input.set_value(text, window, cx));
                        }
                    }
                    field
                }
                None => Self::item_field(item.id, &item.text, window, cx),
            };
            self.items.push(field);
        }
    }

    fn item_field(
        item_id: u32,
        text: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> ItemField {
        let input = cx.new(|cx| InputState::new(window, cx).default_value(text.to_owned()));
        let events = cx.subscribe_in(
            &input,
            window,
            move |editor: &mut Self, _, event: &InputEvent, window, cx| {
                if matches!(event, InputEvent::PressEnter { .. } | InputEvent::Blur) {
                    editor.rename(item_id, window, cx);
                }
            },
        );
        ItemField {
            item_id,
            saved_text: text.to_owned(),
            input,
            _events: events,
        }
    }

    fn show_error(&self, sentence: String, cx: &mut gpui::App) {
        let channel_id = self.channel_id;
        let _ = self.view.update(cx, |view, cx| {
            view.show_action_error(channel_id, sentence, cx);
        });
    }

    fn change(
        &self,
        item_change: ChecklistItemChange,
        cx: &mut Context<Self>,
        on_done: impl FnOnce(&mut Self, bool, &mut Context<Self>) + 'static,
    ) {
        let summary = strings(cx).checklists().change_failed().to_owned();
        let editor = cx.entity().downgrade();
        let (channel_id, message_id) = (self.channel_id, self.message_id);
        self.session.update(cx, |session, cx| {
            session.change_checklist_item(
                channel_id,
                message_id,
                item_change,
                summary,
                move |answer, cx| {
                    let _ = editor.update(cx, |editor, cx| {
                        if let Err(sentence) = &answer {
                            editor.show_error(sentence.clone(), cx);
                        }
                        on_done(editor, answer.is_ok(), cx);
                    });
                },
                cx,
            );
        });
    }

    /// On Enter or leaving the field: a blank or unchanged text keeps the item as it is.
    fn rename(&mut self, item_id: u32, window: &mut Window, cx: &mut Context<Self>) {
        let Some(field) = self.items.iter().find(|field| field.item_id == item_id) else {
            return;
        };
        let typed = field.input.read(cx).value().to_string();
        let saved = field.saved_text.clone();
        let input = field.input.clone();
        match checklist_item_text(&typed) {
            Some(text) if text != saved => {
                let window_handle = window.window_handle();
                self.change(
                    ChecklistItemChange::Rename { item_id, text },
                    cx,
                    move |_, is_renamed, cx| {
                        if !is_renamed {
                            let _ = window_handle.update(cx, |_, window, cx| {
                                input.update(cx, |input, cx| input.set_value(saved, window, cx));
                            });
                        }
                    },
                );
            }
            _ if typed != saved => {
                input.update(cx, |input, cx| input.set_value(saved, window, cx));
            }
            _ => {}
        }
    }

    fn remove(&mut self, item_id: u32, window: &mut Window, cx: &mut Context<Self>) {
        let window_handle = window.window_handle();
        self.change(
            ChecklistItemChange::Remove { item_id },
            cx,
            move |editor, is_removed, cx| {
                // Its Remove button went with the row: focus goes to the new-item field.
                if is_removed {
                    let focus = editor.new_item.read(cx).focus_handle(cx);
                    let _ = window_handle.update(cx, |_, window, cx| window.focus(&focus, cx));
                }
            },
        );
    }

    fn add(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.items.len() >= MAXIMUM_CHECKLIST_ITEMS {
            return;
        }
        let Some(text) = checklist_item_text(&self.new_item.read(cx).value()) else {
            return;
        };
        let window_handle = window.window_handle();
        let new_item = self.new_item.clone();
        self.change(
            ChecklistItemChange::Add(text),
            cx,
            move |_, is_added, cx| {
                if is_added {
                    let _ = window_handle.update(cx, |_, window, cx| {
                        new_item.update(cx, |input, cx| input.set_value("", window, cx));
                    });
                }
            },
        );
    }
}

impl Render for ChecklistEditor {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(checklist) = self.checklist(cx) else {
            return div().into_any_element();
        };
        self.follow_items(&checklist, window, cx);
        let tokens = theme(cx);
        let words = strings(cx).checklists();
        let can_remove = self.items.len() > 1;
        let is_full = self.items.len() >= MAXIMUM_CHECKLIST_ITEMS;
        let entity = cx.entity();
        let rows = self.items.iter().map(|field| {
            let item_id = field.item_id;
            let entity = entity.clone();
            div()
                .flex()
                .items_center()
                .gap(length(SPACE_2XS))
                .child(
                    div().flex_1().min_w_0().child(
                        TextField::new(
                            ("checklist-edit-item", item_id),
                            words.rename(&field.saved_text),
                            &field.input,
                        )
                        .label_hidden(),
                    ),
                )
                .when(can_remove, |row| {
                    row.child(
                        IconButton::new(
                            ("checklist-edit-remove", item_id),
                            IconName::Trash,
                            words.remove(&field.saved_text),
                        )
                        .sizes(SIZE_HIT_WEB_MIN, COMPONENT_POLL_MARK_GLYPH_SIZE)
                        .tone(IconButtonTone::Danger)
                        .on_click(move |window, cx| {
                            entity.update(cx, |editor, cx| editor.remove(item_id, window, cx));
                        }),
                    )
                })
        });
        let left =
            i64::try_from(MAXIMUM_CHECKLIST_ITEMS.saturating_sub(self.items.len())).unwrap_or(0);
        let add_entity = cx.entity();
        div()
            .flex()
            .flex_col()
            .gap(length(COMPONENT_POLL_ANSWER_GAP))
            .children(rows)
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(length(SPACE_2XS))
                    .child(
                        div().flex_1().min_w_0().child(
                            TextField::new("checklist-edit-new", words.new_item(), &self.new_item)
                                .label_hidden()
                                .read_only(is_full),
                        ),
                    )
                    .child(
                        Button::new("checklist-edit-add", words.add_item())
                            .variant(ButtonVariant::Ghost)
                            .disabled(is_full)
                            .on_click(move |window, cx| {
                                add_entity.update(cx, |editor, cx| editor.add(window, cx));
                            }),
                    ),
            )
            .child(
                with_type(div(), TYPE_BODY_SM)
                    .text_color(color(tokens.component_poll_footer_text))
                    .child(if can_remove {
                        words.items_left(left)
                    } else {
                        words.last_item_stays().to_owned()
                    }),
            )
            .into_any_element()
    }
}

impl std::fmt::Debug for ChecklistEditor {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ChecklistEditor")
            .field("message_id", &self.message_id)
            .finish_non_exhaustive()
    }
}
