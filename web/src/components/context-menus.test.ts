import { describe, expect, it } from "vitest";

import {
  type MenuAction,
  type MessageMenuFacts,
  type PersonMenuFacts,
  channelMenuActions,
  groupMenuActions,
  menuItemLook,
  messageMenuActions,
  personMenuActions,
  serverMenuActions,
} from "./context-menus";

describe("messageMenuActions", () => {
  const reader: MessageMenuFacts = {
    isOwn: false,
    hasText: true,
    hasReactions: false,
    isPinned: false,
    canSendMessages: true,
    canManageMessages: false,
    canAddReactions: true,
    canJoinReactions: true,
    canRemoveReactions: false,
    canManagePins: false,
  };

  it("offers a reader of someone else's message what the bar has, without Edit or Delete", () => {
    expect(messageMenuActions(reader)).toEqual(["add-reaction", "separator", "reply", "copy-text"]);
  });

  it("adds Edit and Delete, last and separated, on your own message", () => {
    expect(messageMenuActions({ ...reader, isOwn: true })).toEqual([
      "add-reaction",
      "separator",
      "reply",
      "edit",
      "copy-text",
      "separator",
      "delete",
    ]);
  });

  it("gives a moderator pins, reactions and deleting anyone's message", () => {
    expect(
      messageMenuActions({ ...reader, canManageMessages: true, canManagePins: true, canRemoveReactions: true, hasReactions: true }),
    ).toEqual([
      "add-reaction",
      "separator",
      "reply",
      "copy-text",
      "pin",
      "view-reactions",
      "separator",
      "remove-all-reactions",
      "delete",
    ]);
  });

  it("offers Unpin on a pinned message", () => {
    expect(messageMenuActions({ ...reader, canManagePins: true, isPinned: true })).toContain("unpin");
  });

  it("leaves out Reply and Edit in a read-only channel, and Add reaction without the permission", () => {
    const readOnly = { ...reader, isOwn: true, canSendMessages: false, canAddReactions: false };
    expect(messageMenuActions(readOnly)).toEqual(["copy-text", "separator", "delete"]);
  });

  it("has nothing to copy on a message of files only", () => {
    expect(messageMenuActions({ ...reader, hasText: false })).not.toContain("copy-text");
  });
});

describe("personMenuActions", () => {
  const stranger: PersonMenuFacts = {
    isSelf: false,
    relationship: null,
    canShowProfile: true,
    canOpenConversation: true,
    conversationChannel: false,
  };

  it("offers someone you don't know a message, a request, their ID and Block", () => {
    expect(personMenuActions(stranger)).toEqual([
      "profile",
      "message",
      "separator",
      "add-friend",
      "separator",
      "copy-user-id",
      "separator",
      "block",
    ]);
  });

  it("answers a request received, or cancels one sent", () => {
    expect(personMenuActions({ ...stranger, relationship: "incoming_request" })).toEqual(
      expect.arrayContaining(["accept-request", "ignore-request"]),
    );
    const sent = personMenuActions({ ...stranger, relationship: "outgoing_request" });
    expect(sent).toContain("cancel-request");
    expect(sent).not.toContain("add-friend");
  });

  it("removes a friend, in the danger section", () => {
    expect(personMenuActions({ ...stranger, relationship: "friend" }).slice(-3)).toEqual([
      "separator",
      "remove-friend",
      "block",
    ]);
  });

  it("only unblocks someone you blocked: no message, no Block again", () => {
    expect(personMenuActions({ ...stranger, relationship: "blocked" })).toEqual([
      "profile",
      "separator",
      "unblock",
      "separator",
      "copy-user-id",
    ]);
  });

  it("shows yourself only your profile and ID", () => {
    expect(personMenuActions({ ...stranger, isSelf: true })).toEqual(["profile", "separator", "copy-user-id"]);
  });

  it("copies the conversation's ID too from its row, where Message is pointless", () => {
    const row = personMenuActions({ ...stranger, relationship: "friend", canShowProfile: false, canOpenConversation: false, conversationChannel: true });
    expect(row).toEqual(["copy-user-id", "copy-channel-id", "separator", "remove-friend", "block"]);
  });
});

describe("channelMenuActions", () => {
  it("offers Edit channel only with a settings page to show", () => {
    expect(channelMenuActions({ canEditChannel: true })).toEqual(["edit-channel", "separator", "copy-channel-id"]);
    expect(channelMenuActions({ canEditChannel: false })).toEqual(["copy-channel-id"]);
  });
});

describe("serverMenuActions", () => {
  const everything = { canInvitePeople: true, canOpenSettings: true, canCreateChannels: true, canLeave: true };

  it("creates channels and categories from the header's menu", () => {
    expect(serverMenuActions(everything, "header")).toEqual([
      "invite-people",
      "server-settings",
      "create-channel",
      "create-category",
      "separator",
      "leave-server",
    ]);
  });

  it("copies the ID from the strip, and keeps Leave last", () => {
    expect(serverMenuActions(everything, "strip")).toEqual([
      "invite-people",
      "server-settings",
      "separator",
      "copy-server-id",
      "separator",
      "leave-server",
    ]);
  });

  it("gives a plain member of a server they own nothing but the ID", () => {
    const owner = { canInvitePeople: false, canOpenSettings: false, canCreateChannels: false, canLeave: false };
    expect(serverMenuActions(owner, "strip")).toEqual(["copy-server-id"]);
    expect(serverMenuActions(owner, "header")).toEqual([]);
  });
});

describe("groupMenuActions", () => {
  it("renames, copies the ID, and leaves last", () => {
    expect(groupMenuActions()).toEqual(["rename-group", "separator", "copy-channel-id", "separator", "leave-group"]);
  });
});

describe("menuItemLook", () => {
  it("marks destructive actions as danger", () => {
    const dangerous: MenuAction[] = ["delete", "remove-all-reactions", "remove-friend", "block", "leave-server", "leave-group"];
    for (const action of dangerous) {
      expect(menuItemLook(action).tone).toBe("danger");
    }
    expect(menuItemLook("invite-people").tone).toBe("accent");
    expect(menuItemLook("reply").tone).toBe("normal");
  });
});
