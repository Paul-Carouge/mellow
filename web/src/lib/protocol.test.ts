import { describe, expect, it } from "vitest";

import { generateMessageNonce } from "./api";
import {
  DIRECT_MESSAGE_PERMISSIONS,
  MAXIMUM_PROFILE_IMAGE_BYTES,
  Permission,
  compareSnowflakes,
  computeChannelPermissions,
  customEmojiMarkup,
  customEmojiOfReactionKey,
  customEmojiReactionKey,
  emojiUrl,
  hasPermission,
  iconUrl,
  isValidEmojiName,
  reactionsPath,
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

describe("reactions", () => {
  it("percent-encode the emoji in the path", () => {
    expect(reactionsPath("1", "2", "👍")).toBe("channels/1/messages/2/reactions/%F0%9F%91%8D");
    expect(reactionsPath("1", "2", "#️⃣")).toBe("channels/1/messages/2/reactions/%23%EF%B8%8F%E2%83%A3");
    expect(reactionsPath("1", "2")).toBe("channels/1/messages/2/reactions");
  });

  it("use bit 16, held by direct-message participants", () => {
    expect(Permission.ADD_REACTIONS).toBe(65536n);
    expect(hasPermission(DIRECT_MESSAGE_PERMISSIONS, Permission.ADD_REACTIONS)).toBe(true);
  });
});

describe("custom emoji", () => {
  const party = { id: "500", name: "party", is_animated: false };
  const parrot = { id: "501", name: "parrot", is_animated: true };

  it("are managed with bit 17", () => {
    expect(Permission.MANAGE_EXPRESSIONS).toBe(131072n);
  });

  it("serve their picture like icons, still unless the animation is wanted", () => {
    expect(emojiUrl(party, true)).toBe("/api/v1/icons/500");
    expect(emojiUrl(parrot, false)).toBe("/api/v1/icons/501/still");
    expect(emojiUrl(parrot, true)).toBe("/api/v1/icons/501");
  });

  it("write Discord's syntax in messages and react as :id", () => {
    expect(customEmojiMarkup(party)).toBe("<:party:500>");
    expect(customEmojiMarkup(parrot)).toBe("<a:parrot:501>");
    expect(customEmojiReactionKey(parrot)).toBe(":501");
    expect(reactionsPath("1", "2", customEmojiReactionKey(parrot))).toBe("channels/1/messages/2/reactions/%3A501");
  });

  it("read back from reaction keys, Unicode emoji and malformed keys being none", () => {
    expect(customEmojiOfReactionKey(":501")).toBe("501");
    expect(customEmojiOfReactionKey(":18446744073709551615")).toBe("18446744073709551615");
    for (const key of ["👍", ":", "501", "parrot:501", ":012", "::1", ":1 ", "<:ok:1>"]) {
      expect(customEmojiOfReactionKey(key), key).toBeNull();
    }
  });

  it("are named with 2 to 32 ASCII letters, digits and underscores", () => {
    expect(isValidEmojiName("party_parrot2")).toBe(true);
    expect(isValidEmojiName("a".repeat(32))).toBe(true);
    for (const name of ["a", "a".repeat(33), "café", "a-b", "two words", ""]) {
      expect(isValidEmojiName(name), name).toBe(false);
    }
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

  it("serve server and channel icons the same way, still unless the animation is wanted", () => {
    expect(iconUrl({ id: "7", is_animated: false }, true)).toBe("/api/v1/icons/7");
    expect(iconUrl({ id: "8", is_animated: true }, false)).toBe("/api/v1/icons/8/still");
    expect(iconUrl({ id: "8", is_animated: true }, true)).toBe("/api/v1/icons/8");
  });

  it("refuse files over 8 MB or of another type before sending them", () => {
    expect(profileImageFileProblem({ size: MAXIMUM_PROFILE_IMAGE_BYTES, type: "image/gif" })).toBeNull();
    expect(profileImageFileProblem({ size: MAXIMUM_PROFILE_IMAGE_BYTES + 1, type: "image/png" })).toBe("too-large");
    expect(profileImageFileProblem({ size: 10, type: "image/svg+xml" })).toBe("not-a-picture");
    expect(profileImageFileProblem({ size: 10, type: "" })).toBeNull();
  });
});
