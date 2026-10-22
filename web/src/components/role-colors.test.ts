import { describe, expect, it } from "vitest";

import type { Guild, Role } from "../lib/protocol";
import { contrastRatio, highestColouredRole, readableRoleColor } from "./role-colors";

const DARK_SHEET = "#221B23";
const WHITE_SHEET = "#FFFFFF";

function rgbOf(hex: string): [number, number, number] {
  const value = Number.parseInt(hex.slice(1), 16);
  return [((value >> 16) & 0xff) / 255, ((value >> 8) & 0xff) / 255, (value & 0xff) / 255];
}

function role(id: string, position: number, color: number): Role {
  return { id, name: id, color, position, permissions: "0" };
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
