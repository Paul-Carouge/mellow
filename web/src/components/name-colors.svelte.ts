import { prefersDarkColorScheme } from "../lib/color-scheme.svelte";
import type { Guild, Member, Snowflake } from "../lib/protocol";
import { highestColouredRole, readableRoleColor } from "./role-colors";
import { colorTokenValue } from "./tokens";

/**
 * Each member's name colour on the given surface, by user id; members without a coloured role
 * are absent and keep the surface's text colour. Recomputed when the theme changes.
 */
export function nameColorsByUserId(
  guild: Guild | null,
  members: readonly Member[] | undefined,
  surfaceToken: string,
): Map<Snowflake, string> {
  void prefersDarkColorScheme.current;
  const colors = new Map<Snowflake, string>();
  if (!guild || !members) {
    return colors;
  }
  const surface = colorTokenValue(surfaceToken);
  const byRoleId = new Map<Snowflake, string>();
  for (const member of members) {
    const role = highestColouredRole(guild, member.role_ids);
    if (role) {
      const color = byRoleId.get(role.id) ?? readableRoleColor(role.color, surface);
      byRoleId.set(role.id, color);
      colors.set(member.user.id, color);
    }
  }
  return colors;
}
