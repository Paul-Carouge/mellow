// The composer's "/" menu and formatting shortcuts (DESIGN.md §5.5, docs/design/web-slash-menu.md).
// Every command writes the Markdown that messages render (`markdown.ts`) in the open, never a
// hidden format: the field stays plain text, and the menu shows what it writes so people learn it.

import { foldForSearch } from "../lib/search-text";
import type { IconName } from "./Icon.svelte";

export type SlashCommandId =
  | "heading1"
  | "heading2"
  | "heading3"
  | "bold"
  | "italic"
  | "underline"
  | "strikethrough"
  | "spoiler"
  | "inlineCode"
  | "codeBlock"
  | "quote"
  | "bulletedList"
  | "numberedList"
  | "link"
  | "shrug"
  | "tableflip"
  | "poll";

type SlashCommandEffect =
  /** Starts the line: the spaces and the "/query" in front of it are replaced. */
  | { kind: "linePrefix"; prefix: string }
  /** A pair of marks with the caret between them. */
  | { kind: "wrap"; marks: string }
  | { kind: "codeBlock" }
  /** `[text](https://)` with the text selected, ready to be typed over. */
  | { kind: "link" }
  | { kind: "text"; text: string }
  /** Opens a dialog (a poll, ADR-0043): the "/query" goes, nothing is written. */
  | { kind: "dialog" };

export interface SlashCommand {
  id: SlashCommandId;
  icon: IconName;
  effect: SlashCommandEffect;
  /** What the command writes, shown at the end of its row; empty for a dialog. */
  markup: string;
  /** Words that find it in any language: Markdown's own names ("h1"), short forms. */
  keywords: readonly string[];
}

/**
 * `¯\_(ツ)_/¯` as Markdown: escaped so its backslash shows and its underscores never open an
 * emphasis (a test reads it back through the message parser).
 */
export const SHRUG_MARKDOWN = "¯\\\\\\_(ツ)\\_/¯";
const TABLEFLIP_TEXT = "(╯°□°)╯︵ ┻━┻";
const CODE_FENCE = "```";

/**
 * In the order the menu lists them before anything is typed: headings, inline marks, blocks,
 * extras, then the poll, the one command that opens a dialog instead of writing Markdown.
 */
export const SLASH_COMMANDS: readonly SlashCommand[] = [
  { id: "heading1", icon: "text-h-one", effect: { kind: "linePrefix", prefix: "# " }, markup: "#", keywords: ["h1"] },
  { id: "heading2", icon: "text-h-two", effect: { kind: "linePrefix", prefix: "## " }, markup: "##", keywords: ["h2"] },
  { id: "heading3", icon: "text-h-three", effect: { kind: "linePrefix", prefix: "### " }, markup: "###", keywords: ["h3"] },
  { id: "bold", icon: "text-b", effect: { kind: "wrap", marks: "**" }, markup: "**", keywords: ["strong"] },
  { id: "italic", icon: "text-italic", effect: { kind: "wrap", marks: "*" }, markup: "*", keywords: ["em"] },
  { id: "underline", icon: "text-underline", effect: { kind: "wrap", marks: "__" }, markup: "__", keywords: [] },
  { id: "strikethrough", icon: "text-strikethrough", effect: { kind: "wrap", marks: "~~" }, markup: "~~", keywords: ["strike"] },
  { id: "spoiler", icon: "eye-slash", effect: { kind: "wrap", marks: "||" }, markup: "||", keywords: [] },
  { id: "inlineCode", icon: "code", effect: { kind: "wrap", marks: "`" }, markup: "`", keywords: [] },
  { id: "codeBlock", icon: "code-block", effect: { kind: "codeBlock" }, markup: CODE_FENCE, keywords: [] },
  { id: "quote", icon: "quotes", effect: { kind: "linePrefix", prefix: "> " }, markup: ">", keywords: ["blockquote"] },
  { id: "bulletedList", icon: "list-bullets", effect: { kind: "linePrefix", prefix: "- " }, markup: "-", keywords: ["ul"] },
  { id: "numberedList", icon: "list-numbers", effect: { kind: "linePrefix", prefix: "1. " }, markup: "1.", keywords: ["ol"] },
  { id: "link", icon: "link-simple", effect: { kind: "link" }, markup: "[ ]( )", keywords: ["url"] },
  { id: "shrug", icon: "smiley-meh", effect: { kind: "text", text: SHRUG_MARKDOWN }, markup: "¯\\_(ツ)_/¯", keywords: [] },
  { id: "tableflip", icon: "smiley-angry", effect: { kind: "text", text: TABLEFLIP_TEXT }, markup: TABLEFLIP_TEXT, keywords: [] },
  { id: "poll", icon: "chart-bar-horizontal", effect: { kind: "dialog" }, markup: "", keywords: ["vote"] },
];

