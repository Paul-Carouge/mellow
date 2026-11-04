import { describe, expect, it } from "vitest";

import { en } from "../i18n/en";
import { fr } from "../i18n/fr";
import { parseMessageMarkdown } from "./markdown";
import {
  SHRUG_MARKDOWN,
  SLASH_COMMANDS,
  type SlashCommandId,
  type TextEdit,
  applySlashCommand,
  differingRange,
  formattingMarksForShortcut,
  namedSlashCommands,
  slashCommandQuery,
  suggestSlashCommands,
  toggleInlineMarks,
} from "./slash-commands";

const FRENCH = namedSlashCommands(fr.chat.slashCommands, en.chat.slashCommands);
const ENGLISH = namedSlashCommands(en.chat.slashCommands, en.chat.slashCommands);

function idsFor(query: string, commands = FRENCH): SlashCommandId[] {
  return suggestSlashCommands(query, commands).map((named) => named.command.id);
}

function commandWithId(id: SlashCommandId) {
  const command = SLASH_COMMANDS.find((candidate) => candidate.id === id);
  if (!command) {
    throw new Error(`no command ${id}`);
  }
  return command;
}

/** Applies `id` to `text` where "|" marks the caret after the typed "/query". */
function apply(id: SlashCommandId, textWithCaret: string): TextEdit {
  const caret = textWithCaret.indexOf("|");
  const text = textWithCaret.replace("|", "");
  const query = slashCommandQuery(text, caret);
  if (!query) {
    throw new Error(`no "/" query in ${textWithCaret}`);
  }
  return applySlashCommand(text, query, commandWithId(id), "texte");
}

/** The edit's text with "[" and "]" around its selection, or "|" at a collapsed caret. */
function shown(edit: TextEdit): string {
  const { text, selectionStart, selectionEnd } = edit;
  return selectionStart === selectionEnd
    ? `${text.slice(0, selectionStart)}|${text.slice(selectionStart)}`
    : `${text.slice(0, selectionStart)}[${text.slice(selectionStart, selectionEnd)}]${text.slice(selectionEnd)}`;
}

describe("slashCommandQuery", () => {
  it("opens on a slash at the start of the message or of a line, with or without a word after it", () => {
    expect(slashCommandQuery("/", 1)).toEqual({ start: 0, end: 1, query: "" });
    expect(slashCommandQuery("/co", 3)).toEqual({ start: 0, end: 3, query: "co" });
    expect(slashCommandQuery("Salut\n/gras", 11)).toEqual({ start: 6, end: 11, query: "gras" });
    expect(slashCommandQuery("Salut\n   /tit", 13)).toEqual({ start: 9, end: 13, query: "tit" });
    expect(slashCommandQuery("/souligné", 9)).toEqual({ start: 0, end: 9, query: "souligné" });
  });

  it("never opens on a slash after text on its line, inside a word or in an address", () => {
    expect(slashCommandQuery("et/ou", 3)).toBeNull();
    expect(slashCommandQuery("Salut /gras", 11)).toBeNull();
    expect(slashCommandQuery("https://", 8)).toBeNull();
    expect(slashCommandQuery("https://ex", 10)).toBeNull();
    expect(slashCommandQuery("/usr/bin", 8)).toBeNull();
    expect(slashCommandQuery("/usr/", 5)).toBeNull();
    expect(slashCommandQuery("# /gras", 7)).toBeNull();
  });

  it("closes once a space ends the word, or when the caret is not after the slash", () => {
    expect(slashCommandQuery("/gras ", 6)).toBeNull();
    expect(slashCommandQuery("bonjour", 7)).toBeNull();
    expect(slashCommandQuery("", 0)).toBeNull();
  });

  it("replaces up to the caret only, so words after it stay", () => {
    expect(slashCommandQuery("/bonjour", 1)).toEqual({ start: 0, end: 1, query: "" });
  });

  it("gives up past a command name's length", () => {
    const long = `/${"a".repeat(40)}`;
    expect(slashCommandQuery(long, long.length)).toBeNull();
  });
});

