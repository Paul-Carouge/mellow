// What the interface shows of shared screens (ADR-0037, docs/design/web-screen-sharing.md): the
// "LIVE" chip beside a participant, and the state of the screen you watch.

import type { Snowflake, VoiceState } from "../lib/protocol";

/** What the live chip beside someone who shares their screen does. */
export type LiveBadge =
  /** A button: watch their screen large. */
  | "watch"
  /** Your own share: shown, never opened, since watching yourself would loop the picture back. */
  | "own"
  /** They share in a channel you are not connected to: shown, with how to watch. */
  | "join-to-watch"
  /** In your call, but this browser cannot show video (ADR-0037: no codec choice). */
  | "cannot-watch";

/** The call you are in, as far as watching goes. */
export interface Watcher {
  /** The channel of your call once connected; `null` otherwise. */
  readonly connectedChannelId: Snowflake | null;
  readonly canWatchStreams: boolean;
}

/** `null` when they do not share their screen. */
export function liveBadgeOf(state: VoiceState, ownUserId: Snowflake | null, watcher: Watcher): LiveBadge | null {
  if (state.self_stream !== true) {
    return null;
  }
  if (state.user_id === ownUserId) {
    return "own";
  }
  if (state.channel_id !== watcher.connectedChannelId) {
    return "join-to-watch";
  }
  return watcher.canWatchStreams ? "watch" : "cannot-watch";
}

/** What the stream view shows. */
export type WatchedStreamStatus =
  /** They share, and their picture has not arrived yet. */
  | "connecting"
  | "live"
  /** They stopped sharing, left your call, or you left theirs: the view says so, then closes. */
  | "ended";

/**
 * `state` is the watched person's voice state, `callChannelId` the channel of your call (`null`
 * when you are in none) and `hasTrack` whether their picture's track has arrived.
 */
export function watchedStreamStatus(
  state: VoiceState | undefined,
  callChannelId: Snowflake | null,
  hasTrack: boolean,
): WatchedStreamStatus {
  if (state === undefined || callChannelId === null || state.channel_id !== callChannelId || state.self_stream !== true) {
    return "ended";
  }
  return hasTrack ? "live" : "connecting";
}
