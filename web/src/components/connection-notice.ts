import type { ConnectionStatus } from "../lib/client.svelte";

/** After this long reconnecting, the bar offers "Retry now" (DESIGN.md §5.2, connection dock). */
export const STILL_RECONNECTING_AFTER_MILLISECONDS = 30_000;
/** "Back online" stays this long, then the bar goes. Not a motion token: reduced motion must not
 * turn the confirmation into a flash. */
export const BACK_ONLINE_SHOWN_MILLISECONDS = 2_000;

/** What the top-of-window connection bar says, if anything. */
export type ConnectionTrouble = "offline" | "connecting" | "checking" | "reconnecting" | "still-reconnecting";

/**
 * The device being offline comes first: it explains the rest, and a socket can take a heartbeat
 * interval to notice it. Signed out or unreachable, the shell (and this bar) is not shown.
 */
export function connectionTrouble(
  status: ConnectionStatus,
  isDeviceOnline: boolean,
  hasBeenReconnectingLong: boolean,
  isCheckingLink: boolean,
): ConnectionTrouble | null {
  if (status === "signed-out" || status === "unreachable") {
    return null;
  }
  if (!isDeviceOnline) {
    return "offline";
  }
  switch (status) {
    case "connecting":
      return "connecting";
    case "reconnecting":
      return hasBeenReconnectingLong ? "still-reconnecting" : "reconnecting";
    case "connected":
      // The browser says online again before the socket is known to have survived.
      return isCheckingLink ? "checking" : null;
    default: {
      const unknown: never = status;
      throw new Error(`unhandled connection status ${String(unknown)}`);
    }
  }
}

/** A socket check keeps the trouble before it: it ends in "Back online" only if that was a loss. */
export function troubleToRemember(
  previous: ConnectionTrouble | null,
  current: ConnectionTrouble | null,
): ConnectionTrouble | null {
  return current === "checking" ? previous : current;
}

/** "Back online" follows only a lost connection, never the first connection of the session. */
export function isComingBackOnline(previous: ConnectionTrouble | null, current: ConnectionTrouble | null): boolean {
  return current === null && previous !== null && previous !== "connecting";
}
