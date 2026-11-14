// What a profile says about a person beyond their card (DESIGN.md §5.7.1, §5.7.2): their place in
// the server being read (roles, the role icon after their name, when they joined it) and when
// they joined Mellow, read from their id. Known locally: shown at once, no request.

import type { Guild, Member, Role, Snowflake } from "../lib/protocol";
import { unixMillisecondsOfSnowflake } from "./format";
import { highestRoleWithIcon } from "./role-icons";

export interface ProfileFacts {
  /** Null outside a server, or before its member list arrived, or once they left it. */
  readonly member: Member | null;
  readonly isOwner: boolean;
  readonly roleIcon: Role | null;
  readonly joinedServerAtUnixMilliseconds: number | null;
  /** The account's creation: ids carry the moment they were made (ADR-0004). */
  readonly joinedMellowAtUnixMilliseconds: number;
}

export function profileFactsOf(userId: Snowflake, guild: Guild | null, members: readonly Member[] | undefined): ProfileFacts {
  const member = (guild && members?.find((candidate) => candidate.user.id === userId)) || null;
  return {
    member,
    isOwner: guild !== null && member !== null && guild.owner_id === userId,
    roleIcon: guild && member ? highestRoleWithIcon(guild, member.role_ids) : null,
    joinedServerAtUnixMilliseconds: member?.joined_at_unix_milliseconds ?? null,
    joinedMellowAtUnixMilliseconds: unixMillisecondsOfSnowflake(userId),
  };
}
