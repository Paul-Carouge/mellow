import { describe, expect, it } from "vitest";

import { en } from "./en";
import { fr } from "./fr";
import { catalogueFor, chooseLanguage } from "./index";

describe("chooseLanguage", () => {
  it("speaks French when French is the first language the client knows", () => {
    expect(chooseLanguage(["fr-FR", "en-US"])).toEqual({
      language: "fr",
      formattingLocale: "fr-FR",
      wordedFormattingLocale: "fr-FR",
    });
    expect(chooseLanguage(["de-DE", "fr-CA", "en"])).toMatchObject({ language: "fr", formattingLocale: "fr-CA" });
    expect(chooseLanguage(["FR"])).toMatchObject({ language: "fr", formattingLocale: "FR" });
  });

  it("speaks English when English comes first, or when no language is known", () => {
    expect(chooseLanguage(["en-GB", "fr-FR"])).toEqual({
      language: "en",
      formattingLocale: "en-GB",
      wordedFormattingLocale: "en-GB",
    });
    expect(chooseLanguage([])).toEqual({ language: "en", formattingLocale: "en", wordedFormattingLocale: "en" });
  });

  it("keeps the browser's own formats without words when it speaks neither language", () => {
    expect(chooseLanguage(["de-DE", "es"])).toEqual({
      language: "en",
      formattingLocale: "de-DE",
      wordedFormattingLocale: "en",
    });
  });

  it("skips tags that are not valid language tags", () => {
    expect(chooseLanguage(["not a tag", "fr-BE"])).toMatchObject({ language: "fr", formattingLocale: "fr-BE" });
    expect(chooseLanguage(["not a tag"])).toMatchObject({ language: "en", formattingLocale: "en" });
  });

  it("does not read a language that only starts like French", () => {
    expect(chooseLanguage(["fy-NL", "en"]).language).toBe("en");
  });

  it("gives the catalogue of the chosen language", () => {
    expect(catalogueFor("fr")).toBe(fr);
    expect(catalogueFor("en")).toBe(en);
  });
});

describe("catalogues", () => {
  it("count attachments with each language's plural rules", () => {
    expect(en.messages.attachmentCount(0)).toBe("0 attachments");
    expect(en.messages.attachmentCount(1)).toBe("1 attachment");
    expect(en.messages.attachmentCount(2)).toBe("2 attachments");
    // French treats 0 and 1 as singular.
    expect(fr.messages.attachmentCount(0)).toBe("0 pièce jointe");
    expect(fr.messages.attachmentCount(1)).toBe("1 pièce jointe");
    expect(fr.messages.attachmentCount(2)).toBe("2 pièces jointes");
  });

  it("word the unread bar with its count and date", () => {
    expect(en.chat.newMessagesSince(1, false, "today at 14:32")).toBe("1 new message since today at 14:32");
    expect(en.chat.newMessagesSince(50, true, "21/10/2026")).toBe("50+ new messages since 21/10/2026");
    expect(fr.chat.newMessagesSince(1, false, "aujourd’hui à 14:32")).toBe(
      "1 nouveau message depuis aujourd’hui à 14:32",
    );
    expect(fr.chat.newMessagesSince(1, true, "hier à 09:15")).toBe("1+ nouveaux messages depuis hier à 09:15");
  });

  it("group large numbers and sizes the French way", () => {
    expect(fr.members.groupLabel("En ligne", 1204)).toBe("En ligne, 1 204 membres");
    expect(fr.fileSize.inUnit("2,4", "MB")).toBe("2,4 Mo");
    expect(en.fileSize.inUnit("2.4", "MB")).toBe("2.4 MB");
  });

  it("set French punctuation apart with no-break spaces", () => {
    expect(en.errors.withReason("Couldn't sign in", "the login or the password is wrong.")).toBe(
      "Couldn't sign in: the login or the password is wrong.",
    );
    expect(fr.errors.withReason("Impossible de te connecter", fr.errors.reasons.INVALID_CREDENTIALS)).toBe(
      "Impossible de te connecter : l’identifiant ou le mot de passe est incorrect.",
    );
    expect(fr.deleteMessage.title).toBe("Supprimer ce message ?");
    expect(fr.leaveServer.title("Rust Lyon")).toBe("Quitter « Rust Lyon »");
  });

  it("list the people typing", () => {
    expect(en.typing.andOthersAreTyping(1)).toBe(" and 1 other are typing…");
    expect(fr.typing.andOthersAreTyping(2)).toBe(" et 2 autres écrivent…");
  });
});
