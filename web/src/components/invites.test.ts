import { describe, expect, it } from "vitest";

import { en } from "../i18n/en";
import { fr } from "../i18n/fr";
import {
  DEFAULT_INVITE_LIMITS,
  INVITE_LIFETIME_PRESETS_IN_SECONDS,
  INVITE_MAXIMUM_USES_PRESETS,
  durationLabel,
  expiryLabel,
  inviteCodeGroups,
  inviteLimitsSentence,
  lifetimeOptionLabel,
  maximumUsesOptionLabel,
  spokenInviteCode,
} from "./invites";

/** `MAXIMUM_INVITE_LIFETIME` and `MAXIMUM_INVITE_USES` in `guild_rules.rs`. */
const MAXIMUM_LIFETIME_SECONDS = 30 * 24 * 60 * 60;
const MAXIMUM_USES = 10_000;
const NOW = Date.UTC(2026, 9, 30, 12);
const HOUR = 60 * 60 * 1_000;
const DAY = 24 * HOUR;

describe("invite presets", () => {
  it("offers Discord's lifetimes and use limits, all within the instance's limits", () => {
    expect(INVITE_LIFETIME_PRESETS_IN_SECONDS.map((seconds) => lifetimeOptionLabel(seconds, en, "en"))).toEqual([
      "30\u00a0minutes",
      "1\u00a0hour",
      "6\u00a0hours",
      "12\u00a0hours",
      "1\u00a0day",
      "7\u00a0days",
      "30\u00a0days",
      "Never",
    ]);
    expect(INVITE_MAXIMUM_USES_PRESETS.map((uses) => maximumUsesOptionLabel(uses, en))).toEqual([
      "No limit",
      "1 use",
      "5 uses",
      "10 uses",
      "25 uses",
      "50 uses",
      "100 uses",
    ]);
    for (const seconds of INVITE_LIFETIME_PRESETS_IN_SECONDS) {
      if (seconds !== null) {
        expect(seconds).toBeGreaterThan(0);
        expect(seconds).toBeLessThanOrEqual(MAXIMUM_LIFETIME_SECONDS);
      }
    }
    for (const uses of INVITE_MAXIMUM_USES_PRESETS) {
      if (uses !== null) {
        expect(uses).toBeGreaterThanOrEqual(1);
        expect(uses).toBeLessThanOrEqual(MAXIMUM_USES);
      }
    }
  });

  it("defaults to a week with no use limit, and the default is one of the presets", () => {
    expect(DEFAULT_INVITE_LIMITS).toEqual({ maxUses: null, expiresInSeconds: 7 * 24 * 60 * 60 });
    expect(INVITE_LIFETIME_PRESETS_IN_SECONDS).toContain(DEFAULT_INVITE_LIMITS.expiresInSeconds);
    expect(INVITE_MAXIMUM_USES_PRESETS).toContain(DEFAULT_INVITE_LIMITS.maxUses);
  });

  it("names the presets in French", () => {
    expect(lifetimeOptionLabel(30 * 60, fr, "fr")).toBe("30\u00a0minutes");
    expect(lifetimeOptionLabel(60 * 60, fr, "fr")).toBe("1\u00a0heure");
    expect(lifetimeOptionLabel(7 * 24 * 60 * 60, fr, "fr")).toBe("7\u00a0jours");
    expect(lifetimeOptionLabel(null, fr, "fr")).toBe("Jamais");
    expect(maximumUsesOptionLabel(null, fr)).toBe("Sans limite");
    expect(maximumUsesOptionLabel(25, fr)).toBe("25 utilisations");
  });

  it("counts a duration in the largest unit that fits exactly", () => {
    expect(durationLabel(90 * 60, "en")).toBe("90\u00a0minutes");
    expect(durationLabel(36 * 60 * 60, "en")).toBe("36\u00a0hours");
    expect(durationLabel(45, "en")).toBe("45\u00a0seconds");
  });
});

