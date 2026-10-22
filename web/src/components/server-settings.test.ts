import { describe, expect, it } from "vitest";

import { type AuditLogEntry, type Guild, type Member, Permission, type Role } from "../lib/protocol";
import {
  auditChangeLines,
  auditEntrySentence,
  canManageRole,
  canOpenServerSettings,
  focusAfterRemoval,
  isRoleEditPending,
  matchesMemberSearch,
  mustStayOnPage,
  outranksMember,
  rebaseRoleEdit,
  roleIdsAfterMove,
  roleManagerOf,
  roleUpdateBody,
  rolesHighestFirst,
  visibleSections,
} from "./server-settings";

const GUILD = "100";
const OWNER = "1";
const ME = "2";
const OTHER = "3";

function role(id: string, position: number, permissions: bigint = 0n): Role {
  return { id, name: `role ${id}`, color: 0, position, permissions: String(permissions) };
}

const EVERYONE = role(GUILD, 0, Permission.VIEW_CHANNEL | Permission.SEND_MESSAGES);
const HELPER = role("201", 1);
const MODERATOR = role("202", 2, Permission.MANAGE_ROLES | Permission.KICK_MEMBERS);
const ADMIN = role("203", 3, Permission.ADMINISTRATOR);

function guild(myPermissions: bigint, myRoleIds: string[] = []): Guild {
  return {
    id: GUILD,
    name: "Club",
    owner_id: OWNER,
    my_permissions: String(myPermissions),
    my_role_ids: myRoleIds,
    roles: [EVERYONE, HELPER, MODERATOR, ADMIN],
    channels: [
      { id: "300", guild_id: GUILD, kind: "text", name: "general", topic: "", parent_id: null, position: 0, permission_overwrites: [] },
    ],
  };
}

function member(id: string, displayName: string, roleIds: string[] = []): Member {
  return { user: { id, username: displayName.toLowerCase(), display_name: displayName }, role_ids: roleIds, joined_at_unix_milliseconds: 0 };
}

describe("sections", () => {
  it("shows each section only with its permission, in Discord's order", () => {
    expect(visibleSections(guild(Permission.VIEW_CHANNEL))).toEqual([]);
    expect(visibleSections(guild(Permission.KICK_MEMBERS))).toEqual(["members"]);
    expect(visibleSections(guild(Permission.MANAGE_GUILD | Permission.BAN_MEMBERS))).toEqual([
      "overview",
      "invites",
      "members",
      "bans",
    ]);
    expect(visibleSections(guild(Permission.ADMINISTRATOR))).toEqual([
      "overview",
      "roles",
      "invites",
      "members",
      "bans",
      "moderation-log",
    ]);
  });

  it("opens for the owner even without a section, so they can delete the server", () => {
    expect(canOpenServerSettings(guild(0n), OWNER)).toBe(true);
    expect(canOpenServerSettings(guild(0n), ME)).toBe(false);
    expect(canOpenServerSettings(guild(Permission.VIEW_AUDIT_LOG), ME)).toBe(true);
  });
});

describe("unsaved changes", () => {
  it("keep the reader only on the page that holds them", () => {
    expect(mustStayOnPage("roles", "roles")).toBe(true);
    // Roles went away with MANAGE_ROLES: the page shown now holds nothing.
    expect(mustStayOnPage("roles", "members")).toBe(false);
    expect(mustStayOnPage(null, "overview")).toBe(false);
    expect(mustStayOnPage(null, null)).toBe(false);
  });
});

describe("focus after removing a row", () => {
  it("moves to the next row, else the previous one, else nowhere", () => {
    expect(focusAfterRemoval(["a", "b", "c"], "b")).toBe("c");
    expect(focusAfterRemoval(["a", "b", "c"], "c")).toBe("b");
    expect(focusAfterRemoval(["a"], "a")).toBeNull();
    expect(focusAfterRemoval(["a"], "z")).toBeNull();
  });
});

