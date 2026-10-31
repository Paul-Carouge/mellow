// What the voice panel says (DESIGN.md §5.3, docs/design/web-voice-channels.md): its connection
// line and latency, and the title, sentence and actions of a problem.

import { strings } from "../i18n";
import type { VoiceDisconnectReason, VoiceJoinRefusal } from "../lib/protocol";
import { type LatencyQuality, latencyQuality } from "../lib/voice";
import type { VoiceProblem } from "../lib/voice-session.svelte";
import type { IconName } from "./Icon.svelte";

export interface SignalLook {
  readonly icon: IconName;
  /** `null` before the first measure. */
  readonly quality: LatencyQuality | null;
  /** The tooltip, and what screen readers hear after the state. */
  readonly description: string;
}

const SIGNAL_ICONS: Record<LatencyQuality, IconName> = {
  good: "cell-signal-high",
  medium: "cell-signal-medium",
  poor: "cell-signal-low",
};

/** Bars that fall with the quality, so colour is never the only sign. */
export function signalLook(roundTripMilliseconds: number | null): SignalLook {
  if (roundTripMilliseconds === null) {
    return { icon: "cell-signal-high", quality: null, description: strings.voice.latencyMeasuring };
  }
  const quality = latencyQuality(roundTripMilliseconds);
  return {
    icon: SIGNAL_ICONS[quality],
    quality,
    description: `${strings.voice.latency(roundTripMilliseconds)}, ${strings.voice.latencyQuality[quality]}`,
  };
}

export interface ProblemLook {
  readonly title: string;
  readonly sentence: string;
  /** `notice` for what is not an error (voice moved to another tab). */
  readonly tone: "error" | "notice";
  readonly canRetry: boolean;
}

/** Refusals that may pass on their own: a retry is offered. */
const RETRIABLE_REFUSALS: ReadonlySet<VoiceJoinRefusal> = new Set(["channel_full", "instance_full", "rate_limited"]);

function refusalSentence(reason: VoiceJoinRefusal): string {
  const sentences: Partial<Record<string, string>> = strings.voice.refusals;
  return sentences[reason] ?? strings.voice.unknownReason;
}

function disconnectionSentence(reason: VoiceDisconnectReason): string {
  const sentences: Partial<Record<string, string>> = strings.voice.disconnections;
  return sentences[reason] ?? strings.voice.unknownReason;
}

/** `channelName` is the channel the problem is about, as the list shows it. */
export function problemLook(problem: VoiceProblem, channelName: string): ProblemLook {
  switch (problem.kind) {
    case "unsupported":
      return { title: strings.voice.unsupportedTitle, sentence: strings.voice.unsupported, tone: "error", canRetry: false };
    case "refused":
      return {
        title: strings.voice.couldNotJoin(channelName),
        sentence: refusalSentence(problem.reason),
        tone: "error",
        canRetry: RETRIABLE_REFUSALS.has(problem.reason),
      };
    case "no-answer":
      return { title: strings.voice.couldNotJoin(channelName), sentence: strings.voice.noAnswer, tone: "error", canRetry: true };
    case "unreachable":
      return { title: strings.voice.couldNotJoin(channelName), sentence: strings.voice.unreachable, tone: "error", canRetry: true };
    case "connection-lost":
      return {
        title: strings.voice.disconnectedFrom(channelName),
        sentence: strings.voice.connectionLost,
        tone: "error",
        canRetry: true,
      };
    case "disconnected":
      return problem.reason === "moved_elsewhere"
        ? { title: strings.voice.movedElsewhere, sentence: disconnectionSentence(problem.reason), tone: "notice", canRetry: false }
        : {
            title: strings.voice.disconnectedFrom(channelName),
            sentence: disconnectionSentence(problem.reason),
            tone: "error",
            canRetry: problem.reason === "media_timeout",
          };
    default: {
      const unknown: never = problem;
      throw new Error(`unhandled voice problem ${JSON.stringify(unknown)}`);
    }
  }
}
