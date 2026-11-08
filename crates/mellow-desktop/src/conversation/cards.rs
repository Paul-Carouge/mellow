//! Polls, checklists and progress bars in the conversation (ChatPane.svelte's part of ADR-0043 and
//! ADR-0047): who may vote, tick, create and edit them, the cards each row draws, the one poll
//! clock, and what the cards' buttons and the composer's "/" commands ask of the session.

use std::collections::HashSet;
use std::hash::{DefaultHasher, Hash as _, Hasher as _};
use std::time::Duration;

use gpui::{AnyElement, AppContext as _, Context, Entity, IntoElement as _, Task, Window};
use mellow_client_core::shared_logic::format::display_name;
use mellow_client_core::shared_logic::poll_view::pending_poll_preview;
use mellow_client_core::shared_logic::tracker_form::{
    pending_checklist_preview, pending_progress_bar_preview,
};
use mellow_client_core::state::{MessageCard, is_poll_ended};
use mellow_protocol::{Message, NewProgressBar, Permissions, Snowflake};

use super::card_dialogs::{CardDialogKind, open_card_dialog};
use super::checklist_editor::ChecklistEditor;
use super::poll_voters_dialog::PollVotersBody;
use super::{ConversationView, rows::current_user_as_author};
use crate::components::{ModalDialog, ModalDialogEvent};
use crate::i18n::strings;
use crate::messages::{
    ChecklistCard, PollCard, ProgressBarCard, full_timestamp_label, local_date_time,
};
use crate::session::now_unix_milliseconds;

/// The poll clock moves once a minute, as the web's shared clock (`poll-clock.svelte.ts`).
const POLL_CLOCK_TICK: Duration = Duration::from_secs(60);

/// The view's state for cards: what changes a card's height lives here, so rows remeasure.
#[derive(Default)]
pub(super) struct CardsState {
    /// Polls whose results the reader asked to see before voting.
    revealed_poll_ids: HashSet<Snowflake>,
    /// The checklist being edited, one at a time.
    checklist_editor: Option<(Snowflake, Entity<ChecklistEditor>)>,
    /// The time open polls read, as of the clock's last tick.
    clock_now: u64,
    /// An open poll was drawn since the last tick: the clock keeps going.
    is_open_poll_drawn: bool,
    is_clock_running: bool,
    clock: Option<Task<()>>,
}

/// What the reader may do with cards in a channel (ChatPane.svelte's derived values).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(clippy::struct_excessive_bools)] // One flag per derived right, as the web's.
pub(super) struct CardRights {
    pub(super) can_create_polls: bool,
    pub(super) can_create_checklists: bool,
    /// Whoever reads a server channel, or writes in a conversation (ADR-0043).
    pub(super) can_vote: bool,
    pub(super) can_tick: bool,
    can_send: bool,
    can_read_history: bool,
    can_manage_messages: bool,
}

impl CardRights {
    /// Its author while they may write here, or a server channel's moderator.
    fn can_edit_tracker(self, is_own: bool) -> bool {
        (is_own && self.can_send && self.can_read_history)
            || (self.can_manage_messages && self.can_read_history)
    }
}

/// A hash of what a message's card shows that may change its height.
pub(super) fn card_version(message: &Message, cards: &CardsState) -> u64 {
    let mut hasher = DefaultHasher::new();
    if let Some(poll) = &message.poll {
        for answer in &poll.answers {
            (answer.vote_count, answer.me_voted).hash(&mut hasher);
        }
        is_poll_ended(poll, cards.clock_now).hash(&mut hasher);
        cards
            .revealed_poll_ids
            .contains(&message.id)
            .hash(&mut hasher);
    }
    if let Some(checklist) = &message.checklist {
        checklist.revision.hash(&mut hasher);
        for item in &checklist.items {
            item.checked.is_some().hash(&mut hasher);
        }
        cards
            .checklist_editor
            .as_ref()
            .is_some_and(|(editing_id, _)| *editing_id == message.id)
            .hash(&mut hasher);
    }
    if let Some(bar) = &message.progress_bar {
        bar.revision.hash(&mut hasher);
    }
    hasher.finish()
}

