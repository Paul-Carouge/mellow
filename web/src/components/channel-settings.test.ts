import { describe, expect, it } from "vitest";

import { type Channel, type Guild, type Member, Permission, type PermissionOverwrite, type Role } from "../lib/protocol";
import {
  CHANNEL_PERMISSION_GROUPS,
  addableMembers,
  addableRoles,
  canChangeChannelPermission,
  canEditTarget,
  canRemoveOverwrite,
  channelNameAsSaved,
  channelOverviewChanges,
  overwriteStateOf,
  overwriteTargets,
  rebaseOverwriteEdit,
  visibleChannelSections,
  withOverwriteState,
} from "./channel-settings";
import type { RoleManager } from "./server-settings";

const GUILD = "100";
const CHANNEL = "500";

function role(id: string, position: number, name = `role ${id}`): Role {
  return { id, name, color: 0, position, permissions: "0" };
}

const EVERYONE = role(GUILD, 0, "@everyone");
const HELPER = role("201", 1, "Helpers");
const MODERATOR = role("202", 2, "Moderators");
const ADMIN = role("203", 3, "Admins");

function overwrite(id: string, kind: "role" | "member", allow: bigint, deny: bigint): PermissionOverwrite {
  return { id, kind, allow: String(allow), deny: String(deny) };
}

function channel(overwrites: PermissionOverwrite[] = [], kind: Channel["kind"] = "text"): Channel {
  return {
    id: CHANNEL,
    guild_id: GUILD,
    kind,
    name: "general",
    topic: "",
    parent_id: null,
    position: 0,
    permission_overwrites: overwrites,
  };
}

function guild(channels: Channel[] = [channel()]): Guild {
  return {
    id: GUILD,
    name: "Club",
    owner_id: "1",
    my_permissions: "0",
    my_role_ids: [],
    roles: [EVERYONE, HELPER, MODERATOR, ADMIN],
    channels,
  };
}

function member(id: string, displayName: string): Member {
  return { user: { id, username: displayName.toLowerCase(), display_name: displayName }, role_ids: [], joined_at_unix_milliseconds: 0 };
}

/** A moderator: may target Helpers and @everyone, not Moderators or Admins. */
const MODERATOR_MANAGER: RoleManager = { permissions: Permission.MANAGE_ROLES, isOwner: false, highestRolePosition: 2 };
const OWNER_MANAGER: RoleManager = { permissions: 0n, isOwner: true, highestRolePosition: 0 };

describe("visibleChannelSections", () => {
  it("shows Overview with Manage channels and Permissions with Manage roles, as held in the channel", () => {
    const text = channel();
    expect(visibleChannelSections(text, String(Permission.MANAGE_CHANNELS))).toEqual(["overview"]);
    expect(visibleChannelSections(text, String(Permission.MANAGE_ROLES))).toEqual(["permissions"]);
    expect(visibleChannelSections(text, String(Permission.MANAGE_CHANNELS | Permission.MANAGE_ROLES))).toEqual([
      "overview",
      "permissions",
    ]);
    expect(visibleChannelSections(text, String(Permission.SEND_MESSAGES))).toEqual([]);
    expect(visibleChannelSections(text, null)).toEqual([]);
  });

  it("gives administrators both, and categories none yet", () => {
    expect(visibleChannelSections(channel(), String(Permission.ADMINISTRATOR))).toEqual(["overview", "permissions"]);
    expect(visibleChannelSections(channel([], "category"), String(Permission.ADMINISTRATOR))).toEqual([]);
  });
});

describe("channelNameAsSaved", () => {
  it("lowercases and dashes spaces, as the instance does", () => {
    expect(channelNameAsSaved("  Off Topic  ")).toBe("off-topic");
    expect(channelNameAsSaved("Book   club")).toBe("book-club");
    expect(channelNameAsSaved("   ")).toBe("");
  });
});

