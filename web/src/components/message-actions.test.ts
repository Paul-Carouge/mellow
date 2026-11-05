import { describe, expect, it } from "vitest";

import { type HoverBarFacts, hoverBarActions } from "./message-actions";

const OTHERS_TEXT: HoverBarFacts = {
  isOwn: false,
  isForward: false,
  isPoll: false,
  hasText: true,
  canSendMessages: true,
  canManageMessages: false,
  isShiftHeld: false,
  hasMenu: true,
};
const OWN_TEXT: HoverBarFacts = { ...OTHERS_TEXT, isOwn: true };

describe("hoverBarActions", () => {
  it("gives your own message Edit, Forward, Copy and More, with Delete left in More", () => {
    expect(hoverBarActions(OWN_TEXT)).toEqual(["edit", "forward", "copy-text", "more"]);
  });

  it("gives others' messages Reply, Forward, Copy and More, even to a moderator", () => {
    expect(hoverBarActions({ ...OTHERS_TEXT, canManageMessages: true })).toEqual(["reply", "forward", "copy-text", "more"]);
  });

  it("adds Delete before More only while Shift is held, and only where you may delete", () => {
    expect(hoverBarActions({ ...OWN_TEXT, isShiftHeld: true })).toEqual(["edit", "forward", "copy-text", "delete", "more"]);
    expect(hoverBarActions({ ...OTHERS_TEXT, isShiftHeld: true, canManageMessages: true })).toContain("delete");
    expect(hoverBarActions({ ...OTHERS_TEXT, isShiftHeld: true })).not.toContain("delete");
  });

  it("drops Copy on a message without text (a file alone)", () => {
    expect(hoverBarActions({ ...OTHERS_TEXT, hasText: false })).toEqual(["reply", "forward", "more"]);
  });

  it("offers Reply on your own forward or poll, which cannot be edited", () => {
    expect(hoverBarActions({ ...OWN_TEXT, isForward: true })).toEqual(["reply", "forward", "copy-text", "more"]);
    expect(hoverBarActions({ ...OWN_TEXT, isPoll: true, hasText: false })).toEqual(["reply", "more"]);
  });

  it("has neither Reply nor Edit where you cannot send", () => {
    expect(hoverBarActions({ ...OWN_TEXT, canSendMessages: false })).toEqual(["forward", "copy-text", "more"]);
  });

  it("leaves More out when the menu would be empty", () => {
    expect(hoverBarActions({ ...OTHERS_TEXT, hasMenu: false })).toEqual(["reply", "forward", "copy-text"]);
  });
});
