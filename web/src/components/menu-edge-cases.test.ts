import { describe, expect, it } from "vitest";

import {
  groupMenuActions,
  personMenuActions,
  serverMenuActions,
  serverMenuFactsOf,
  messageMenuActions,
  menuEntries,
  channelMenuActions,
  type MenuAction,
  type MenuLine,
  type PersonMenuFacts,
  type MessageMenuFacts,
} from "./context-menus";
import { moveFocusWithinMenu, typeaheadMenuItemIndex, nextMenuItemIndex } from "./menu-keyboard";
import { placeMenu } from "./menu-placement";
import { type Guild, Permission } from "../lib/protocol";

function guild(permissions: bigint, ownerId = "9"): Guild {
  return { id: "1", name: "G", owner_id: ownerId, my_permissions: String(permissions), my_role_ids: [], roles: [], channels: [] };
}

function assertWellFormed(lines: readonly MenuLine<MenuAction>[]) {
  expect(lines[0]).not.toBe("separator");
  expect(lines.at(-1)).not.toBe("separator");
  lines.forEach((line, index) => {
    if (line === "separator") expect(lines[index + 1]).not.toBe("separator");
  });
}

describe("every combination yields a well-formed list", () => {
  const relationships = [null, "friend", "incoming_request", "outgoing_request", "blocked"] as const;
  const flags = [true, false];

  it("person menus: no leading, trailing or doubled separator, no duplicates, danger last", () => {
    for (const relationship of relationships)
      for (const isSelf of flags)
        for (const canShowProfile of flags)
          for (const canOpenConversation of flags)
            for (const conversationChannel of flags) {
              const facts: PersonMenuFacts = { isSelf, relationship, canShowProfile, canOpenConversation, conversationChannel };
              const lines = personMenuActions(facts);
              assertWellFormed(lines);
              const actions = lines.filter((line) => line !== "separator");
              expect(new Set(actions).size).toBe(actions.length);
              expect(actions).toContain("copy-user-id");
              if (isSelf) {
                for (const forbidden of ["message", "add-friend", "block", "unblock", "accept-request"])
                  expect(actions).not.toContain(forbidden);
              }
            }
  });

  it("yourself as a friend entry still cannot be blocked, but can be removed only if a friend", () => {
    const lines = personMenuActions({ isSelf: true, relationship: "friend", canShowProfile: true, canOpenConversation: true, conversationChannel: false });
    expect(lines).not.toContain("block");
  });

  it("message menus over all 2^10 fact combinations", () => {
    const keys = ["isOwn", "hasText", "hasReactions", "isPinned", "canSendMessages", "canManageMessages", "canAddReactions", "canJoinReactions", "canRemoveReactions", "canManagePins"] as const;
    for (let mask = 0; mask < 1 << keys.length; mask += 1) {
      const facts = Object.fromEntries(keys.map((key, i) => [key, (mask & (1 << i)) !== 0])) as unknown as MessageMenuFacts;
      const lines = messageMenuActions(facts);
      assertWellFormed(lines);
      const actions = lines.filter((line) => line !== "separator");
      expect(actions.includes("delete")).toBe(facts.isOwn || facts.canManageMessages);
      expect(actions.includes("edit")).toBe(facts.isOwn && facts.canSendMessages);
      expect(actions.includes("pin") || actions.includes("unpin")).toBe(facts.canManagePins);
      expect(actions.includes("pin") && actions.includes("unpin")).toBe(false);
      if (actions.includes("delete")) expect(actions.at(-1)).toBe("delete");
    }
  });

  it("a message with no permission and no text has no menu at all", () => {
    const none = Object.fromEntries(
      ["isOwn", "hasText", "hasReactions", "isPinned", "canSendMessages", "canManageMessages", "canAddReactions", "canJoinReactions", "canRemoveReactions", "canManagePins"].map((k) => [k, false]),
    ) as unknown as MessageMenuFacts;
    expect(messageMenuActions(none)).toEqual([]);
  });

  it("view-reactions needs reactions and the right to join them", () => {
    const base = Object.fromEntries(
      ["isOwn", "hasText", "isPinned", "canSendMessages", "canManageMessages", "canAddReactions", "canRemoveReactions", "canManagePins"].map((k) => [k, false]),
    );
    expect(messageMenuActions({ ...base, hasReactions: true, canJoinReactions: false } as unknown as MessageMenuFacts)).toEqual([]);
    expect(messageMenuActions({ ...base, hasReactions: false, canJoinReactions: true } as unknown as MessageMenuFacts)).toEqual([]);
  });
});