impl ConversationView {
    pub(super) fn card_rights(&self, channel_id: Snowflake, cx: &gpui::App) -> CardRights {
        let rights = self.permissions(channel_id, cx);
        let state = self.session.read(cx).state();
        let permissions = state.my_channel_permissions(channel_id);
        let has =
            |permission: Permissions| permissions.is_none_or(|bits| bits.contains(permission));
        let is_conversation = state.direct_message_channel(channel_id).is_some();
        let can_read_history = rights.can_join_reactions;
        CardRights {
            can_create_polls: rights.can_send && has(Permissions::SEND_POLLS),
            can_create_checklists: rights.can_send && has(Permissions::SEND_CHECKLISTS),
            can_vote: if is_conversation {
                rights.can_send
            } else {
                can_read_history
            },
            can_tick: rights.can_send && can_read_history,
            can_send: rights.can_send,
            can_read_history,
            can_manage_messages: rights.can_manage_messages,
        }
    }

    // ---- The poll clock ----

    /// The time the open polls on screen read; starts the clock if it stopped.
    fn poll_clock_now(&mut self) -> u64 {
        if !self.cards.is_clock_running {
            self.cards.clock_now = now_unix_milliseconds();
        }
        self.cards.clock_now
    }

    /// An open poll is on screen: the clock ticks once a minute until none is.
    fn watch_poll_clock(&mut self, cx: &mut Context<Self>) {
        self.cards.is_open_poll_drawn = true;
        if self.cards.is_clock_running {
            return;
        }
        self.cards.is_clock_running = true;
        self.cards.clock = Some(cx.spawn(async move |view, cx| {
            loop {
                cx.background_executor().timer(POLL_CLOCK_TICK).await;
                let is_running = view
                    .update(cx, ConversationView::tick_poll_clock)
                    .unwrap_or(false);
                if !is_running {
                    break;
                }
            }
        }));
    }

    /// False once no open poll was drawn since the last tick: the clock stops.
    fn tick_poll_clock(&mut self, cx: &mut Context<Self>) -> bool {
        if !std::mem::take(&mut self.cards.is_open_poll_drawn) {
            self.cards.is_clock_running = false;
            return false;
        }
        self.cards.clock_now = now_unix_milliseconds();
        self.are_rows_stale = true;
        cx.notify();
        true
    }

    /// Whether a checklist is being edited, for tests.
    #[cfg(test)]
    pub(super) fn is_editing_checklist(&self) -> bool {
        self.cards.checklist_editor.is_some()
    }

    /// Whether the poll clock ticks, for tests.
    #[cfg(test)]
    pub(super) fn is_poll_clock_running(&self) -> bool {
        self.cards.is_clock_running
    }

    // ---- Cards in rows ----

    fn name_of_user(&self, user_id: Snowflake, cx: &gpui::App) -> String {
        let state = self.session.read(cx).state();
        let open = self.open_conversation.as_ref();
        let member = open.and_then(|open| open.guild_id).and_then(|guild_id| {
            state
                .members(guild_id)
                .iter()
                .find(|member| member.user.id == user_id)
                .map(|member| display_name(&member.user).to_owned())
        });
        let recipient = || {
            open.and_then(|open| state.direct_message_channel(open.channel_id))
                .and_then(|conversation| {
                    conversation
                        .recipients
                        .iter()
                        .find(|recipient| recipient.id == user_id)
                        .map(|recipient| display_name(recipient).to_owned())
                })
        };
        let reader = || {
            current_user_as_author(self.session.read(cx))
                .filter(|reader| reader.id == user_id)
                .map(|reader| display_name(&reader).to_owned())
        };
        member
            .or_else(recipient)
            .or_else(reader)
            .unwrap_or_else(|| strings(cx).common().someone().to_owned())
    }

