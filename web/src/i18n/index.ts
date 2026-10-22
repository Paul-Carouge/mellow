// The interface language, chosen once from the browser's languages (ADR-0022 §7), and its catalogue.

import { type Catalogue, en } from "./en";
import { fr } from "./fr";

export type { Catalogue };
export type Language = "en" | "fr";

export interface LanguageChoice {
  language: Language;
  /** The browser's own tag for that language ("fr-CA", "en-GB"), so `Intl` keeps its regional formats. */
  formattingLocale: string;
}

const CATALOGUES: Record<Language, Catalogue> = { en, fr };

/** The first of the reader's languages the client speaks, in their order; English when none is. */
export function chooseLanguage(preferredLanguages: readonly string[]): LanguageChoice {
  for (const tag of preferredLanguages) {
    const primary = tag.split("-")[0]?.toLowerCase();
    if (primary === "fr" || primary === "en") {
      return { language: primary, formattingLocale: tag };
    }
  }
  return { language: "en", formattingLocale: "en" };
}

export function catalogueFor(language: Language): Catalogue {
  return CATALOGUES[language];
}

export const { language, formattingLocale } = chooseLanguage(globalThis.navigator?.languages ?? []);
/** Every string the client shows or announces, in the reader's language. */
export const strings: Catalogue = catalogueFor(language);
