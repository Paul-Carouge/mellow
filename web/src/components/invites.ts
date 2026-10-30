// Invite codes and limits as people read them (DESIGN.md §5.3 "Invite people", §5.8 "Invites").

import { type Catalogue, strings, wordedFormattingLocale } from "../i18n";
import type { InviteLimits } from "../lib/api";

const SECONDS_PER_MINUTE = 60;
const SECONDS_PER_HOUR = 60 * SECONDS_PER_MINUTE;
const SECONDS_PER_DAY = 24 * SECONDS_PER_HOUR;
const MILLISECONDS_PER_SECOND = 1_000;
const MINUTES_PER_HOUR = 60;
const HOURS_PER_DAY = 24;

/** Discord's lifetimes; `null` never expires. The instance accepts up to 30 days. */
export const INVITE_LIFETIME_PRESETS_IN_SECONDS: readonly (number | null)[] = [
  30 * SECONDS_PER_MINUTE,
  SECONDS_PER_HOUR,
  6 * SECONDS_PER_HOUR,
  12 * SECONDS_PER_HOUR,
  SECONDS_PER_DAY,
  7 * SECONDS_PER_DAY,
  30 * SECONDS_PER_DAY,
  null,
];

/** Discord's use limits; `null` is no limit. The instance accepts up to 10 000. */
export const INVITE_MAXIMUM_USES_PRESETS: readonly (number | null)[] = [null, 1, 5, 10, 25, 50, 100];

/** Discord's default: a week, any number of people, so forgotten codes stop working on their own. */
export const DEFAULT_INVITE_LIMITS: InviteLimits = { maxUses: null, expiresInSeconds: 7 * SECONDS_PER_DAY };

const DURATION_UNITS = [
  { unit: "day", seconds: SECONDS_PER_DAY },
  { unit: "hour", seconds: SECONDS_PER_HOUR },
  { unit: "minute", seconds: SECONDS_PER_MINUTE },
  { unit: "second", seconds: 1 },
] as const;

/** "7 days", "1 hour", "30 minutes": the largest unit that counts the duration exactly. */
export function durationLabel(seconds: number, locale: string = wordedFormattingLocale): string {
  const { unit, seconds: unitSeconds } =
    DURATION_UNITS.find((candidate) => seconds % candidate.seconds === 0) ?? DURATION_UNITS[3];
  const label = new Intl.NumberFormat(locale, { style: "unit", unit, unitDisplay: "long" }).format(seconds / unitSeconds);
  // ICU puts a plain space after some numbers ("30 minutes" in French): keep the number with its unit.
  return label.replace(/(\d) /u, "$1\u00a0");
}

export function lifetimeOptionLabel(
  seconds: number | null,
  catalogue: Catalogue = strings,
  locale: string = wordedFormattingLocale,
): string {
  return seconds === null ? catalogue.inviteLimits.never : durationLabel(seconds, locale);
}

export function maximumUsesOptionLabel(maximumUses: number | null, catalogue: Catalogue = strings): string {
  return maximumUses === null ? catalogue.inviteLimits.noLimit : catalogue.inviteLimits.uses(maximumUses);
}

/** "This invite expires in 7 days and has no use limit." */
export function inviteLimitsSentence(
  limits: InviteLimits,
  catalogue: Catalogue = strings,
  locale: string = wordedFormattingLocale,
): string {
  const lifetime = limits.expiresInSeconds === null ? null : durationLabel(limits.expiresInSeconds, locale);
  return catalogue.invitePeople.limits(lifetime, limits.maxUses);
}

/**
 * "Expires in 3 days", "Expires in 5 hours", "Never expires": rounded to the nearest unit, so a
 * week-long invite created a second ago still says 7 days.
 */
export function expiryLabel(
  expiresAtUnixMilliseconds: number | null,
  nowUnixMilliseconds: number,
  catalogue: Catalogue = strings,
  locale: string = wordedFormattingLocale,
): string {
  const labels = catalogue.serverSettings.invites;
  if (expiresAtUnixMilliseconds === null) {
    return labels.neverExpires;
  }
  const remainingSeconds = (expiresAtUnixMilliseconds - nowUnixMilliseconds) / MILLISECONDS_PER_SECOND;
  if (remainingSeconds <= 0) {
    return labels.expired;
  }
  const relative = new Intl.RelativeTimeFormat(locale, { numeric: "always" });
  const minutes = Math.max(1, Math.round(remainingSeconds / SECONDS_PER_MINUTE));
  if (minutes < MINUTES_PER_HOUR) {
    return labels.expiresIn(relative.format(minutes, "minute"));
  }
  const hours = Math.round(remainingSeconds / SECONDS_PER_HOUR);
  if (hours < HOURS_PER_DAY) {
    return labels.expiresIn(relative.format(hours, "hour"));
  }
  return labels.expiresIn(relative.format(Math.round(remainingSeconds / SECONDS_PER_DAY), "day"));
}

/** `AE8C-78A9-K2QF` as its groups; a code from before v0.18.0 is one group. */
export function inviteCodeGroups(code: string): string[] {
  return code.split("-").filter((group) => group !== "");
}

/** "A E 8 C, 7 8 A 9, K 2 Q F": one symbol at a time, a pause between groups, for screen readers. */
export function spokenInviteCode(code: string): string {
  return inviteCodeGroups(code)
    .map((group) => [...group].join(" "))
    .join(", ");
}
