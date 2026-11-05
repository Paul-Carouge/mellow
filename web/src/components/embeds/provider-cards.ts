import type { GitHubIssueState, GitHubRepositoryEmbed } from "../../lib/protocol";
import type { IconName } from "../Icon.svelte";
import { linkPreviewBarColor } from "../link-previews";
import { LINGUIST_LANGUAGE_COLORS } from "./linguist-colors";

/** A repository card shows the first five topics; the page has them all. */
export const MAXIMUM_SHOWN_TOPICS = 5;

/** "104K", « 104 k »: a count at a glance; the exact one goes to screen readers and the tooltip. */
export function compactCount(value: number, locale: string): string {
  // Three digits at most: "104K", "13.6K", "1.23K".
  return new Intl.NumberFormat(locale, {
    notation: "compact",
    maximumSignificantDigits: 3,
  }).format(value);
}

/** The dot's `#rrggbb` beside the language's name, or null for the token's neutral dot. */
export function languageColorOf(embed: GitHubRepositoryEmbed): string | null {
  if (embed.language === undefined) {
    return null;
  }
  // Own keys only: a language named "constructor" must not read the prototype.
  const tableColor = Object.hasOwn(LINGUIST_LANGUAGE_COLORS, embed.language) ? LINGUIST_LANGUAGE_COLORS[embed.language] : undefined;
  return tableColor ?? linkPreviewBarColor(embed.language_color);
}

/** How a state's badge looks: draft is the only outlined one, and each state has its own glyph. */
export type IssueStateTone = "open" | "draft" | "merged" | "completed" | "closed";

export function issueStateBadge(state: GitHubIssueState, isPullRequest: boolean): { icon: IconName; tone: IssueStateTone } {
  switch (state) {
    case "open":
      return { icon: isPullRequest ? "git-pull-request" : "record", tone: "open" };
    case "draft":
      return { icon: "git-pull-request", tone: "draft" };
    case "merged":
      return { icon: "git-merge", tone: "merged" };
    case "completed":
      return { icon: "check-circle", tone: "completed" };
    case "not_planned":
    case "closed":
      return { icon: "x-circle", tone: "closed" };
  }
}

/**
 * The wiki's language as the interface names it ("French", « anglais »), or null when it is the
 * interface's own language, so the card says plainly "Wikipedia".
 */
export function wikipediaLanguageName(wikiLanguage: string, interfaceLanguage: string): string | null {
  if (wikiLanguage === interfaceLanguage) {
    return null;
  }
  try {
    return new Intl.DisplayNames([interfaceLanguage], { type: "language", fallback: "code" }).of(wikiLanguage) ?? wikiLanguage;
  } catch {
    // A RangeError: a wiki code that is no language tag; the code itself names it.
    return wikiLanguage;
  }
}
