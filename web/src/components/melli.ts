import type { ConnectionStatus } from "../lib/client.svelte";

/** What Melli is showing (DESIGN.md §2.10). */
export type MelliState =
  | "idle"
  | "connecting"
  | "connection-lost"
  | "notification"
  | "do-not-disturb"
  | "incoming-call"
  | "in-call"
  | "update-available"
  | "sleeping";

/**
 * The Home button's Melli: connection trouble first, then Do not disturb (quiet even with unread
 * messages), then unread conversations or mentions.
 */
export function homeMelliState(
  status: ConnectionStatus,
  hasSomethingForTheReader: boolean,
  isDoNotDisturb: boolean,
): MelliState {
  if (status === "reconnecting") {
    return "connection-lost";
  }
  if (status === "connecting") {
    return "connecting";
  }
  if (isDoNotDisturb) {
    return "do-not-disturb";
  }
  return hasSomethingForTheReader ? "notification" : "idle";
}
