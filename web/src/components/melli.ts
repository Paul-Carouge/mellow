import type { ConnectionStatus } from "../lib/client.svelte";

/** What Melli is showing (DESIGN.md §2.10). */
export type MelliState =
  | "idle"
  | "connecting"
  | "connection-lost"
  | "notification"
  | "incoming-call"
  | "in-call"
  | "update-available"
  | "sleeping";

/** The Home button's Melli: connection trouble first, then unread direct messages. */
export function homeMelliState(status: ConnectionStatus, hasUnreadConversation: boolean): MelliState {
  if (status === "reconnecting") {
    return "connection-lost";
  }
  if (status === "connecting") {
    return "connecting";
  }
  return hasUnreadConversation ? "notification" : "idle";
}