    /// The card of a sent message, with its buttons wired to the view.
    #[allow(clippy::too_many_lines)] // One card per kind, each wired to the view.
    pub(super) fn card_for_message(
        &mut self,
        message: &Message,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let message_id = message.id;
        let channel_id = message.channel_id;
        let rights = self.card_rights(channel_id, cx);
        let view = cx.entity().downgrade();
        let id = gpui::SharedString::from(format!("card-{message_id}"));
        if let Some(poll) = &message.poll {
            let now = self.poll_clock_now();
            if !is_poll_ended(poll, now) {
                self.watch_poll_clock(cx);
            }
            let (choose, remove, results, voters) =
                (view.clone(), view.clone(), view.clone(), view);
            return Some(
                PollCard::new(id, poll.clone(), now)
                    .can_vote(rights.can_vote)
                    .results_revealed(self.cards.revealed_poll_ids.contains(&message_id))
                    .on_choose(move |answer_id, _, cx| {
                        let _ = choose.update(cx, |view, cx| {
                            view.change_own_poll_votes(message_id, Some(answer_id), cx);
                        });
                    })
                    .on_remove_vote(move |_, cx| {
                        let _ = remove.update(cx, |view, cx| {
                            view.change_own_poll_votes(message_id, None, cx);
                        });
                    })
                    .on_toggle_results(move |_, cx| {
                        let _ = results.update(cx, |view, cx| {
                            if !view.cards.revealed_poll_ids.remove(&message_id) {
                                view.cards.revealed_poll_ids.insert(message_id);
                            }
                            view.are_rows_stale = true;
                            cx.notify();
                        });
                    })
                    .on_view_voters(move |answer_id, window, cx| {
                        let _ = voters.update(cx, |view, cx| {
                            view.open_poll_voters(message_id, answer_id, window, cx);
                        });
                    })
                    .into_any_element(),
            );
        }
        let is_own = self
            .session
            .read(cx)
            .state()
            .current_user()
            .is_some_and(|user| user.id == message.author.id);
        let can_edit = rights.can_edit_tracker(is_own);
        if let Some(checklist) = &message.checklist {
            let catalogue = strings(cx);
            let ticked_by = checklist
                .items
                .iter()
                .map(|item| {
                    item.checked.map(|tick| {
                        let when = full_timestamp_label(
                            local_date_time(tick.at_unix_milliseconds),
                            catalogue,
                        );
                        catalogue
                            .checklists()
                            .ticked_by(&self.name_of_user(tick.user_id, cx), &when)
                    })
                })
                .collect();
            let editor = self
                .cards
                .checklist_editor
                .as_ref()
                .filter(|(editing_id, _)| *editing_id == message_id)
                .map(|(_, editor)| editor.clone().into());
            let (toggle, edit) = (view.clone(), view);
            return Some(
                ChecklistCard::new(id, checklist.clone(), ticked_by)
                    .can_tick(rights.can_tick)
                    .can_edit(can_edit)
                    .editor(editor)
                    .on_toggle(move |item_id, _, cx| {
                        let _ = toggle.update(cx, |view, cx| {
                            view.toggle_checklist_item(message_id, item_id, cx);
                        });
                    })
                    .on_edit(move |window, cx| {
                        let _ = edit.update(cx, |view, cx| {
                            view.toggle_checklist_editing(message_id, window, cx);
                        });
                    })
                    .into_any_element(),
            );
        }
        let bar = message.progress_bar.as_ref()?;
        let card = ProgressBarCard::new(id, bar.clone());
        let card = if can_edit {
            card.on_update(move |window, cx| {
                let _ = view.update(cx, |view, cx| {
                    view.open_card_dialog(
                        CardDialogKind::ProgressBar(Some(message_id)),
                        window,
                        cx,
                    );
                });
            })
        } else {
            card
        };
        Some(card.into_any_element())
    }

    /// The card of a message still in the outbox: shown as it will arrive, nothing to press.
    pub(super) fn card_for_pending(&mut self, nonce: &str, card: &MessageCard) -> AnyElement {
        let id = gpui::SharedString::from(format!("card-{nonce}"));
        match card {
            MessageCard::Poll(poll) => {
                let now = self.poll_clock_now();
                PollCard::new(id, pending_poll_preview(poll, now), now)
                    .pending(true)
                    .into_any_element()
            }
            MessageCard::Checklist(checklist) => {
                ChecklistCard::new(id, pending_checklist_preview(checklist), Vec::new())
                    .pending(true)
                    .into_any_element()
            }
            MessageCard::ProgressBar(bar) => {
                ProgressBarCard::new(id, pending_progress_bar_preview(bar))
                    .pending(true)
                    .into_any_element()
            }
        }
    }

