import { render } from "svelte/server";
import { describe, expect, it } from "vitest";

import { strings } from "../i18n";
import { MAXIMUM_MESSAGE_LENGTH, type Message, type PublicUser } from "../lib/protocol";
import { MessageEditor } from "./message-editor.svelte";
import MessageItem from "./MessageItem.svelte";

const AUTHOR: PublicUser = { id: "2", username: "ada", display_name: "Ada" };
const MESSAGE: Message = {
  id: "4194304000",
  channel_id: "10",
  author: AUTHOR,
  content: "helo",
  reply_to_id: null,
  edited_at_unix_milliseconds: null,
};

const ignore = () => {};

/** The row of `MESSAGE` with the chat's editor open on it. */
function renderedEditingRow(editor: MessageEditor): string {
  return render(MessageItem, {
    props: {
      row: { kind: "sent", key: MESSAGE.id, message: MESSAGE, sentAt: 0, startsAuthorGroup: true, isFirstUnread: false },
      currentUser: AUTHOR,
      replyReference: null,
      authorColor: null,
      mentionablePeople: new Map(),
      customEmojis: new Map(),
      roles: new Map(),
      mentionReader: { userId: AUTHOR.id, guildId: null, roleIds: [] },
      doesServerFindMentions: true,
      areGifsShown: false,
      canSendMessages: true,
      canManageMessages: false,
      reactionAndPinActions: {
        canJoinReactions: false,
        canAddReactions: false,
        canRemoveReactions: false,
        canManagePins: false,
        heldMessageId: null,
        toggleReaction: ignore,
        openPicker: ignore,
        loadReactors: () => Promise.resolve([]),
        askToPin: ignore,
        viewReactions: ignore,
        askToRemoveAllReactions: ignore,
        canVoteInPolls: false,
        choosePollAnswer: ignore,
        removeOwnPollVotes: ignore,
        viewPollVoters: ignore,
        askToEndPoll: ignore,
      },
      isEditing: editor.messageId === MESSAGE.id,
      messageEditor: editor,
      isRovingTarget: false,
      isMenuOpen: false,
      onopenmenu: ignore,
      onopenauthormenu: () => false,
      maximumEditorHeight: () => 400,
      onreply: ignore,
      onforward: ignore,
      forwardOriginPlaceOf: () => null,
      onopenforwardorigin: ignore,
      onstartedit: ignore,
      onsaveedit: ignore,
      oncanceledit: ignore,
      ondelete: ignore,
      oncopy: ignore,
      onretry: ignore,
      ondiscard: ignore,
      onfocusrow: ignore,
    },
  }).body;
}

describe("MessageItem's editor", () => {
  it("is open again with the typed text after the instance refused the edit", () => {
    const editor = new MessageEditor((text) => text);
    editor.open(MESSAGE);
    editor.text = "hello, fixed";
    const typed = editor.takeTextToSave() ?? "";
    expect(renderedEditingRow(editor)).not.toContain("<textarea");

    editor.giveBack(MESSAGE, typed, true);

    const html = renderedEditingRow(editor);
    expect(html).toMatch(/<textarea[^>]*>hello, fixed<\/textarea>/);
  });

  it("shows the characters left near the limit and why a too long text is not saved", () => {
    const editor = new MessageEditor((text) => text);
    editor.open(MESSAGE);
    editor.text = "a".repeat(MAXIMUM_MESSAGE_LENGTH - 10);
    expect(renderedEditingRow(editor)).toContain(strings.chat.messageCharactersLeft(10));

    editor.text = "a".repeat(MAXIMUM_MESSAGE_LENGTH + 1);
    expect(editor.takeTextToSave()).toBeNull();

    const html = renderedEditingRow(editor);
    expect(html).toContain(strings.chat.messageCharactersLeft(-1));
    expect(html).toContain(strings.chat.messageTooLong(MAXIMUM_MESSAGE_LENGTH));
  });
});
