// Friends, requests and blocks (ADR-0025): what `Ready` and the `RELATIONSHIP_*` events bring,
// and the user's own changes, applied as soon as the instance answers.

import { SvelteMap } from "svelte/reactivity";

import { ApiError, type RestClient } from "./api";
import type { PublicUser, Relationship, RelationshipKind, Snowflake } from "./protocol";

export class Relationships {
  /** By the other person's id. */
  readonly byUserId = new SvelteMap<Snowflake, Relationship>();

  constructor(private readonly rest: Pick<RestClient, "sendFriendRequest" | "setRelationship" | "removeRelationship">) {}

  /** `Ready`'s list replaces everything; `undefined` from an older instance empties it. */
  replaceAll(relationships: readonly Relationship[] | undefined): void {
    this.byUserId.clear();
    for (const relationship of relationships ?? []) {
      this.byUserId.set(relationship.user.id, relationship);
    }
  }

  clear(): void {
    this.byUserId.clear();
  }

  /** `RELATIONSHIP_ADD`, or the instance's answer to the user's own change. */
  apply(relationship: Relationship): void {
    this.byUserId.set(relationship.user.id, relationship);
  }

  /** `RELATIONSHIP_REMOVE`. */
  forget(userId: Snowflake): void {
    this.byUserId.delete(userId);
  }

  /** A user's profile changed: their relationship shows the new one. */
  updateUser(userId: Snowflake, update: (user: PublicUser) => PublicUser): void {
    const relationship = this.byUserId.get(userId);
    if (relationship) {
      this.byUserId.set(userId, { ...relationship, user: update(relationship.user) });
    }
  }

  /** The relationships of `kind`, by name. */
  ofKind(kind: RelationshipKind): Relationship[] {
    return [...this.byUserId.values()]
      .filter((relationship) => relationship.kind === kind)
      .sort((a, b) =>
        (a.user.display_name || a.user.username).localeCompare(b.user.display_name || b.user.username),
      );
  }

  /** Requests waiting for the user's answer: the Friends entry's badge. */
  incomingRequestCount(): number {
    return this.ofKind("incoming_request").length;
  }

  isBlocked(userId: Snowflake): boolean {
    return this.byUserId.get(userId)?.kind === "blocked";
  }

  /**
   * Asks `username` to be friends, or accepts their request. The answer is applied at once; the
   * gateway's `RELATIONSHIP_ADD` brings the same.
   */
  async sendFriendRequest(username: string): Promise<Relationship> {
    const relationship = await this.rest.sendFriendRequest(username.trim());
    this.apply(relationship);
    return relationship;
  }

  async acceptFriendRequest(userId: Snowflake): Promise<void> {
    this.apply(await this.rest.setRelationship(userId, "friend"));
  }

  async blockUser(userId: Snowflake): Promise<void> {
    this.apply(await this.rest.setRelationship(userId, "blocked"));
  }

  /**
   * Removes a friend, refuses or cancels a request, or unblocks. Already gone (the other person
   * removed it, or another session did) counts as done: the wanted state is reached.
   */
  async removeRelationship(userId: Snowflake): Promise<void> {
    try {
      await this.rest.removeRelationship(userId);
    } catch (error) {
      if (!(error instanceof ApiError && error.code === "NOT_FOUND")) {
        throw error;
      }
    }
    this.forget(userId);
  }
}
