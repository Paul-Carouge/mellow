// Pure rules of voice channels (ADR-0030, docs/design/web-voice-channels.md): the latency
// indicator, what a microphone error means, and who joined or left the channel you are in.

import type { Snowflake, VoiceState } from "./protocol";

/** At most this round trip to the instance reads as good, then medium; above, poor. */
export const GOOD_ROUND_TRIP_MILLISECONDS = 100;
export const MEDIUM_ROUND_TRIP_MILLISECONDS = 250;

export type LatencyQuality = "good" | "medium" | "poor";

export function latencyQuality(roundTripMilliseconds: number): LatencyQuality {
  if (roundTripMilliseconds <= GOOD_ROUND_TRIP_MILLISECONDS) {
    return "good";
  }
  return roundTripMilliseconds <= MEDIUM_ROUND_TRIP_MILLISECONDS ? "medium" : "poor";
}

/** Reads a field of a stats entry without trusting its shape (`RTCStatsReport` values are loose). */
function field(entry: unknown, name: string): unknown {
  return typeof entry === "object" && entry !== null ? (entry as Record<string, unknown>)[name] : undefined;
}

/**
 * The round trip to the instance from `RTCPeerConnection.getStats()`, in whole milliseconds: the
 * transport's selected candidate pair, else a nominated pair that succeeded. `null` until measured.
 */
export function roundTripMillisecondsOf(stats: Iterable<unknown>): number | null {
  const entries = [...stats];
  const selectedPairId = entries.map((entry) => field(entry, "selectedCandidatePairId")).find((id) => typeof id === "string");
  const pairs = entries.filter((entry) => field(entry, "type") === "candidate-pair");
  const pair =
    pairs.find((candidate) => field(candidate, "id") === selectedPairId) ??
    pairs.find((candidate) => field(candidate, "nominated") === true && field(candidate, "state") === "succeeded");
  const seconds = field(pair, "currentRoundTripTime");
  return typeof seconds === "number" && Number.isFinite(seconds) ? Math.round(seconds * 1000) : null;
}

/** Why there is no microphone: each has its own sentence, saying how to fix it. */
export type MicrophoneProblem = "denied" | "missing" | "busy" | "unavailable";

/** `getUserMedia`'s rejection, by its `DOMException` name; a page without `mediaDevices` (not HTTPS) is unavailable. */
export function microphoneProblemOf(error: unknown): MicrophoneProblem {
  switch (field(error, "name")) {
    case "NotAllowedError":
    case "SecurityError":
      return "denied";
    case "NotFoundError":
    case "OverconstrainedError":
      return "missing";
    case "NotReadableError":
    case "AbortError":
      return "busy";
    default:
      return "unavailable";
  }
}

/**
 * Whether someone else came into or went out of the channel you are in, from their state before
 * and after an event (`undefined`: not connected anywhere you can see).
 */
export function participantChange(
  previous: Pick<VoiceState, "channel_id"> | undefined,
  next: Pick<VoiceState, "channel_id"> | undefined,
  myChannelId: Snowflake | null,
): "joined" | "left" | null {
  if (myChannelId === null) {
    return null;
  }
  const wasHere = previous?.channel_id === myChannelId;
  const isHere = next?.channel_id === myChannelId;
  return wasHere === isHere ? null : isHere ? "joined" : "left";
}

/** What `waitForIceGathering` reads of an `RTCPeerConnection`. */
export interface IceGatheringSource {
  readonly iceGatheringState: RTCIceGatheringState;
  addEventListener(type: "icegatheringstatechange", listener: () => void): void;
  removeEventListener(type: "icegatheringstatechange", listener: () => void): void;
}

/** Resolves once ICE gathering is complete, or after `timeoutMilliseconds`: the offer then holds what was found. */
export function waitForIceGathering(peer: IceGatheringSource, timeoutMilliseconds: number): Promise<void> {
  if (peer.iceGatheringState === "complete") {
    return Promise.resolve();
  }
  return new Promise((resolve) => {
    const finish = () => {
      clearTimeout(timer);
      peer.removeEventListener("icegatheringstatechange", check);
      resolve();
    };
    const check = () => {
      if (peer.iceGatheringState === "complete") {
        finish();
      }
    };
    const timer = setTimeout(finish, timeoutMilliseconds);
    peer.addEventListener("icegatheringstatechange", check);
  });
}
