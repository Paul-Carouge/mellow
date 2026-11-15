// Which call sound plays, under which key, and for how long (DESIGN.md §5.6.3, ADR-0062). Pure, so
// the rules a ringing follows are tested apart from Web Audio.

import { type CallSound, callSound, localRingMilliseconds, ringKeyOf } from "../lib/calls";
import type { Call } from "../lib/protocol";

/**
 * A sound to play. Only a new `key` starts one: a call's other changes (someone else answering, the
 * reader joining a moment before it stops ringing them) never restart or swap it.
 */
export interface PlannedCallSound {
  readonly kind: CallSound;
  readonly key: string;
  /** On this device's clock: how long it may still ring (`localRingMilliseconds`). */
  readonly durationMilliseconds: number;
}

/**
 * For the call ringing the reader: one sound per ringing, chosen when it starts; Do not disturb
 * turned on while it rings is a new key, which silences it.
 */
export function plannedIncomingSound(
  ringing: Call | null,
  facts: { isInAnotherCallOrVoiceChannel: boolean; isDoNotDisturb: boolean; now: number },
): PlannedCallSound | null {
  if (ringing === null) {
    return null;
  }
  const kind = callSound("incoming", facts.isInAnotherCallOrVoiceChannel, facts.isDoNotDisturb);
  return kind === null
    ? null
    : { kind, key: `${ringKeyOf(ringing)}:${facts.isDoNotDisturb}`, durationMilliseconds: localRingMilliseconds(ringing, facts.now) };
}

/** For the reader's own call while the others are rung: the waiting tone. */
export function plannedRingback(ownRingingCall: Call | null, now: number): PlannedCallSound | null {
  return ownRingingCall === null
    ? null
    : { kind: "ringback", key: `ringback:${ringKeyOf(ownRingingCall)}`, durationMilliseconds: localRingMilliseconds(ownRingingCall, now) };
}
