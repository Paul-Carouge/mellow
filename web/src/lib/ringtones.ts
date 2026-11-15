// The ringtone of an incoming call (ADR-0062): the owner's "Friendly Chime", and "Jingle of the
// Handbells" from 1 to 31 December, by the device's local date, without a setting. The files live
// in design/sounds/ (Ogg Opus, about 10 s, looped while the call rings); Vite hashes and caches
// them for good. Mirrors `mellow_client_core::ringtones`.
import standardRingtoneUrl from "../../../design/sounds/ringtone.opus?url";
import christmasRingtoneUrl from "../../../design/sounds/ringtone-christmas.opus?url";

export type Ringtone = "standard" | "christmas";

/** December, as `Date.getMonth()` counts (0-based). */
const CHRISTMAS_RINGTONE_MONTH_INDEX = 11;

/** The ringtone for this local date: the Christmas one all December. */
export function ringtoneFor(localDate: Date): Ringtone {
  return localDate.getMonth() === CHRISTMAS_RINGTONE_MONTH_INDEX ? "christmas" : "standard";
}

/** Where the ringtone's file is served from. */
export function ringtoneUrl(ringtone: Ringtone): string {
  return ringtone === "christmas" ? christmasRingtoneUrl : standardRingtoneUrl;
}