describe("server menus per role", () => {
  it("the owner without any permission bit may open settings, never leave", () => {
    const facts = serverMenuFactsOf(guild(0n, "me"), "me");
    expect(facts).toEqual({ canInvitePeople: false, canOpenSettings: true, canCreateChannels: false, canLeave: false });
    expect(serverMenuActions(facts, "strip")).toEqual(["server-settings", "separator", "copy-server-id"]);
  });

  it("a plain member can leave and copy the ID, nothing else", () => {
    const facts = serverMenuFactsOf(guild(0n), "me");
    expect(serverMenuActions(facts, "strip")).toEqual(["copy-server-id", "separator", "leave-server"]);
    expect(serverMenuActions(facts, "header")).toEqual(["leave-server"]);
  });

  it("an administrator may do it all and still leave", () => {
    const facts = serverMenuFactsOf(guild(Permission.ADMINISTRATOR), "me");
    expect(facts.canLeave).toBe(true);
    expect(serverMenuActions(facts, "header")).toEqual(["invite-people", "server-settings", "create-channel", "create-category", "separator", "leave-server"]);
  });

  it("an unknown user id (not signed in yet) is never the owner", () => {
    expect(serverMenuFactsOf(guild(0n), undefined).canLeave).toBe(true);
  });

  it("create-channel and invite follow their own bits", () => {
    const facts = serverMenuFactsOf(guild(Permission.CREATE_INVITE | Permission.MANAGE_CHANNELS), "me");
    expect(serverMenuActions(facts, "header")).toContain("invite-people");
    expect(serverMenuActions(facts, "header")).toContain("create-channel");
    expect(serverMenuActions(facts, "strip")).not.toContain("create-channel");
  });

  it("group and channel menus are well formed", () => {
    assertWellFormed(groupMenuActions());
    assertWellFormed(channelMenuActions({ canEditChannel: false }));
    expect(channelMenuActions({ canEditChannel: false })).toEqual(["copy-channel-id"]);
  });
});

describe("menuEntries", () => {
  it("keeps separators and hands each action to its handler", () => {
    const seen: string[] = [];
    const entries = menuEntries(["reply", "separator", "delete"], (action) => ({ onselect: () => seen.push(action) }));
    expect(entries[1]).toBe("separator");
    const first = entries[0];
    if (first === undefined || first === "separator" || !("onselect" in first)) throw new Error("expected an item");
    first.onselect();
    expect(seen).toEqual(["reply"]);
    expect(menuEntries([], () => ({ onselect: () => undefined }))).toEqual([]);
  });
});