describe("roles", () => {
  const moderatorGuild = guild(Permission.MANAGE_ROLES | Permission.KICK_MEMBERS, [MODERATOR.id]);
  const moderator = roleManagerOf(moderatorGuild, ME);

  it("lists roles highest first with @everyone last", () => {
    expect(rolesHighestFirst(moderatorGuild).map((candidate) => candidate.id)).toEqual(["203", "202", "201", GUILD]);
  });

  it("lets managers edit only @everyone and roles below their highest whose permissions they hold", () => {
    expect(canManageRole(moderator, EVERYONE, GUILD)).toBe(false);
    expect(canManageRole(moderator, HELPER, GUILD)).toBe(true);
    expect(canManageRole(moderator, MODERATOR, GUILD)).toBe(false);
    expect(canManageRole(moderator, ADMIN, GUILD)).toBe(false);
    const owner = roleManagerOf(guild(Permission.ADMINISTRATOR | Permission.MANAGE_ROLES), OWNER);
    expect(canManageRole(owner, ADMIN, GUILD)).toBe(true);
    const withEveryonesPermissions = roleManagerOf(
      guild(Permission.MANAGE_ROLES | Permission.VIEW_CHANNEL | Permission.SEND_MESSAGES, [MODERATOR.id]),
      ME,
    );
    expect(canManageRole(withEveryonesPermissions, EVERYONE, GUILD)).toBe(true);
  });

  it("moves a role by swapping it with its neighbour, only when both are manageable", () => {
    const owner = roleManagerOf(guild(BigInt.asUintN(64, -1n)), OWNER);
    expect(roleIdsAfterMove(guild(0n), owner, HELPER.id, "up")).toEqual(["202", "201", "203"]);
    expect(roleIdsAfterMove(guild(0n), owner, ADMIN.id, "up")).toBeNull();
    expect(roleIdsAfterMove(guild(0n), owner, HELPER.id, "down")).toBeNull();
    expect(roleIdsAfterMove(moderatorGuild, moderator, HELPER.id, "up")).toBeNull();
  });

  it("keeps a draft's own changes on top of a role changed meanwhile", () => {
    const stored = role("201", 1, Permission.SEND_MESSAGES | Permission.ATTACH_FILES);
    const edit = {
      roleId: stored.id,
      base: { name: stored.name, color: 0, permissions: stored.permissions },
      draft: { name: "Helpers", color: 0, permissions: String(Permission.SEND_MESSAGES | Permission.ATTACH_FILES | Permission.KICK_MEMBERS) },
    };
    // Someone else removed ATTACH_FILES and set a colour.
    const changed = { ...stored, color: 0x3498db, permissions: String(Permission.SEND_MESSAGES) };
    expect(rebaseRoleEdit(edit, changed)?.draft).toEqual({
      name: "Helpers",
      color: 0x3498db,
      permissions: String(Permission.SEND_MESSAGES | Permission.KICK_MEMBERS),
    });
    const saved = { ...stored, name: "Helpers", permissions: edit.draft.permissions };
    expect(rebaseRoleEdit(edit, saved)).toBeNull();
    expect(rebaseRoleEdit({ ...edit, draft: { ...edit.base, name: `${stored.name} ` } }, stored)).toBeNull();
  });

  it("drops an edit once the stored role holds it, so a later undo cannot revive it", () => {
    const stored = role("201", 1);
    const edit = {
      roleId: stored.id,
      base: { name: stored.name, color: 0, permissions: "0" },
      draft: { name: "Helpers", color: 0, permissions: "0" },
    };
    expect(isRoleEditPending(edit, stored)).toBe(true);
    // Our save, echoed by the gateway: nothing left to save, the page drops the edit.
    expect(isRoleEditPending(edit, { ...stored, name: "Helpers" })).toBe(false);
    // Kept, the same edit would come back once someone renames the role back.
    expect(rebaseRoleEdit(edit, stored)).not.toBeNull();
    expect(isRoleEditPending(edit, null)).toBe(false);
    expect(isRoleEditPending(edit, role("202", 2))).toBe(false);
  });

  it("sends only the changed fields, and only permissions for @everyone", () => {
    const stored = role("201", 1);
    expect(roleUpdateBody(stored, { name: " Helpers ", color: 0, permissions: "0" }, false)).toEqual({ name: "Helpers" });
    expect(roleUpdateBody(stored, { name: "  ", color: 0, permissions: "0" }, false)).toBeNull();
    expect(roleUpdateBody(EVERYONE, { name: "x", color: 5, permissions: "4" }, true)).toEqual({ permissions: "4" });
  });
});