    // ---- What the cards ask ----

    fn change_own_poll_votes(
        &mut self,
        message_id: Snowflake,
        answer_id: Option<u32>,
        cx: &mut Context<Self>,
    ) {
        let Some(channel_id) = self.open_channel_id() else {
            return;
        };
        self.action_error = None;
        let polls = strings(cx).polls();
        let summary = if answer_id.is_some() {
            polls.vote_failed()
        } else {
            polls.remove_vote_failed()
        };
        let view = cx.entity().downgrade();
        self.session.update(cx, |session, cx| {
            session.change_own_poll_votes(
                channel_id,
                message_id,
                answer_id,
                summary.to_owned(),
                polls.has_ended().to_owned(),
                move |sentence, cx| {
                    let _ = view.update(cx, |view, cx| {
                        view.show_action_error(channel_id, sentence, cx);
                    });
                },
                cx,
            );
        });
    }

    fn open_poll_voters(
        &mut self,
        message_id: Snowflake,
        answer_id: u32,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(channel_id) = self.open_channel_id() else {
            return;
        };
        let Some(answer_text) = self
            .cached_message(message_id, cx)
            .and_then(|message| message.poll)
            .and_then(|poll| {
                poll.answers
                    .into_iter()
                    .find(|answer| answer.id == answer_id)
                    .map(|answer| answer.text)
            })
        else {
            return;
        };
        let catalogue = strings(cx);
        let session = self.session.clone();
        let body = cx.new(|cx| PollVotersBody::new(session, channel_id, message_id, answer_id, cx));
        let dialog = cx.new(|cx| {
            ModalDialog::new(
                catalogue.polls().voters_title(&answer_text),
                catalogue.common().close(),
                window,
                cx,
            )
            .body(body.into())
        });
        let subscription = cx.subscribe(&dialog, |view, _, _: &ModalDialogEvent, cx| {
            view.confirmation = None;
            cx.notify();
        });
        self.confirmation = Some((dialog, subscription));
        cx.notify();
    }

    /// "End poll" in the menu: confirmed first, as on the web.
    pub(super) fn ask_to_end_poll(
        &mut self,
        message_id: Snowflake,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(channel_id) = self.open_channel_id() else {
            return;
        };
        let catalogue = strings(cx);
        let polls = catalogue.polls();
        let dialog = cx.new(|cx| {
            ModalDialog::new(polls.end_title(), catalogue.common().cancel(), window, cx)
                .subtitle(polls.end_subtitle())
                .submit(polls.end(), polls.end(), true)
        });
        let subscription = cx.subscribe_in(
            &dialog,
            window,
            move |view, dialog, event: &ModalDialogEvent, window, cx| {
                if *event == ModalDialogEvent::Submitted {
                    // Closed at once, as on the web: a failure shows above the composer.
                    dialog.update(cx, |dialog, cx| dialog.request_close(window, cx));
                    view.end_poll_now(channel_id, message_id, cx);
                }
                view.confirmation = None;
                cx.notify();
            },
        );
        self.confirmation = Some((dialog, subscription));
        cx.notify();
    }

    fn end_poll_now(
        &mut self,
        channel_id: Snowflake,
        message_id: Snowflake,
        cx: &mut Context<Self>,
    ) {
        self.action_error = None;
        let summary = strings(cx).polls().end_failed().to_owned();
        let view = cx.entity().downgrade();
        self.session.update(cx, |session, cx| {
            session.end_poll(
                channel_id,
                message_id,
                summary,
                move |sentence, cx| {
                    let _ = view.update(cx, |view, cx| {
                        view.show_action_error(channel_id, sentence, cx);
                    });
                },
                cx,
            );
        });
    }

    fn toggle_checklist_item(
        &mut self,
        message_id: Snowflake,
        item_id: u32,
        cx: &mut Context<Self>,
    ) {
        let Some(channel_id) = self.open_channel_id() else {
            return;
        };
        self.action_error = None;
        let summary = strings(cx).checklists().tick_failed().to_owned();
        let view = cx.entity().downgrade();
        self.session.update(cx, |session, cx| {
            session.toggle_checklist_item(
                channel_id,
                message_id,
                item_id,
                summary,
                move |sentence, cx| {
                    let _ = view.update(cx, |view, cx| {
                        view.show_action_error(channel_id, sentence, cx);
                    });
                },
                cx,
            );
        });
    }

