import { describe, expect, it } from "vitest";

import { generateMessageNonce } from "./api";
import { Permission, compareSnowflakes, computeChannelPermissions, hasPermission } from "./protocol";

describe("snowflakes", () => {
  it("compare by value, beyond JavaScript's safe integers", () => {
    expect(compareSnowflakes("9", "10")).toBe(-1);
    expect(compareSnowflakes("18446744073709551615", "18446744073709551614")).toBe(1);
    expect(compareSnowflakes("42", "42")).toBe(0);
  });
});

describe("permissions", () => {
  it("read decimal bitfields, and administrators hold everything", () => {
    expect(hasPermission("4", Permission.SEND_MESSAGES)).toBe(true);
    expect(hasPermission("2", Permission.SEND_MESSAGES)).toBe(false);
    expect(hasPermission("1", Permission.MANAGE_CHANNELS)).toBe(true);
  });
});

describe("message nonces", () => {
  it("are 32 lowercase hex characters, unique per call", () => {
    const nonce = generateMessageNonce();
    expect(nonce).toMatch(/^[0-9a-f]{32}$/);
    expect(generateMessageNonce()).not.toBe(nonce);
  });
});

describe("channel permissions", () => {
  const SEND = String(Permission.SEND_MESSAGES);
  const VIEW_AND_SEND = String(Permission.VIEW_CHANNEL | Permission.SEND_MESSAGES);

  it("apply @everyone, then role denies and allows together, then the member, like the server", () => {
    const overwrites = [
      { id: "1", kind: "role" as const, allow: "0", deny: SEND },
      { id: "5", kind: "role" as const, allow: SEND, deny: "0" },
      { id: "6", kind: "role" as const, allow: "0", deny: SEND },
      { id: "9", kind: "member" as const, allow: "0", deny: SEND },
    ];
    // @everyone denies, one role allows, another denies: allows win among roles.
    expect(computeChannelPermissions(VIEW_AND_SEND, "1", "8", ["5", "6"], overwrites)).toBe(VIEW_AND_SEND);
    expect(computeChannelPermissions(VIEW_AND_SEND, "1", "8", [], overwrites)).toBe(
      String(Permission.VIEW_CHANNEL),
    );
    // The member's own overwrite comes last.
    expect(computeChannelPermissions(VIEW_AND_SEND, "1", "9", ["5"], overwrites)).toBe(
      String(Permission.VIEW_CHANNEL),
    );
  });

  it("are left alone for administrators", () => {
    const deny = [{ id: "1", kind: "role" as const, allow: "0", deny: SEND }];
    expect(computeChannelPermissions("1", "1", "8", [], deny)).toBe("1");
  });
});
