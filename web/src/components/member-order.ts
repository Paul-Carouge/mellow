// The order of the members side sheet (DESIGN.md §5.7).

import type { Member, Role } from "../lib/protocol";
import { displayName } from "./format";

/**
 * Built once, where `localeCompare` sets one up per comparison; the default locale, as
 * `localeCompare` without arguments uses.
 */
const MEMBER_NAME_COLLATOR = new Intl.Collator();

/** A new array, by display name in the reader's alphabet. */
export function membersSortedByDisplayName(members: readonly Member[]): Member[] {
  return [...members].sort((left, right) => MEMBER_NAME_COLLATOR.compare(displayName(left.user), displayName(right.user)));
}

/** A heading of the members side sheet and the members under it, in the order given. */
export type MemberListGroup =
  | { kind: "role"; role: Role; members: Member[] }
  | { kind: "online"; members: Member[] }
  | { kind: "offline"; members: Member[] };

/**
 * The side sheet's groups, as on Discord: online members under the highest role displayed
 * separately that they hold (highest role first), the other online members under "Online", and
 * every offline member under "Offline". Empty groups are left out; each group keeps the order
 * of `members`.
 */
export function memberListGroups(
  members: readonly Member[],
  roles: readonly Role[],
  isOnline: (userId: string) => boolean,
): MemberListGroup[] {
  const displayedRoles = roles
    .filter((role) => role.is_displayed_separately)
    .sort((left, right) => right.position - left.position);
  const membersByRoleId = new Map<string, Member[]>(displayedRoles.map((role) => [role.id, []]));
  const online: Member[] = [];
  const offline: Member[] = [];
  for (const member of members) {
    if (!isOnline(member.user.id)) {
      offline.push(member);
      continue;
    }
    const heldRoleIds = new Set(member.role_ids);
    const highest = displayedRoles.find((role) => heldRoleIds.has(role.id));
    const group = highest ? membersByRoleId.get(highest.id) : undefined;
    if (group) {
      group.push(member);
    } else {
      online.push(member);
    }
  }
  const groups: MemberListGroup[] = [
    ...displayedRoles.map((role): MemberListGroup => ({ kind: "role", role, members: membersByRoleId.get(role.id) ?? [] })),
    { kind: "online", members: online },
    { kind: "offline", members: offline },
  ];
  return groups.filter((group) => group.members.length > 0);
}