    /// "Edit list" opens the fields of one checklist; "Done" or another checklist closes them.
    pub(super) fn toggle_checklist_editing(
        &mut self,
        message_id: Snowflake,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let was_editing = self
            .cards
            .checklist_editor
            .take()
            .is_some_and(|(editing_id, _)| editing_id == message_id);
        if !was_editing && let Some(channel_id) = self.open_channel_id() {
            let session = self.session.clone();
            let view = cx.entity().downgrade();
            let editor = cx
                .new(|cx| ChecklistEditor::new(session, view, channel_id, message_id, window, cx));
            editor.update(cx, |editor, cx| editor.focus_new_item(window, cx));
            self.cards.checklist_editor = Some((message_id, editor));
        }
        self.are_rows_stale = true;
        cx.notify();
    }

    /// Leaving the channel closes the checklist being edited and forgets shown results.
    pub(super) fn forget_cards_of_channel(&mut self) {
        self.cards.checklist_editor = None;
        self.cards.revealed_poll_ids.clear();
    }

    // ---- Creating them ----

    /// The composer's "/poll", "/checklist" and "/progress", or a progress bar's "Update".
    pub(super) fn open_card_dialog(
        &mut self,
        kind: CardDialogKind,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(channel_id) = self.open_channel_id() else {
            return;
        };
        let is_conversation = self
            .open_conversation
            .as_ref()
            .is_some_and(|open| open.guild_id.is_none());
        let current_bar = match kind {
            CardDialogKind::ProgressBar(Some(message_id)) => {
                match self
                    .cached_message(message_id, cx)
                    .and_then(|message| message.progress_bar)
                {
                    Some(bar) => Some(bar),
                    None => return,
                }
            }
            _ => None,
        };
        let view = cx.entity().downgrade();
        let (dialog, subscription) = open_card_dialog(
            kind,
            is_conversation,
            current_bar.as_ref(),
            window,
            cx,
            move |done, cx| {
                let _ = view.update(cx, |view, cx| view.finish_card_dialog(channel_id, done, cx));
            },
        );
        self.confirmation = Some((dialog, subscription));
        cx.notify();
    }

    fn finish_card_dialog(
        &mut self,
        channel_id: Snowflake,
        done: CardDialogDone,
        cx: &mut Context<Self>,
    ) {
        self.confirmation = None;
        match done {
            CardDialogDone::Closed => {}
            CardDialogDone::Send(card) => self.send_card(channel_id, card, cx),
            CardDialogDone::UpdateProgressBar(message_id, bar) => {
                self.update_progress_bar(channel_id, message_id, bar, cx);
            }
        }
        cx.notify();
    }

    fn send_card(&mut self, channel_id: Snowflake, card: MessageCard, cx: &mut Context<Self>) {
        if self.open_channel_id() != Some(channel_id) {
            return;
        }
        self.action_error = None;
        let answer = self
            .session
            .update(cx, |session, cx| session.send_card(channel_id, card, cx));
        if let Err(error) = answer {
            tracing::warn!(%error, "cannot queue a card");
            self.action_error = Some(strings(cx).chat().send_failed().into());
        }
        self.jump_to_present(cx);
    }

    fn update_progress_bar(
        &mut self,
        channel_id: Snowflake,
        message_id: Snowflake,
        bar: NewProgressBar,
        cx: &mut Context<Self>,
    ) {
        self.action_error = None;
        let summary = strings(cx).progress_bars().update_failed().to_owned();
        let view = cx.entity().downgrade();
        self.session.update(cx, |session, cx| {
            session.update_progress_bar(
                channel_id,
                message_id,
                bar,
                summary,
                move |sentence, cx| {
                    let _ = view.update(cx, |view, cx| {
                        view.show_action_error(channel_id, sentence, cx);
                    });
                },
                cx,
            );
        });
    }
}

/// How a creation or update dialog ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum CardDialogDone {
    Closed,
    Send(MessageCard),
    UpdateProgressBar(Snowflake, NewProgressBar),
}
