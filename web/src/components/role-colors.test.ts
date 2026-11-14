import { describe, expect, it } from "vitest";

import type { Guild, Role } from "../lib/protocol";
import { contrastRatio, highestColouredRole, readableRoleColor, roleMentionChipColors } from "./role-colors";

const DARK_SHEET = "#221B23";
const WHITE_SHEET = "#FFFFFF";

function rgbOf(hex: string): [number, number, number] {
  const value = Number.parseInt(hex.slice(1), 16);
  return [((value >> 16) & 0xff) / 255, ((value >> 8) & 0xff) / 255, (value & 0xff) / 255];
}

function role(id: string, position: number, color: number): Role {
  return { id, name: id, color, position, permissions: "0", is_displayed_separately: false };
}

describe("readableRoleColor", () => {
  it("keeps a colour that already reads", () => {
    expect(readableRoleColor(0x4fb39a, DARK_SHEET)).toBe("#4FB39A");
  });

  it("darkens on white and lightens on the dark sheet until 4.5:1", () => {
    const onWhite = readableRoleColor(0x4fb39a, WHITE_SHEET);
    expect(contrastRatio(rgbOf(onWhite), rgbOf(WHITE_SHEET))).toBeGreaterThanOrEqual(4.5);
    const onDark = readableRoleColor(0x3a1f6b, DARK_SHEET);
    expect(contrastRatio(rgbOf(onDark), rgbOf(DARK_SHEET))).toBeGreaterThanOrEqual(4.5);
  });
});

describe("highestColouredRole", () => {
  it("picks the highest role with a colour among the member's roles", () => {
    const guild: Guild = {
      id: "g",
      name: "Club",
      owner_id: "o",
      my_permissions: "0",
      my_role_ids: [],
      roles: [role("plain", 9, 0), role("low", 1, 0x111111), role("high", 5, 0x4fb39a), role("other", 7, 0x222222)],
      channels: [],
    };
    expect(highestColouredRole(guild, ["plain", "low", "high"])?.id).toBe("high");
    expect(highestColouredRole(guild, ["plain"])).toBeNull();
  });
});

describe("role colours on names", () => {
  // The backgrounds a name sits on (DESIGN.md §2.3), in dark then light: the side sheet and a
  // hovered member row; the chat sheet and a hovered message. Colours are worked out on the hover.
  const BACKGROUNDS_BY_HOVER = [
    { hover: "#221B23", others: ["#1B151C"] },
    { hover: "#ECE6ED", others: ["#FFFFFF"] },
    { hover: "#282129", others: ["#221B23"] },
    { hover: "#FBF9FC", others: ["#FFFFFF"] },
  ];
  const COLORS = [0x010101, 0xffffff, 0xffff00, 0x0000ff, 0x3a1f6b, 0xf1c40f, 0xe74c3c, 0x3498db, 0x4fb39a, 0x99aab5];

  it("keeps 4.5:1 on the hovered row and on the plain sheet, in both themes", () => {
    for (const { hover, others } of BACKGROUNDS_BY_HOVER) {
      for (const color of COLORS) {
        const shown = rgbOf(readableRoleColor(color, hover));
        for (const background of [hover, ...others]) {
          expect(contrastRatio(shown, rgbOf(background)), `${color.toString(16)} on ${background}`).toBeGreaterThanOrEqual(4.5);
        }
      }
    }
  });
});

describe("roleMentionChipColors", () => {
  it("washes the sheet with the role colour and keeps 4.5:1 for the text, in both themes", () => {
    for (const sheet of [DARK_SHEET, WHITE_SHEET]) {
      for (const color of [0x4fb39a, 0x3a1f6b, 0xf1c40f, 0xffffff, 0x010101]) {
        const chip = roleMentionChipColors(color, sheet);
        // A role as light as the white sheet washes nothing; its text still reads.
        if (!(color === 0xffffff && sheet === WHITE_SHEET)) {
          expect(chip.background).not.toBe(sheet);
        }
        expect(contrastRatio(rgbOf(chip.text), rgbOf(chip.background))).toBeGreaterThanOrEqual(4.5);
      }
    }
  });
});
