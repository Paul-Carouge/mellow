// The interface language, chosen once from the browser's languages (ADR-0022 §7), and its catalogue.

import { type Catalogue, en } from "./en";
import { fr } from "./fr";

export type { Catalogue };
export type Language = "en" | "fr";

export interface LanguageChoice {
  language: Language;
  /**
   * For formats without words (times, numeric dates, numbers): the browser's tag for that language
   * ("fr-CA", "en-GB"), or its first language when the client speaks none of them ("de-DE").
   */
  formattingLocale: string;
  /** For formats with words (weekday and month names): always a tag of `language`. */
  wordedFormattingLocale: string;
}

const CATALOGUES: Record<Language, Catalogue> = { en, fr };

/** The first of the reader's languages the client speaks, in their order; English when none is. */
export function chooseLanguage(preferredLanguages: readonly string[]): LanguageChoice {
  const usableTags = preferredLanguages.filter(isUsableLocale);
  for (const tag of usableTags) {
    const primary = tag.split("-")[0]?.toLowerCase();
    if (primary === "fr" || primary === "en") {
      return { language: primary, formattingLocale: tag, wordedFormattingLocale: tag };
    }
  }
  return { language: "en", formattingLocale: usableTags[0] ?? "en", wordedFormattingLocale: "en" };
}

/** `Intl` throws on a malformed tag; the browser's list is not trusted to hold only valid ones. */
function isUsableLocale(tag: string): boolean {
  try {
    return Intl.getCanonicalLocales(tag).length > 0;
  } catch {
    // A RangeError: not a valid BCP 47 language tag, so it is skipped.
    return false;
  }
}

export function catalogueFor(language: Language): Catalogue {
  return CATALOGUES[language];
}

export const { language, formattingLocale, wordedFormattingLocale } = chooseLanguage(globalThis.navigator?.languages ?? []);
/** Every string the client shows or announces, in the reader's language. */
export const strings: Catalogue = catalogueFor(language);