describe("invite limits sentence", () => {
  it("says how long the invite lasts and how many people it lets in", () => {
    expect(inviteLimitsSentence(DEFAULT_INVITE_LIMITS, en, "en")).toBe("This invite expires in 7\u00a0days and has no use limit.");
    expect(inviteLimitsSentence({ expiresInSeconds: null, maxUses: 1 }, en, "en")).toBe(
      "This invite never expires and can be used once.",
    );
    expect(inviteLimitsSentence({ expiresInSeconds: 30 * 60, maxUses: 10 }, en, "en")).toBe(
      "This invite expires in 30\u00a0minutes and can be used 10 times.",
    );
  });

  it("says it in French", () => {
    expect(inviteLimitsSentence(DEFAULT_INVITE_LIMITS, fr, "fr")).toBe(
      "Cette invitation expire dans 7\u00a0jours et n’a pas de limite d’utilisations.",
    );
    expect(inviteLimitsSentence({ expiresInSeconds: null, maxUses: 1 }, fr, "fr")).toBe(
      "Cette invitation n’expire jamais et ne peut servir qu’une fois.",
    );
    expect(inviteLimitsSentence({ expiresInSeconds: 60 * 60, maxUses: 5 }, fr, "fr")).toBe(
      "Cette invitation expire dans 1\u00a0heure et peut servir 5 fois.",
    );
  });
});

describe("expiry label", () => {
  it("says never for an invite without a lifetime", () => {
    expect(expiryLabel(null, NOW, en, "en")).toBe("Never expires");
    expect(expiryLabel(null, NOW, fr, "fr")).toBe("N’expire jamais");
  });

  it("rounds to the nearest unit, so a fresh week-long invite says 7 days", () => {
    expect(expiryLabel(NOW + 7 * DAY - 1_000, NOW, en, "en")).toBe("Expires in 7 days");
    expect(expiryLabel(NOW + 3 * DAY + 2 * HOUR, NOW, en, "en")).toBe("Expires in 3 days");
    expect(expiryLabel(NOW + 23 * HOUR + 50 * 60 * 1_000, NOW, en, "en")).toBe("Expires in 1 day");
    expect(expiryLabel(NOW + 5 * HOUR, NOW, en, "en")).toBe("Expires in 5 hours");
    expect(expiryLabel(NOW + 59 * 60 * 1_000 + 40_000, NOW, en, "en")).toBe("Expires in 1 hour");
    expect(expiryLabel(NOW + 12 * 60 * 1_000, NOW, en, "en")).toBe("Expires in 12 minutes");
    expect(expiryLabel(NOW + 10_000, NOW, en, "en")).toBe("Expires in 1 minute");
    expect(expiryLabel(NOW + 3 * DAY, NOW, fr, "fr")).toBe("Expire dans 3 jours");
  });

  it("says expired once the time has passed, as a list left open can show", () => {
    expect(expiryLabel(NOW, NOW, en, "en")).toBe("Expired");
    expect(expiryLabel(NOW - HOUR, NOW, fr, "fr")).toBe("Expirée");
  });
});

describe("uses label", () => {
  it("shows the uses out of the maximum, or the uses alone without a limit", () => {
    expect(en.serverSettings.invites.uses(23, 100)).toBe("23 / 100 uses");
    expect(en.serverSettings.invites.uses(1, null)).toBe("1 use");
    expect(en.serverSettings.invites.uses(1_204, null)).toBe("1,204 uses");
    expect(fr.serverSettings.invites.uses(23, 100)).toBe("23 / 100 utilisations");
  });
});

describe("invite code display", () => {
  it("splits a readable code into its groups", () => {
    expect(inviteCodeGroups("AE8C-78A9-K2QF")).toEqual(["AE8C", "78A9", "K2QF"]);
    // Codes from before v0.18.0 have no dash: one group, case kept.
    expect(inviteCodeGroups("aB3dE5gH7k")).toEqual(["aB3dE5gH7k"]);
  });

  it("spells a code out for screen readers, pausing between groups", () => {
    expect(spokenInviteCode("AE8C-78A9-K2QF")).toBe("A E 8 C, 7 8 A 9, K 2 Q F");
    expect(spokenInviteCode("aB3")).toBe("a B 3");
  });
});
