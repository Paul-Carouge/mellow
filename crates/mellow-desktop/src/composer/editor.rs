//! The message editor's bookkeeping (`message-editor.svelte.ts`, DESIGN.md §5.5): one message at a
//! time, and a refused save gives the typed text back instead of losing it. The text itself lives
//! in the composer's field.

use std::collections::HashMap;

use mellow_protocol::Snowflake;

#[derive(Debug, Default)]
pub struct MessageEditor {
    message_id: Option<Snowflake>,
    /// The instance would refuse it: say why at the first try, until the editor closes.
    has_tried_saving_too_long: bool,
    // Refused saves of messages whose editor could not reopen, until the message is edited again.
    refused_text_by_message_id: HashMap<Snowflake, String>,
}

impl MessageEditor {
    #[must_use]
    pub fn message_id(&self) -> Option<Snowflake> {
        self.message_id
    }

    #[must_use]
    pub fn has_tried_saving_too_long(&self) -> bool {
        self.has_tried_saving_too_long
    }

    /// Opens on `message_id` and returns the text to edit: a refused save of it, or `content`.
    pub fn open(&mut self, message_id: Snowflake, content: &str) -> String {
        self.message_id = Some(message_id);
        self.has_tried_saving_too_long = false;
        self.refused_text_by_message_id
            .remove(&message_id)
            .unwrap_or_else(|| content.to_owned())
    }

    pub fn close(&mut self) {
        self.message_id = None;
        self.has_tried_saving_too_long = false;
    }

    /// The edited message and `text` to save, closing the editor; `None`, with the editor left
    /// open, when the text is too long.
    pub fn take_text_to_save(
        &mut self,
        text: String,
        is_too_long: bool,
    ) -> Option<(Snowflake, String)> {
        let message_id = self.message_id?;
        if is_too_long {
            self.has_tried_saving_too_long = true;
            return None;
        }
        self.close();
        Some((message_id, text))
    }

    /// A refused save: the editor reopens on `text` when it is free and `can_reopen` (the message
    /// is still in the open chat), returning the text to show; otherwise the text waits for the
    /// message's next edit.
    pub fn give_back(
        &mut self,
        message_id: Snowflake,
        text: String,
        can_reopen: bool,
    ) -> Option<String> {
        if can_reopen && self.message_id.is_none() {
            self.message_id = Some(message_id);
            self.has_tried_saving_too_long = false;
            return Some(text);
        }
        if self.message_id != Some(message_id) {
            self.refused_text_by_message_id.insert(message_id, text);
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(raw: u64) -> Snowflake {
        Snowflake::from_raw(raw)
    }

    #[test]
    fn opens_on_the_messages_text_and_hands_the_typed_text_over_when_saved() {
        let mut editor = MessageEditor::default();
        assert_eq!(editor.open(id(1), "helo"), "helo");
        assert_eq!(editor.message_id(), Some(id(1)));

        assert_eq!(
            editor.take_text_to_save("hello".into(), false),
            Some((id(1), "hello".into()))
        );
        assert_eq!(editor.message_id(), None);
    }

    #[test]
    fn reopens_with_the_typed_text_when_the_instance_refuses_the_edit() {
        let mut editor = MessageEditor::default();
        editor.open(id(1), "helo");
        let (message_id, text) = editor
            .take_text_to_save("hello, fixed".into(), false)
            .expect("saved");

        assert_eq!(
            editor.give_back(message_id, text, true),
            Some("hello, fixed".into())
        );
        assert_eq!(editor.message_id(), Some(id(1)));
    }

    #[test]
    fn keeps_a_refused_text_for_the_messages_next_edit_when_busy_or_the_chat_moved_on() {
        let mut editor = MessageEditor::default();
        editor.open(id(1), "one");
        let (_, text) = editor
            .take_text_to_save("one, edited".into(), false)
            .expect("saved");
        editor.open(id(2), "two");

        assert_eq!(editor.give_back(id(1), text, true), None);
        assert_eq!(editor.message_id(), Some(id(2)));

        editor.close();
        assert_eq!(editor.give_back(id(2), "two, edited".into(), false), None);
        assert_eq!(editor.message_id(), None);

        assert_eq!(editor.open(id(1), "one"), "one, edited");
        editor.close();
        assert_eq!(editor.open(id(1), "one"), "one");
        assert_eq!(editor.open(id(2), "two"), "two, edited");
    }

    #[test]
    fn refuses_to_save_a_text_that_is_too_long_staying_open_until_it_closes() {
        let mut editor = MessageEditor::default();
        editor.open(id(1), "");
        assert!(!editor.has_tried_saving_too_long());

        assert_eq!(editor.take_text_to_save("long".into(), true), None);
        assert_eq!(editor.message_id(), Some(id(1)));
        assert!(editor.has_tried_saving_too_long());

        assert_eq!(
            editor.take_text_to_save("short".into(), false),
            Some((id(1), "short".into()))
        );
        editor.open(id(1), "");
        assert!(!editor.has_tried_saving_too_long());
    }
}
