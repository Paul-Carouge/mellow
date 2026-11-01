// The order of the members side sheet (DESIGN.md §5.7).

import type { Member } from "../lib/protocol";
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