describe("overwrite states", () => {
  const bits = { allow: String(Permission.SEND_MESSAGES), deny: String(Permission.ATTACH_FILES) };

  it("reads Allow, Deny or Inherit for each bit", () => {
    expect(overwriteStateOf(bits, Permission.SEND_MESSAGES)).toBe("allow");
    expect(overwriteStateOf(bits, Permission.ATTACH_FILES)).toBe("deny");
    expect(overwriteStateOf(bits, Permission.VIEW_CHANNEL)).toBe("inherit");
  });

  it("sets one bit without touching the others, never allowed and denied at once", () => {
    const denied = withOverwriteState(bits, Permission.SEND_MESSAGES, "deny");
    expect(denied).toEqual({ allow: "0", deny: String(Permission.SEND_MESSAGES | Permission.ATTACH_FILES) });
    const inherited = withOverwriteState(denied, Permission.ATTACH_FILES, "inherit");
    expect(inherited).toEqual({ allow: "0", deny: String(Permission.SEND_MESSAGES) });
    const allowed = withOverwriteState(inherited, Permission.SEND_MESSAGES, "allow");
    expect(allowed).toEqual({ allow: String(Permission.SEND_MESSAGES), deny: "0" });
  });

  it("offers only the channel-scoped permissions", () => {
    const keys = CHANNEL_PERMISSION_GROUPS.flatMap((group) => group.permissions.map((permission) => permission.key));
    expect(keys).toContain("sendMessages");
    expect(keys).toContain("manageRoles");
    expect(keys).not.toContain("administrator");
    expect(keys).not.toContain("kickMembers");
    expect(keys).not.toContain("manageServer");
  });
});

describe("rebaseOverwriteEdit", () => {
  const target = { kind: "role" as const, id: HELPER.id };
  const base = { allow: "0", deny: "0" };

  it("keeps someone else's change to another bit under the draft", () => {
    const edit = { target, base, draft: { allow: String(Permission.SEND_MESSAGES), deny: "0" } };
    const stored = { allow: "0", deny: String(Permission.ATTACH_FILES) };
    expect(rebaseOverwriteEdit(edit, stored)?.draft).toEqual({
      allow: String(Permission.SEND_MESSAGES),
      deny: String(Permission.ATTACH_FILES),
    });
  });

  it("ends once the stored overwrite holds the draft (our save, echoed)", () => {
    const draft = { allow: String(Permission.SEND_MESSAGES), deny: String(Permission.VIEW_CHANNEL) };
    expect(rebaseOverwriteEdit({ target, base, draft }, draft)).toBeNull();
  });

  it("ends when the user puts every bit back", () => {
    expect(rebaseOverwriteEdit({ target, base, draft: base }, base)).toBeNull();
  });

  it("moves a bit from allow to deny as one change", () => {
    const allowed = { allow: String(Permission.SEND_MESSAGES), deny: "0" };
    const edit = { target, base: allowed, draft: { allow: "0", deny: String(Permission.SEND_MESSAGES) } };
    expect(rebaseOverwriteEdit(edit, allowed)?.draft).toEqual({ allow: "0", deny: String(Permission.SEND_MESSAGES) });
  });
});

describe("overwriteTargets", () => {
  it("lists roles highest first, then @everyone always, then members", () => {
    const withOverwrites = channel([
      overwrite("9", "member", 0n, 0n),
      overwrite(HELPER.id, "role", 0n, 0n),
      overwrite(ADMIN.id, "role", 0n, 0n),
    ]);
    expect(overwriteTargets(guild([withOverwrites]), withOverwrites)).toEqual([
      { kind: "role", id: ADMIN.id },
      { kind: "role", id: HELPER.id },
      { kind: "role", id: GUILD },
      { kind: "member", id: "9" },
    ]);
    expect(overwriteTargets(guild(), channel())).toEqual([{ kind: "role", id: GUILD }]);
  });
});

