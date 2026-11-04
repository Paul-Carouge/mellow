import { describe, expect, it } from "vitest";

import type { Role } from "../lib/protocol";
import { MAXIMUM_MENTION_SUGGESTIONS } from "./mentions";
import {
  messageAsPlainText,
  roleMentionLabel,
  rolesWriterMayMention,
  suggestRoleMentions,
  withRoleNamesAsMarkup,
} from "./role-mentions";

const GUILD = "100";

function role(id: string, name: string, position: number, isMentionable = false): Role {
  return { id, name, color: 0, position, permissions: "0", is_displayed_separately: false, is_mentionable: isMentionable };
}

const everyoneRole = role(GUILD, "@everyone", 0);
const moderators = role("201", "Modérateurs", 3, true);
const sound = role("202", "Équipe son", 2);
const admins = role("203", "Admins", 5);
const guildRoles = [everyoneRole, sound, moderators, admins];
const NOBODY = new Set<string>();

describe("roleMentionLabel and messageAsPlainText", () => {
  it("names a role, or says it was deleted", () => {
    expect(roleMentionLabel(moderators)).toBe("@Modérateurs");
    expect(roleMentionLabel(null)).toBe("@deleted-role");
  });

  it("reads markup as names, emoji included", () => {
    const roles = new Map([[moderators.id, moderators]]);
    expect(messageAsPlainText("<@&201> and <@&999> <:party:5>", roles)).toBe("@Modérateurs and @deleted-role :party:");
  });
});

describe("rolesWriterMayMention", () => {
  it("keeps only mentionable roles without MENTION_EVERYONE, never @everyone's role", () => {
    expect(rolesWriterMayMention(guildRoles, GUILD, false)).toEqual([moderators]);
  });

  it("keeps every role with MENTION_EVERYONE, highest first", () => {
    expect(rolesWriterMayMention(guildRoles, GUILD, true)).toEqual([admins, moderators, sound]);
  });

  it("reads a role from an older server, without the field, as not mentionable", () => {
    const older = { ...moderators, is_mentionable: undefined };
    expect(rolesWriterMayMention([older], GUILD, false)).toEqual([]);
  });
});

describe("suggestRoleMentions", () => {
  const allowed = rolesWriterMayMention(guildRoles, GUILD, true);

  it("offers @everyone first, then every role, for a lone @", () => {
    expect(suggestRoleMentions("", allowed, true)).toEqual([
      { kind: "everyone" },
      { kind: "role", role: admins },
      { kind: "role", role: moderators },
      { kind: "role", role: sound },
    ]);
  });

  it("matches names ignoring case and accents, a start first, then a word, then inside", () => {
    expect(suggestRoleMentions("mod", allowed, true)).toEqual([{ kind: "role", role: moderators }]);
    expect(suggestRoleMentions("son", allowed, true)).toEqual([{ kind: "role", role: sound }]);
    expect(suggestRoleMentions("e", allowed, true)).toEqual([
      { kind: "everyone" },
      { kind: "role", role: sound },
      { kind: "role", role: moderators },
    ]);
  });

  it("never offers @everyone without the permission", () => {
    expect(suggestRoleMentions("ev", [moderators], false)).toEqual([]);
  });

  it("offers at most the maximum", () => {
    const many = Array.from({ length: 20 }, (_, index) => role(`${300 + index}`, `Team ${index}`, index, true));
    expect(suggestRoleMentions("team", many, false)).toHaveLength(MAXIMUM_MENTION_SUGGESTIONS);
  });
});

describe("withRoleNamesAsMarkup", () => {
  it("leaves a dotted username that starts like a role name to the person", () => {
    const mod = role("301", "Mod", 1, true);
    expect(withRoleNamesAsMarkup("@mod.ana hi", [mod], new Set(["mod.ana"]))).toBe("@mod.ana hi");
    expect(withRoleNamesAsMarkup("ask @Mod.", [mod], NOBODY)).toBe("ask <@&301>.");
  });
  const allowed = [moderators, sound];

  it("turns a picked role's @Name into its markup, longest name first, any case", () => {
    expect(withRoleNamesAsMarkup("@Équipe son, @modérateurs!", allowed, NOBODY)).toBe("<@&202>, <@&201>!");
    expect(withRoleNamesAsMarkup("**@Modérateurs**", allowed, NOBODY)).toBe("**<@&201>**");
  });

  it("leaves code, escapes, words running on and addresses alone", () => {
    expect(withRoleNamesAsMarkup("`@Modérateurs` \\@Modérateurs @Modérateursx a@Modérateurs", allowed, NOBODY)).toBe(
      "`@Modérateurs` \\@Modérateurs @Modérateursx a@Modérateurs",
    );
  });

  it("leaves roles the writer may not mention, usernames and everyone as written", () => {
    expect(withRoleNamesAsMarkup("@Admins", allowed, NOBODY)).toBe("@Admins");
    expect(withRoleNamesAsMarkup("@modérateurs", allowed, new Set(["modérateurs"]))).toBe("@modérateurs");
    expect(withRoleNamesAsMarkup("@everyone", [role("204", "everyone", 1, true)], NOBODY)).toBe("@everyone");
  });
});