// --- Finding the "/co" being typed -----------------------------------------------------------

/** The "/" and the word after it, up to the caret: what a chosen command replaces. */
export interface SlashCommandQuery {
  start: number;
  end: number;
  query: string;
}

/** Longer than any command's name in any language: past it, the "/" is text. */
const MAXIMUM_SLASH_QUERY_LENGTH = 32;
const QUERY_CHARACTER = /[\p{L}\p{N}]/u;

/**
 * The "/co" ending at `caret`, when its "/" starts its line (only spaces before it): a "/" after
 * text, inside a word or in "https://" is text. The replaced range ends at the caret, so a "/"
 * typed in front of existing words never eats them.
 */
export function slashCommandQuery(text: string, caret: number): SlashCommandQuery | null {
  let start = caret;
  while (start > 0 && caret - start <= MAXIMUM_SLASH_QUERY_LENGTH && QUERY_CHARACTER.test(text[start - 1] ?? "")) {
    start -= 1;
  }
  const slash = start - 1;
  if (text[slash] !== "/" || caret - start > MAXIMUM_SLASH_QUERY_LENGTH) {
    return null;
  }
  const lineStart = lineStartOf(text, slash);
  if (text.slice(lineStart, slash).trim() !== "" || isInsideCodeFence(text, lineStart)) {
    return null;
  }
  return { start: slash, end: caret, query: text.slice(start, caret) };
}

/** Whether the line starting at `lineStart` is inside a fenced code block: an odd number of fence lines before it. */
function isInsideCodeFence(text: string, lineStart: number): boolean {
  const fences = text
    .slice(0, lineStart)
    .split("\n")
    .filter((line) => line.trimStart().startsWith("```")).length;
  return fences % 2 === 1;
}

function lineStartOf(text: string, position: number): number {
  return position === 0 ? 0 : text.lastIndexOf("\n", position - 1) + 1;
}

// --- Choosing among them ---------------------------------------------------------------------

/** A command with its words in the reader's language and the folded names it is found by. */
export interface NamedSlashCommand {
  command: SlashCommand;
  name: string;
  description: string;
  /** The reader's name first, then English names and keywords; folded, also without spaces. */
  foldedNames: readonly string[];
  foldedNameWords: readonly string[];
}

type CommandWords = Readonly<Record<SlashCommandId, { name: string; description: string }>>;

/** Built once per language: `words` are the reader's, `englishWords` are aliases in any language. */
export function namedSlashCommands(words: CommandWords, englishWords: CommandWords): NamedSlashCommand[] {
  return SLASH_COMMANDS.map((command) => {
    const names = [words[command.id].name, englishWords[command.id].name, ...command.keywords].map(foldForSearch);
    return {
      command,
      name: words[command.id].name,
      description: words[command.id].description,
      foldedNames: [...new Set(names.flatMap((name) => [name, name.replaceAll(" ", "")]))],
      foldedNameWords: names.flatMap((name) => name.split(" ")),
    };
  });
}

/**
 * The commands `query` names, best first: the reader's name starting with it, then another name
 * ("/bold" in French), then a word inside a name ("/num" for "Liste numérotée"); ties keep the
 * menu's order. Case and accents are ignored; an empty query lists them all.
 */