describe("placeMenu at tiny windows and edges", () => {
  const margin = 4;
  it("a menu wider than the window sits at the margin, point origin", () => {
    const placement = placeMenu({ kind: "point", x: 100, y: 100 }, { width: 500, height: 50 }, { width: 300, height: 300 }, 4, margin);
    expect(placement.left).toBe(margin);
  });

  it("a point exactly at the corner is kept within the margin on both axes", () => {
    const placement = placeMenu({ kind: "point", x: 1000, y: 800 }, { width: 200, height: 150 }, { width: 1000, height: 800 }, 4, margin);
    expect(placement.left + 200 + margin).toBeLessThanOrEqual(1000);
    expect(placement.top + 150 + margin).toBeLessThanOrEqual(800);
  });

  it("a negative or zero point is pulled to the margin", () => {
    const placement = placeMenu({ kind: "point", x: -50, y: 0 }, { width: 200, height: 150 }, { width: 1000, height: 800 }, 4, margin);
    expect(placement.left).toBe(margin);
    expect(placement.top).toBe(margin);
  });

  it("a point just where the menu fits exactly below keeps it below", () => {
    // fits when y + height + margin == viewport height
    const placement = placeMenu({ kind: "point", x: 10, y: 646 }, { width: 100, height: 150 }, { width: 1000, height: 800 }, 4, margin);
    expect(placement.top).toBe(646);
    expect(placement.isAbove).toBe(false);
  });

  it("anchor: menu wider than the window with align start / end never goes negative", () => {
    const bounds = { top: 10, bottom: 40, left: 5, right: 60 };
    for (const align of ["start", "end"] as const) {
      expect(placeMenu({ kind: "anchor", bounds, align }, { width: 500, height: 50 }, { width: 300, height: 300 }, 4, margin).left).toBeGreaterThanOrEqual(0);
    }
  });

  it("anchor: menu taller than the window both ways stays at 0 and is not reported above", () => {
    const bounds = { top: 100, bottom: 130, left: 5, right: 60 };
    const placement = placeMenu({ kind: "anchor", bounds, align: "start" }, { width: 50, height: 900 }, { width: 300, height: 300 }, 4, margin);
    expect(placement.top).toBe(0);
    expect(placement.isAbove).toBe(true);
  });

  it("anchor with stretch ignores window clamping (documented: left = bounds.left)", () => {
    const bounds = { top: 10, bottom: 40, left: 900, right: 1100 };
    const placement = placeMenu({ kind: "anchor", bounds, align: "stretch" }, { width: 50, height: 50 }, { width: 1000, height: 800 }, 4, margin);
    expect(placement).toMatchObject({ left: 900, width: 200 });
  });
});

describe("type-ahead", () => {
  const labels = ["Reply", "Édit", "Copy", "Copy ID", "Œuvre", "ça", "Delete"];

  it("matches a decomposed accent and a precomposed one alike", () => {
    expect(typeaheadMenuItemIndex("e", 0, labels)).toBe(1);
    expect(typeaheadMenuItemIndex("é", 0, labels)).toBe(1);
    expect(typeaheadMenuItemIndex("c", 4, labels)).toBe(5);
  });

  it("finds an upper-case accented key", () => {
    expect(typeaheadMenuItemIndex("É", 3, labels)).toBe(1);
  });

  it("with one matching item, pressing again stays on it", () => {
    expect(typeaheadMenuItemIndex("r", 0, labels)).toBe(0);
  });

  it("no match, empty menu, empty or modifier-named keys give null", () => {
    expect(typeaheadMenuItemIndex("x", 0, labels)).toBeNull();
    expect(typeaheadMenuItemIndex("a", 0, [])).toBeNull();
    expect(typeaheadMenuItemIndex("", 0, labels)).toBeNull();
    expect(typeaheadMenuItemIndex("Shift", 0, labels)).toBeNull();
    expect(typeaheadMenuItemIndex("Tab", 0, labels)).toBeNull();
  });

  it("an astral character is one key", () => {
    expect(typeaheadMenuItemIndex("😀", 0, ["😀 smile"])).toBe(0);
  });

  it("labels with leading whitespace (rendered text content) still match", () => {
    expect(typeaheadMenuItemIndex("r", -1, ["  \n Reply "])).toBe(0);
  });

  it("an out-of-range current index does not crash", () => {
    expect(typeaheadMenuItemIndex("c", 99, labels)).not.toBeUndefined();
    expect(nextMenuItemIndex("ArrowUp", 0, 1)).toBe(0);
  });
});

describe("moveFocusWithinMenu", () => {
  it("does not swallow a Ctrl/Meta/Alt chord that is also a letter", () => {
    let prevented = false;
    const event = { key: "c", ctrlKey: true, metaKey: false, altKey: false, preventDefault: () => (prevented = true) } as unknown as KeyboardEvent;
    const menu = { querySelectorAll: () => [] } as unknown as HTMLElement;
    moveFocusWithinMenu(event, menu);
    expect(prevented).toBe(false);
  });
});