describe("suggestSlashCommands", () => {
  it("lists every command in the menu's order before anything is typed", () => {
    expect(idsFor("")).toEqual(SLASH_COMMANDS.map((command) => command.id));
  });

  it("finds commands by the reader's name, ignoring case and accents", () => {
    expect(idsFor("co")).toEqual(["inlineCode", "codeBlock"]);
    expect(idsFor("GRAS")).toEqual(["bold"]);
    expect(idsFor("souligne")).toEqual(["underline"]);
    expect(idsFor("barre")).toEqual(["strikethrough"]);
    expect(idsFor("titre")).toEqual(["heading1", "heading2", "heading3"]);
    expect(idsFor("titre2")).toEqual(["heading2"]);
  });

  it("also finds them by English names and Markdown's words, after the reader's own names", () => {
    expect(idsFor("bold")).toEqual(["bold"]);
    expect(idsFor("h2")).toEqual(["heading2"]);
    expect(idsFor("quote")).toEqual(["quote"]);
    // All three French names start with it: the menu's order decides.
    expect(idsFor("li")).toEqual(["bulletedList", "numberedList", "link"]);
    expect(idsFor("lie")).toEqual(["link"]);
  });

  it("finds a word inside a name last", () => {
    expect(idsFor("num")).toEqual(["numberedList"]);
    expect(idsFor("puce")).toEqual(["bulletedList"]);
    // Markdown's "blockquote" starts with it; "Code block" only holds it as a word.
    expect(idsFor("block")).toEqual(["quote", "codeBlock"]);
  });

  it("finds nothing for words that name no command", () => {
    expect(idsFor("usr")).toEqual([]);
    expect(idsFor("zz")).toEqual([]);
  });

  it("speaks the reader's language", () => {
    expect(idsFor("bo", ENGLISH)).toEqual(["bold"]);
    expect(suggestSlashCommands("gr", FRENCH)[0]?.name).toBe("Gras");
    expect(suggestSlashCommands("bo", FRENCH)[0]?.description).toBe("Mise en avant forte");
  });
});

describe("applySlashCommand", () => {
  it("starts the line for headings, quotes and lists", () => {
    expect(shown(apply("heading1", "/tit|"))).toBe("# |");
    expect(shown(apply("heading2", "Salut\n/titre2|"))).toBe("Salut\n## |");
    expect(shown(apply("heading3", "/|bonjour"))).toBe("### |bonjour");
    expect(shown(apply("quote", "  /cit|"))).toBe("> |");
    expect(shown(apply("bulletedList", "a\n/li|\nb"))).toBe("a\n- |\nb");
    expect(shown(apply("numberedList", "/num|"))).toBe("1. |");
  });

  it("writes an empty pair of marks with the caret inside", () => {
    expect(shown(apply("bold", "/gras|"))).toBe("**|**");
    expect(shown(apply("italic", "/it|"))).toBe("*|*");
    expect(shown(apply("underline", "/sou|"))).toBe("__|__");
    expect(shown(apply("strikethrough", "/barr|"))).toBe("~~|~~");
    expect(shown(apply("spoiler", "/spo|"))).toBe("|||||");
    expect(shown(apply("inlineCode", "Salut\n/code|"))).toBe("Salut\n`|`");
  });

  it("opens a code block with the caret on its empty line", () => {
    expect(shown(apply("codeBlock", "/bloc|"))).toBe("```\n|\n```");
    expect(shown(apply("codeBlock", "Regarde :\n/bloc|\nmerci"))).toBe("Regarde :\n```\n|\n```\nmerci");
    expect(shown(apply("codeBlock", "/|reste"))).toBe("```\n|\n```\nreste");
  });

  it("writes a link with its words selected, ready to be typed over", () => {
    expect(shown(apply("link", "/lien|"))).toBe("[[texte]](https://)");
  });

  it("writes the shrug and the table flip in place of the command", () => {
    expect(shown(apply("shrug", "/shrug|"))).toBe(`${SHRUG_MARKDOWN}|`);
    expect(shown(apply("tableflip", "/table|bonjour"))).toBe("(╯°□°)╯︵ ┻━┻ |bonjour");
  });

  it("writes Markdown that messages show as meant", () => {
    const shrug = parseMessageMarkdown(SHRUG_MARKDOWN, new Map());
    expect(shrug).toEqual([{ kind: "paragraph", children: expect.any(Array) }]);
    const children = shrug[0]?.kind === "paragraph" ? shrug[0].children : [];
    expect(children.every((child) => child.kind === "text")).toBe(true);
    expect(children.map((child) => (child.kind === "text" ? child.text : "")).join("")).toBe("¯\\_(ツ)_/¯");

    const heading = apply("heading1", "/tit|");
    expect(parseMessageMarkdown(`${heading.text}Bienvenue`, new Map())[0]?.kind).toBe("heading");
    const block = apply("codeBlock", "/bloc|");
    const code = `${block.text.slice(0, block.selectionStart)}let a = 1;${block.text.slice(block.selectionStart)}`;
    expect(parseMessageMarkdown(code, new Map())).toEqual([{ kind: "codeBlock", language: null, text: "let a = 1;" }]);
  });

  it("shows each command's own Markdown in its row", () => {
    for (const command of SLASH_COMMANDS) {
      expect(command.markup).not.toBe("");
    }
  });
});