export function suggestSlashCommands(query: string, commands: readonly NamedSlashCommand[]): NamedSlashCommand[] {
  const foldedQuery = foldForSearch(query);
  const ranked: { named: NamedSlashCommand; rank: number }[] = [];
  for (const named of commands) {
    const rank = slashCommandMatchRank(named, foldedQuery);
    if (rank !== null) {
      ranked.push({ named, rank });
    }
  }
  // `sort` is stable: equal ranks stay in the menu's order.
  return ranked.sort((left, right) => left.rank - right.rank).map((entry) => entry.named);
}

function slashCommandMatchRank(named: NamedSlashCommand, foldedQuery: string): number | null {
  const [readersName, readersNameWithoutSpaces] = named.foldedNames;
  if (readersName?.startsWith(foldedQuery) || readersNameWithoutSpaces?.startsWith(foldedQuery)) {
    return 0;
  }
  if (named.foldedNames.some((name) => name.startsWith(foldedQuery))) {
    return 1;
  }
  return named.foldedNameWords.some((word) => word.startsWith(foldedQuery)) ? 2 : null;
}

// --- Writing them ----------------------------------------------------------------------------

/** The field's text and selection after a command. */
export interface TextEdit {
  text: string;
  selectionStart: number;
  selectionEnd: number;
}

/**
 * `text` with the "/query" in `range` replaced by `command`'s Markdown. `linkTextPlaceholder` is
 * the selected word of a new link ("texte"), in the reader's language.
 */
export function applySlashCommand(
  text: string,
  range: { start: number; end: number },
  command: SlashCommand,
  linkTextPlaceholder: string,
): TextEdit {
  const lineStart = lineStartOf(text, range.start);
  const before = text.slice(0, range.start);
  const after = text.slice(range.end);
  const effect = command.effect;
  switch (effect.kind) {
    case "linePrefix":
      return caretAfter(text.slice(0, lineStart), effect.prefix, after);
    case "wrap":
      return caretAfter(before, effect.marks, effect.marks + after);
    case "codeBlock": {
      // Words after the "/" on its line go after the block: a fence must end its line.
      const closingFence = after === "" || after.startsWith("\n") ? `\n${CODE_FENCE}` : `\n${CODE_FENCE}\n`;
      return caretAfter(text.slice(0, lineStart), `${CODE_FENCE}\n`, closingFence + after);
    }
    case "link": {
      const selectionStart = before.length + 1;
      return {
        text: `${before}[${linkTextPlaceholder}](https://)${after}`,
        selectionStart,
        selectionEnd: selectionStart + linkTextPlaceholder.length,
      };
    }
    case "text": {
      const inserted = after === "" || /^\s/u.test(after) ? effect.text : `${effect.text} `;
      return caretAfter(before, inserted, after);
    }
    case "dialog":
      return caretAfter(before, "", after);
  }
}

function caretAfter(before: string, inserted: string, after: string): TextEdit {
  const caret = before.length + inserted.length;
  return { text: before + inserted + after, selectionStart: caret, selectionEnd: caret };
}

/** The marks Ctrl+B, Ctrl+I, Ctrl+U and Ctrl+Shift+X put around a selection (Cmd on macOS). */
export function formattingMarksForShortcut(event: {
  readonly key: string;
  readonly ctrlKey: boolean;
  readonly metaKey: boolean;
  readonly shiftKey: boolean;
  readonly altKey: boolean;
}): string | null {
  if (!(event.ctrlKey || event.metaKey) || event.altKey) {
    return null;
  }
  const key = event.key.toLowerCase();
  if (event.shiftKey) {
    return key === "x" ? "~~" : null;
  }
  return key === "b" ? "**" : key === "i" ? "*" : key === "u" ? "__" : null;
}

/**
 * Puts `marks` around the selection, or takes them off when they are already there (around it or
 * at its ends), as Ctrl+B does in a word processor. Spaces at the selection's edges stay outside
 * the marks, where Markdown needs them; an empty selection gets an empty pair with the caret inside.
 */
