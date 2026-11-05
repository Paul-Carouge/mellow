// The "about me" and links of the profile on screen (ADR-0050): read when it opens, since no
// gateway event carries them. An answer for a profile no longer shown is dropped.

import { ApiError } from "../lib/api";
import type { Snowflake, UserProfile } from "../lib/protocol";

export type UserProfileLoad =
  | { readonly kind: "idle" }
  | { readonly kind: "loading"; readonly userId: Snowflake }
  | { readonly kind: "loaded"; readonly userId: Snowflake; readonly profile: UserProfile }
  | { readonly kind: "failed"; readonly userId: Snowflake };

export interface UserProfileLoader {
  readonly current: UserProfileLoad;
  /** Starts reading `userId`'s profile; `null` forgets the one shown. */
  show(userId: Snowflake | null): void;
}

export function createUserProfileLoader(load: (userId: Snowflake) => Promise<UserProfile>): UserProfileLoader {
  let current = $state<UserProfileLoad>({ kind: "idle" });
  let generation = 0;
  return {
    get current() {
      return current;
    },
    show(userId) {
      generation += 1;
      if (userId === null) {
        current = { kind: "idle" };
        return;
      }
      const shownGeneration = generation;
      current = { kind: "loading", userId };
      load(userId).then(
        (profile) => {
          if (shownGeneration === generation) {
            current = { kind: "loaded", userId, profile };
          }
        },
        (error: unknown) => {
          if (shownGeneration !== generation) {
            return;
          }
          // Someone the reader shares nothing with any more: their profile simply has no details.
          current =
            error instanceof ApiError && error.code === "NOT_FOUND"
              ? { kind: "loaded", userId, profile: { user_id: userId, about_me: "", links: [] } }
              : { kind: "failed", userId };
        },
      );
    },
  };
}