describe("members", () => {
  const moderatorGuild = guild(Permission.KICK_MEMBERS, [MODERATOR.id]);
  const moderator = roleManagerOf(moderatorGuild, ME);

  it("acts only on members ranked strictly below, never the owner or oneself", () => {
    expect(outranksMember(moderatorGuild, moderator, member(OTHER, "Ana", [HELPER.id]), ME)).toBe(true);
    expect(outranksMember(moderatorGuild, moderator, member(OTHER, "Ana", [MODERATOR.id]), ME)).toBe(false);
    expect(outranksMember(moderatorGuild, moderator, member(OWNER, "Owner"), ME)).toBe(false);
    expect(outranksMember(moderatorGuild, moderator, member(ME, "Me"), ME)).toBe(false);
  });

  it("searches display names and usernames without case or accents", () => {
    const zoe = member(OTHER, "Zoé");
    expect(matchesMemberSearch(zoe, "zoe")).toBe(true);
    expect(matchesMemberSearch(zoe, " ZO ")).toBe(true);
    expect(matchesMemberSearch(zoe, "ana")).toBe(false);
    expect(matchesMemberSearch(zoe, "")).toBe(true);
  });
});

describe("moderation log", () => {
  const context = { guild: guild(Permission.ADMINISTRATOR), people: [member(OWNER, "Ana").user, member(OTHER, "Paul").user] };

  function entry(overrides: Partial<AuditLogEntry>): AuditLogEntry {
    return {
      id: "1",
      actor_id: OWNER,
      action: "member_kick",
      target_id: null,
      context_id: null,
      reason: "",
      changes: [],
      ...overrides,
    };
  }

  it("names people, roles and channels, and what is gone", () => {
    expect(auditEntrySentence(entry({ target_id: OTHER }), context)).toBe("Ana kicked Paul");
    expect(auditEntrySentence(entry({ target_id: "999" }), context)).toBe("Ana kicked a former member");
    expect(auditEntrySentence(entry({ actor_id: null, target_id: OTHER }), context)).toBe("A deleted account kicked Paul");
    expect(
      auditEntrySentence(entry({ action: "member_role_add", target_id: OTHER, context_id: HELPER.id }), context),
    ).toBe("Ana gave role 201 to Paul");
    expect(auditEntrySentence(entry({ action: "overwrite_set", target_id: GUILD, context_id: "300" }), context)).toBe(
      "Ana changed what role 100 can do in #general",
    );
    expect(auditEntrySentence(entry({ action: "message_pin", target_id: "500", context_id: "300" }), context)).toBe(
      "Ana pinned a message in #general",
    );
    expect(auditEntrySentence(entry({ action: "role_update", target_id: "404" }), context)).toBe(
      "Ana edited the role a deleted role",
    );
  });

  it("tells renames and invites from their changes, and actions of a newer server", () => {
    const rename = entry({
      action: "guild_update",
      changes: [{ key: "name", old_value: "Club", new_value: "Book club" }],
    });
    expect(auditEntrySentence(rename, context)).toBe("Ana renamed the server from Club to Book club");
    expect(auditChangeLines(rename)).toEqual([]);
    const revoked = entry({ action: "invite_delete", changes: [{ key: "code", old_value: "abc", new_value: "" }] });
    expect(auditEntrySentence(revoked, context)).toBe("Ana revoked the invite abc");
    expect(auditEntrySentence(entry({ action: "voice_kick" as AuditLogEntry["action"] }), context)).toBe(
      "Ana made a change this version of Mellow can't show yet",
    );
  });

  it("lists granted and removed permissions by name", () => {
    const update = entry({
      action: "role_update",
      changes: [
        {
          key: "permissions",
          old_value: String(Permission.BAN_MEMBERS | Permission.SEND_MESSAGES),
          new_value: String(Permission.SEND_MESSAGES | Permission.KICK_MEMBERS | Permission.ATTACH_FILES),
        },
        { key: "name", old_value: "Mods", new_value: "Moderators" },
        { key: "color", old_value: "0", new_value: "255" },
      ],
    });
    expect(auditChangeLines(update)).toEqual([
      "Granted: Attach files, Kick members",
      "Removed: Ban members",
      "Name: Mods to Moderators",
      "Colour changed",
    ]);
  });
});