export function toggleInlineMarks(text: string, selectionStart: number, selectionEnd: number, marks: string): TextEdit {
  if (text.slice(selectionStart, selectionEnd).trim().includes("\n")) {
    return toggleMarksOnEachLine(text, selectionStart, selectionEnd, marks);
  }
  let start = selectionStart;
  let end = selectionEnd;
  while (start < end && /\s/u.test(text[start] ?? "")) {
    start += 1;
  }
  while (end > start && /\s/u.test(text[end - 1] ?? "")) {
    end -= 1;
  }
  const length = marks.length;
  if (hasMarks(text, start, end, marks)) {
    return {
      text: text.slice(0, start - length) + text.slice(start, end) + text.slice(end + length),
      selectionStart: start - length,
      selectionEnd: end - length,
    };
  }
  const selected = text.slice(start, end);
  if (selected.length >= 2 * length && hasMarksAtEnds(selected, marks)) {
    const inner = selected.slice(length, selected.length - length);
    return { text: text.slice(0, start) + inner + text.slice(end), selectionStart: start, selectionEnd: start + inner.length };
  }
  return {
    text: text.slice(0, start) + marks + selected + marks + text.slice(end),
    selectionStart: start + length,
    selectionEnd: end + length,
  };
}

/**
 * Marks across several lines would break their blocks ("**# Plan" is no heading): each line's
 * text after its block mark ("# ", "> ", "- ", "1. ") is toggled on its own; blank lines stay.
 */
function toggleMarksOnEachLine(text: string, selectionStart: number, selectionEnd: number, marks: string): TextEdit {
  const lines = text.slice(selectionStart, selectionEnd).split("\n");
  const toggled = lines.map((line) => {
    const prefix = /^\s*(?:#{1,3} |> |- |\d+\. )?/u.exec(line)?.[0] ?? "";
    const content = line.slice(prefix.length);
    if (content.trim() === "") {
      return line;
    }
    return prefix + toggleInlineMarks(content, 0, content.length, marks).text;
  });
  const replaced = toggled.join("\n");
  return {
    text: text.slice(0, selectionStart) + replaced + text.slice(selectionEnd),
    selectionStart,
    selectionEnd: selectionStart + replaced.length,
  };
}

/** Whether `selected` starts and ends with `marks`, whole runs of the mark's character counted. */
function hasMarksAtEnds(selected: string, marks: string): boolean {
  const markCharacter = marks[0] ?? "";
  let runAtStart = 0;
  while (runAtStart < selected.length && selected[runAtStart] === markCharacter) {
    runAtStart += 1;
  }
  let runAtEnd = 0;
  while (runAtEnd < selected.length - runAtStart && selected[selected.length - 1 - runAtEnd] === markCharacter) {
    runAtEnd += 1;
  }
  return isMarkRun(Math.min(runAtStart, runAtEnd), marks);
}

function isMarkRun(run: number, marks: string): boolean {
  return marks.length === 1 ? run % 2 === 1 : run >= marks.length;
}

/**
 * Whether `marks` sit right before `start` and right after `end`. A run of the mark's character
 * counts: "*" is there when the run is odd ("*a*", "***a***"), "**" when it holds at least two.
 */
function hasMarks(text: string, start: number, end: number, marks: string): boolean {
  const markCharacter = marks[0] ?? "";
  let runBefore = 0;
  while (start - runBefore > 0 && text[start - runBefore - 1] === markCharacter) {
    runBefore += 1;
  }
  let runAfter = 0;
  while (end + runAfter < text.length && text[end + runAfter] === markCharacter) {
    runAfter += 1;
  }
  return isMarkRun(Math.min(runBefore, runAfter), marks);
}

/**
 * The part of `before` that differs from `after`: replacing `before`'s `start`–`end` with
 * `inserted` gives `after`. The field writes only that part, so the browser's undo has one small step.
 */
export function differingRange(before: string, after: string): { start: number; end: number; inserted: string } {
  let prefix = 0;
  while (prefix < before.length && prefix < after.length && before[prefix] === after[prefix]) {
    prefix += 1;
  }
  let suffix = 0;
  while (
    suffix < before.length - prefix &&
    suffix < after.length - prefix &&
    before[before.length - 1 - suffix] === after[after.length - 1 - suffix]
  ) {
    suffix += 1;
  }
  return { start: prefix, end: before.length - suffix, inserted: after.slice(prefix, after.length - suffix) };
}
