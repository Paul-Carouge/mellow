import { prefersDarkColorScheme } from "../lib/color-scheme.svelte";
import type { Guild, Member, Snowflake } from "../lib/protocol";
import { type RoleMentionChipColors, highestColouredRole, readableRoleColor, roleMentionChipColors } from "./role-colors";
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

// Worked out once per colour and theme: every chip of a role in every row shares it.
const chipColorsByThemeAndColor = new Map<string, RoleMentionChipColors>();

/**
 * A role mention chip's colours on the chat sheet in the current theme (DESIGN.md §5.4). The
 * chip is opaque, so it reads the same on a highlighted row or a search result.
 */
export function roleMentionChipColorsInTheme(color: number): RoleMentionChipColors {
  const key = `${prefersDarkColorScheme.current ? "dark" : "light"}:${color}`;
  let colors = chipColorsByThemeAndColor.get(key);
  if (!colors) {
    colors = roleMentionChipColors(color, colorTokenValue("color-surface-chat"));
    chipColorsByThemeAndColor.set(key, colors);
  }
  return colors;
}