describe("what a manager may change", () => {
  it("targets @everyone and roles below their highest; the owner any; members always", () => {
    const club = guild();
    expect(canEditTarget(club, MODERATOR_MANAGER, { kind: "role", id: GUILD })).toBe(true);
    expect(canEditTarget(club, MODERATOR_MANAGER, { kind: "role", id: HELPER.id })).toBe(true);
    expect(canEditTarget(club, MODERATOR_MANAGER, { kind: "role", id: MODERATOR.id })).toBe(false);
    expect(canEditTarget(club, OWNER_MANAGER, { kind: "role", id: ADMIN.id })).toBe(true);
    expect(canEditTarget(club, MODERATOR_MANAGER, { kind: "member", id: "1" })).toBe(true);
  });

  it("changes only permissions held in the channel, all of them as an administrator", () => {
    const held = String(Permission.MANAGE_ROLES | Permission.SEND_MESSAGES);
    expect(canChangeChannelPermission(held, Permission.SEND_MESSAGES)).toBe(true);
    expect(canChangeChannelPermission(held, Permission.MANAGE_MESSAGES)).toBe(false);
    expect(canChangeChannelPermission(String(Permission.ADMINISTRATOR), Permission.MANAGE_MESSAGES)).toBe(true);
  });

  it("removes an overwrite only when it holds every bit in it, never @everyone's", () => {
    const withOverwrites = channel([
      overwrite(GUILD, "role", 0n, Permission.SEND_MESSAGES),
      overwrite(HELPER.id, "role", Permission.SEND_MESSAGES, 0n),
      overwrite("9", "member", Permission.MANAGE_MESSAGES, 0n),
    ]);
    const club = guild([withOverwrites]);
    const held = String(Permission.MANAGE_ROLES | Permission.SEND_MESSAGES);
    const remove = (target: { kind: "role" | "member"; id: string }) =>
      canRemoveOverwrite(club, MODERATOR_MANAGER, held, withOverwrites, target);
    expect(remove({ kind: "role", id: HELPER.id })).toBe(true);
    expect(remove({ kind: "role", id: GUILD })).toBe(false);
    expect(remove({ kind: "member", id: "9" })).toBe(false);
    expect(remove({ kind: "role", id: MODERATOR.id })).toBe(false);
  });

  it("offers to add the roles it may target and the members not listed yet", () => {
    const withOverwrites = channel([overwrite(HELPER.id, "role", 0n, 0n), overwrite("7", "member", 0n, 0n)]);
    const club = guild([withOverwrites]);
    expect(addableRoles(club, withOverwrites, MODERATOR_MANAGER, "")).toEqual([]);
    expect(addableRoles(club, withOverwrites, OWNER_MANAGER, "").map((candidate) => candidate.id)).toEqual([
      ADMIN.id,
      MODERATOR.id,
    ]);
    expect(addableRoles(club, withOverwrites, OWNER_MANAGER, "mod").map((candidate) => candidate.id)).toEqual([
      MODERATOR.id,
    ]);
    const members = [member("7", "Ana"), member("8", "Éloïse"), member("9", "Paul")];
    expect(addableMembers(withOverwrites, members, "").map((candidate) => candidate.user.id)).toEqual(["8", "9"]);
    expect(addableMembers(withOverwrites, members, "eloise").map((candidate) => candidate.user.id)).toEqual(["8"]);
  });
});

describe("channelOverviewChanges", () => {
  it("sends only the typed fields, so a rename by someone else is not undone", () => {
    // Alice typed a topic; Bob renamed #general to #lobby meanwhile.
    const stored = { name: "lobby", topic: "" };
    expect(channelOverviewChanges(stored, { name: null, topic: " News " })).toEqual({ topic: "News" });
  });

  it("sends a name as the instance stores it, and nothing once stored", () => {
    const stored = { name: "general", topic: "Hi" };
    expect(channelOverviewChanges(stored, { name: "Off Topic", topic: null })).toEqual({ name: "off-topic" });
    expect(channelOverviewChanges(stored, { name: "General ", topic: "Hi " })).toEqual({});
  });
});
