import { describe, expect, it } from "vitest";

import { generateMessageNonce } from "./api";
import {
  MAXIMUM_PROFILE_IMAGE_BYTES,
  Permission,
  compareSnowflakes,
  computeChannelPermissions,
  hasPermission,
  profileImageFileProblem,
  profileImageUrl,
} from "./protocol";

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

describe("profile images", () => {
  const still = { id: "7", is_animated: false };
  const animated = { id: "8", is_animated: true };

  it("serve a still picture from one URL, and an animated one still unless animation is wanted", () => {
    expect(profileImageUrl(still, false)).toBe("/api/v1/profile-images/7");
    expect(profileImageUrl(still, true)).toBe("/api/v1/profile-images/7");
    expect(profileImageUrl(animated, false)).toBe("/api/v1/profile-images/8/still");
    expect(profileImageUrl(animated, true)).toBe("/api/v1/profile-images/8");
  });

  it("refuse files over 8 MB or of another type before sending them", () => {
    expect(profileImageFileProblem({ size: MAXIMUM_PROFILE_IMAGE_BYTES, type: "image/gif" })).toBeNull();
    expect(profileImageFileProblem({ size: MAXIMUM_PROFILE_IMAGE_BYTES + 1, type: "image/png" })).toBe("too-large");
    expect(profileImageFileProblem({ size: 10, type: "image/svg+xml" })).toBe("not-a-picture");
    expect(profileImageFileProblem({ size: 10, type: "" })).toBeNull();
  });
});
