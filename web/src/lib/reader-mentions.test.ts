import { describe, expect, it } from "vitest";

import { type MentionReader, doesMessageMentionReader } from "./reader-mentions";

const GUILD = "100";
const MODERATORS = "201";
const ARTISTS = "202";
const reader: MentionReader = { userId: "7", guildId: GUILD, roleIds: [MODERATORS] };
const author = { id: "8", username: "ana", display_name: "Ana" };

describe("doesMessageMentionReader", () => {
  it("is true when the reader is named", () => {
    expect(doesMessageMentionReader({ author, mention_ids: ["7"] }, reader)).toBe(true);
    expect(doesMessageMentionReader({ author, mention_ids: ["9"] }, reader)).toBe(false);
  });

  it("is true for a role the reader holds, not for another one", () => {
    expect(doesMessageMentionReader({ author, mention_role_ids: [ARTISTS, MODERATORS] }, reader)).toBe(true);
    expect(doesMessageMentionReader({ author, mention_role_ids: [ARTISTS] }, reader)).toBe(false);
  });

  it("is true for @everyone, written as the guild's id, even without any role", () => {
    expect(doesMessageMentionReader({ author, mention_role_ids: [GUILD] }, { ...reader, roleIds: [] })).toBe(true);
  });

  it("is false for the reader's own message, even naming a role they hold", () => {
    const own = { ...author, id: reader.userId };
    expect(doesMessageMentionReader({ author: own, mention_role_ids: [MODERATORS, GUILD] }, reader)).toBe(false);
  });

  it("is false without mentions, from an older server, and for roles in a conversation", () => {
    expect(doesMessageMentionReader({ author }, reader)).toBe(false);
    expect(doesMessageMentionReader({ author, mention_role_ids: [GUILD] }, { userId: "7", guildId: null, roleIds: [] })).toBe(false);
  });
});