describe("formattingMarksForShortcut", () => {
  const key = (value: string, modifiers: Partial<Record<"ctrlKey" | "metaKey" | "shiftKey" | "altKey", boolean>> = {}) => ({
    key: value,
    ctrlKey: false,
    metaKey: false,
    shiftKey: false,
    altKey: false,
    ...modifiers,
  });

  it("maps Ctrl or Cmd with B, I, U and Shift+X to their marks", () => {
    expect(formattingMarksForShortcut(key("b", { ctrlKey: true }))).toBe("**");
    expect(formattingMarksForShortcut(key("i", { metaKey: true }))).toBe("*");
    expect(formattingMarksForShortcut(key("u", { ctrlKey: true }))).toBe("__");
    expect(formattingMarksForShortcut(key("X", { ctrlKey: true, shiftKey: true }))).toBe("~~");
  });

  it("leaves every other key to the browser and the app", () => {
    expect(formattingMarksForShortcut(key("b"))).toBeNull();
    expect(formattingMarksForShortcut(key("x", { ctrlKey: true }))).toBeNull();
    expect(formattingMarksForShortcut(key("b", { ctrlKey: true, shiftKey: true }))).toBeNull();
    expect(formattingMarksForShortcut(key("b", { ctrlKey: true, altKey: true }))).toBeNull();
    expect(formattingMarksForShortcut(key("e", { ctrlKey: true }))).toBeNull();
  });
});

describe("toggleInlineMarks", () => {
  /** Toggles `marks` on `textWithSelection`, whose selection is between "[" and "]". */
  function toggle(textWithSelection: string, marks: string): string {
    const start = textWithSelection.indexOf("[");
    const end = textWithSelection.indexOf("]") - 1;
    return shown(toggleInlineMarks(textWithSelection.replace("[", "").replace("]", ""), start, end, marks));
  }

  it("wraps the selection and keeps it selected", () => {
    expect(toggle("un [mot] ici", "**")).toBe("un **[mot]** ici");
    expect(toggle("[mot]", "~~")).toBe("~~[mot]~~");
  });

  it("writes an empty pair at a caret", () => {
    expect(toggle("un []", "**")).toBe("un **|**");
  });

  it("leaves spaces at the selection's edges outside the marks", () => {
    expect(toggle("un[ mot ]ici", "*")).toBe("un *[mot]* ici");
  });

  it("takes marks off when they are around the selection or at its ends", () => {
    expect(toggle("un **[mot]** ici", "**")).toBe("un [mot] ici");
    expect(toggle("un [**mot**] ici", "**")).toBe("un [mot] ici");
    expect(toggle("*[mot]*", "*")).toBe("[mot]");
  });

  it("tells italic from bold in a run of stars", () => {
    expect(toggle("**[mot]**", "*")).toBe("***[mot]***");
    expect(toggle("***[mot]***", "*")).toBe("**[mot]**");
    expect(toggle("***[mot]***", "**")).toBe("*[mot]*");
  });

  it("reads the whole run of stars inside a selection too", () => {
    expect(toggle("[**mot**]", "*")).toBe("*[**mot**]*");
    expect(toggle("[*mot*]", "*")).toBe("[mot]");
    expect(toggle("[***mot***]", "*")).toBe("[**mot**]");
    expect(toggle("[**mot**]", "**")).toBe("[mot]");
  });

  it("formats each line of a selection on several lines, after its block mark", () => {
    expect(toggle("[# Plan\n- a\n\n> b]", "**")).toBe("[# **Plan**\n- **a**\n\n> **b**]");
    expect(toggle("[# **Plan**\n- **a**]", "**")).toBe("[# Plan\n- a]");
  });
});

describe("slashCommandQuery in code", () => {
  it("stays closed inside a fenced code block and opens after it", () => {
    const inside = "```\n/co";
    const after = "```\nx\n```\n/co";
    expect(slashCommandQuery(inside, inside.length)).toBeNull();
    expect(slashCommandQuery(after, after.length)).not.toBeNull();
  });
});

describe("differingRange", () => {
  it("finds the one changed part, so undo has a small step", () => {
    expect(differingRange("/gras", "****")).toEqual({ start: 0, end: 5, inserted: "****" });
    expect(differingRange("a\n/tit\nb", "a\n# \nb")).toEqual({ start: 2, end: 6, inserted: "# " });
    expect(differingRange("un mot", "un **mot**")).toEqual({ start: 3, end: 6, inserted: "**mot**" });
    expect(differingRange("un **mot** ici", "un mot ici")).toEqual({ start: 3, end: 10, inserted: "mot" });
    expect(differingRange("abc", "")).toEqual({ start: 0, end: 3, inserted: "" });
    expect(differingRange("same", "same")).toEqual({ start: 4, end: 4, inserted: "" });
  });
});
