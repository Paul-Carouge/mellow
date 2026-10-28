import { describe, expect, it } from "vitest";

import { en } from "../i18n/en";
import { fr } from "../i18n/fr";
import { MAXIMUM_CHANNEL_NAME_LENGTH, MAXIMUM_CHANNEL_TOPIC_LENGTH } from "../lib/protocol";
import { SERVER_TEMPLATE_IDS, serverTemplates, templateChannelNames } from "./server-templates";

/** What `CreateGuildRequest::initial_categories` may hold (`guild_rules.rs`). */
const MAXIMUM_INITIAL_CATEGORIES = 10;
const MAXIMUM_INITIAL_TEXT_CHANNELS = 50;

describe("server templates", () => {
  for (const [language, catalogue] of [
    ["English", en],
    ["French", fr],
  ] as const) {
    it(`lists every template in ${language}, each within the instance's limits`, () => {
      const templates = serverTemplates(catalogue);
      expect(templates.map((template) => template.id)).toEqual(SERVER_TEMPLATE_IDS);
      for (const template of templates) {
        expect(template.name.trim()).not.toBe("");
        expect(template.categories.length).toBeGreaterThan(0);
        expect(template.categories.length).toBeLessThanOrEqual(MAXIMUM_INITIAL_CATEGORIES);
        const channels = template.categories.flatMap((category) => category.channels);
        expect(channels.length).toBeGreaterThan(0);
        expect(channels.length).toBeLessThanOrEqual(MAXIMUM_INITIAL_TEXT_CHANNELS);
        for (const name of [...template.categories.map((category) => category.name), ...channels.map((channel) => channel.name)]) {
          // Sent as typed: already trimmed, so the preview shows what the instance stores.
          expect(name).toBe(name.trim());
          expect(name.length).toBeGreaterThan(0);
          expect(Array.from(name).length).toBeLessThanOrEqual(MAXIMUM_CHANNEL_NAME_LENGTH);
        }
        for (const channel of channels) {
          expect(Array.from(channel.topic ?? "").length).toBeLessThanOrEqual(MAXIMUM_CHANNEL_TOPIC_LENGTH);
        }
      }
    });
  }

  it("keeps Blank to one category with a general channel, as the instance's own default", () => {
    const blank = serverTemplates(en).find((template) => template.id === "blank");
    expect(blank?.categories).toEqual([{ name: "Text channels", channels: [{ name: "general" }] }]);
    expect(serverTemplates(fr).find((template) => template.id === "blank")?.categories[0]?.channels).toEqual([
      { name: "général" },
    ]);
  });

  it("describes a template by the channels it brings", () => {
    const friends = serverTemplates(en)[0];
    expect(friends ? templateChannelNames(friends) : "").toBe("general, photos, plans");
  });
});
