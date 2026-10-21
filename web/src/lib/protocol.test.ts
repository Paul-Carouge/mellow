import { describe, expect, it } from "vitest";

import { generateMessageNonce } from "./api";
import { Permission, compareSnowflakes, hasPermission } from "./protocol";

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
