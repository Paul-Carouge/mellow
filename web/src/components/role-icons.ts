// Role icons (ADR-0050): each member shows the icon of their highest role that has one, next to
// their name in messages and the member list, as on Discord.

import type { Guild, Member, Role, Snowflake } from "../lib/protocol";

/** A role with a picture or an emoji. */
export function hasRoleIcon(role: Role): boolean {
  return Boolean(role.icon) || Boolean(role.unicode_emoji);
}

/** The member's highest role with an icon; `@everyone` never has one. */
export function highestRoleWithIcon(guild: Guild, roleIds: readonly Snowflake[]): Role | null {
  let highest: Role | null = null;
  for (const role of guild.roles) {
    if (hasRoleIcon(role) && roleIds.includes(role.id) && (highest === null || role.position > highest.position)) {
      highest = role;
    }
  }
  return highest;
}

/**
 * Each member's shown role icon, by user id; members without one are absent. Worked out once per
 * member list, so a row reads its icon from the map rather than walking the roles again.
 */
export function roleIconsByUserId(guild: Guild | null, members: readonly Member[] | undefined): Map<Snowflake, Role> {
  const icons = new Map<Snowflake, Role>();
  if (!guild || !members || !guild.roles.some(hasRoleIcon)) {
    return icons;
  }
  for (const member of members) {
    const role = highestRoleWithIcon(guild, member.role_ids);
    if (role) {
      icons.set(member.user.id, role);
    }
  }
  return icons;
}
