import { describe, expect, it } from "vitest";

import type { PublicUser, Role } from "../lib/protocol";
import {
  type BlockNode,
  type InlineNode,
  type MarkdownList,
  type MarkdownListItem,
  markdownMentionsUser,
  parseMessageMarkdown,
  parseMessageMarkdownCountingSteps,
  safeHttpUrl,
} from "./markdown";
import { mentionablePeople } from "./mentions";

const paul: PublicUser = { id: "1", username: "paul", display_name: "Paul Carouge" };
const people = mentionablePeople([paul]);

const text = (value: string): InlineNode => ({ kind: "text", text: value });
const paragraph = (...children: InlineNode[]): BlockNode => ({ kind: "paragraph", children });
const item = (...parts: (InlineNode | MarkdownList)[]): MarkdownListItem => ({
  children: parts.filter((part): part is InlineNode => part.kind !== "list"),
  sublists: parts.filter((part): part is MarkdownList => part.kind === "list"),
});
const bullets = (...items: MarkdownListItem[]): MarkdownList => ({ kind: "list", start: null, items });
const numbered = (start: number, ...items: MarkdownListItem[]): MarkdownList => ({ kind: "list", start, items });

/** The inline nodes of a message that is one paragraph. */
function inlines(content: string): InlineNode[] {
  const blocks = parseMessageMarkdown(content, people);
  expect(blocks).toHaveLength(1);
  const [block] = blocks;
  if (block?.kind !== "paragraph") {
    throw new Error(`expected one paragraph, got ${JSON.stringify(blocks)}`);
  }
  return block.children;
}

describe("inline formatting", () => {
  it("reads each construct", () => {
    expect(inlines("**bold**")).toEqual([{ kind: "strong", children: [text("bold")] }]);
    expect(inlines("*italic*")).toEqual([{ kind: "emphasis", children: [text("italic")] }]);
    expect(inlines("_italic_")).toEqual([{ kind: "emphasis", children: [text("italic")] }]);
    expect(inlines("__underline__")).toEqual([{ kind: "underline", children: [text("underline")] }]);
    expect(inlines("~~strike~~")).toEqual([{ kind: "strikethrough", children: [text("strike")] }]);
    expect(inlines("`code`")).toEqual([{ kind: "code", text: "code" }]);
    expect(inlines("||secret||")).toEqual([{ kind: "spoiler", id: 0, children: [text("secret")] }]);
  });

  it("nests constructs", () => {
    expect(inlines("***both***")).toEqual([
      { kind: "strong", children: [{ kind: "emphasis", children: [text("both")] }] },
    ]);
    expect(inlines("*a **b** c*")).toEqual([
      { kind: "emphasis", children: [text("a "), { kind: "strong", children: [text("b")] }, text(" c")] },
    ]);
    expect(inlines("||__~~x~~__||")).toEqual([
      {
        kind: "spoiler",
        id: 0,
        children: [{ kind: "underline", children: [{ kind: "strikethrough", children: [text("x")] }] }],
      },
    ]);
  });

  it("numbers spoilers in reading order", () => {
    expect(inlines("||a|| ||b||")).toEqual([
      { kind: "spoiler", id: 0, children: [text("a")] },
      text(" "),
      { kind: "spoiler", id: 1, children: [text("b")] },
    ]);
  });

  it("leaves unclosed and spaced markers literal", () => {
    for (const literal of ["**bold", "*italic", "a * b * c", "~~strike", "||spoiler", "`code", "2 * 3 = 6", "****"]) {
      expect(inlines(literal)).toEqual([text(literal)]);
    }
  });

  it("keeps underscores inside words literal", () => {
    expect(inlines("snake_case_name")).toEqual([text("snake_case_name")]);
  });

  it("lets code beat everything", () => {
    expect(inlines("`**not bold** ||nor hidden||`")).toEqual([{ kind: "code", text: "**not bold** ||nor hidden||" }]);
    expect(inlines("**a `b** c`")).toEqual([text("**a "), { kind: "code", text: "b** c" }]);
    expect(inlines("``a ` b``")).toEqual([{ kind: "code", text: "a ` b" }]);
  });

  it("reads backslash escapes as literal symbols", () => {
    expect(inlines("\\*not italic\\*")).toEqual([text("*"), text("not italic"), text("*")]);
    expect(inlines("\\`not code\\`")).toEqual([text("`"), text("not code"), text("`")]);
    expect(inlines("\\a")).toEqual([text("\\a")]);
  });
});

describe("links", () => {
  it("autolinks http and https, leaving sentence punctuation outside", () => {
    expect(inlines("See https://example.org/a?b=1. Thanks")).toEqual([
      text("See "),
      { kind: "link", url: "https://example.org/a?b=1", isMasked: false, children: [text("https://example.org/a?b=1")] },
      text(". Thanks"),
    ]);
  });

  it("does not format inside an autolink", () => {
    expect(inlines("http://example.org/a_b_c")).toEqual([
      { kind: "link", url: "http://example.org/a_b_c", isMasked: false, children: [text("http://example.org/a_b_c")] },
    ]);
  });

  it("reads masked links with formatted text", () => {
    expect(inlines("[the **docs**](https://example.org/docs)")).toEqual([
      {
        kind: "link",
        url: "https://example.org/docs",
        isMasked: true,
        children: [text("the "), { kind: "strong", children: [text("docs")] }],
      },
    ]);
  });

  it("keeps balanced parentheses inside links", () => {
    const wikipedia = "https://en.wikipedia.org/wiki/Mellow_(album)";
    expect(inlines(`[the album](${wikipedia})`)).toEqual([
      { kind: "link", url: wikipedia, isMasked: true, children: [text("the album")] },
    ]);
    expect(inlines(`see ${wikipedia}.`)).toEqual([
      text("see "),
      { kind: "link", url: wikipedia, isMasked: false, children: [text(wikipedia)] },
      text("."),
    ]);
    expect(inlines("(https://example.org/a)")).toEqual([
      text("("),
      { kind: "link", url: "https://example.org/a", isMasked: false, children: [text("https://example.org/a")] },
      text(")"),
    ]);
    expect(inlines("[a](https://example.org/x_(y)").some((node) => node.kind === "link" && node.isMasked)).toBe(false);
  });

  it("keeps spoiler bars literal inside masked link text", () => {
    expect(inlines("[||x||](https://e.org)")).toEqual([
      { kind: "link", url: "https://e.org/", isMasked: true, children: [text("||x||")] },
    ]);
  });

  it("keeps masked links literal when their text could mislead", () => {
    // The destination is still linked on its own, so the reader sees where it goes.
    const isMaskedLink = (node: InlineNode) => node.kind === "link" && node.isMasked;
    for (const content of [
      "[paypal.com](https://evil.example)",
      "[login.paypal.com/secure](https://evil.example)",
      "[https://paypal.com](https://evil.example)",
      "[see ftp://files](https://evil.example)",
      "[\u200B](https://evil.example)",
      "[ ](https://evil.example)",
      "[\u2800](https://evil.example)",
      "[@paul](https://evil.example)",
    ]) {
      expect(inlines(content).some(isMaskedLink)).toBe(false);
    }
    expect(inlines("[e.g. this](https://example.org)")).toEqual([
      { kind: "link", url: "https://example.org/", isMasked: true, children: [text("e.g. this")] },
    ]);
  });

  it("never links a URL carrying a user name or password", () => {
    expect(inlines("https://paypal.com@evil.example")).toEqual([text("https://paypal.com@evil.example")]);
    expect(inlines("[pay](https://user:secret@evil.example)")).toEqual([text("[pay](https://user:secret@evil.example)")]);
    expect(safeHttpUrl("https://paypal.com@evil.example")).toBeNull();
  });

  it("never links other schemes", () => {
    for (const literal of [
      "[click](javascript:alert(1))",
      "[click](data:text/html,x)",
      "[click](JaVaScRiPt:alert(1))",
      "[click](//example.org)",
      "javascript:alert(1) data:text/html,x ftp://example.org",
    ]) {
      const nodes = inlines(literal);
      expect(nodes.some((node) => node.kind === "link")).toBe(false);
      expect(nodes.map((node) => (node.kind === "text" ? node.text : "")).join("")).toBe(literal);
    }
    expect(safeHttpUrl("javascript:alert(1)")).toBeNull();
    expect(safeHttpUrl("https:example.org")).toBeNull();
  });

  it("keeps markup as text", () => {
    expect(inlines('<img src=x onerror="y">')).toEqual([text('<img src=x onerror="y">')]);
  });
});

describe("mentions", () => {
  it("are found in text and formatting, never in code", () => {
    expect(inlines("hi **@paul**")).toEqual([
      text("hi "),
      { kind: "strong", children: [{ kind: "mention", text: "@paul", user: paul }] },
    ]);
    expect(inlines("`@paul`")).toEqual([{ kind: "code", text: "@paul" }]);
    expect(markdownMentionsUser(parseMessageMarkdown("`@paul`", people), "1")).toBe(false);
    expect(markdownMentionsUser(parseMessageMarkdown("```\n@paul\n```", people), "1")).toBe(false);
    expect(markdownMentionsUser(parseMessageMarkdown("> - ||@paul||", people), "1")).toBe(true);
    expect(markdownMentionsUser(parseMessageMarkdown("- a\n  1. @paul", people), "1")).toBe(true);
    expect(markdownMentionsUser(parseMessageMarkdown("-# @paul", people), "1")).toBe(true);
    expect(markdownMentionsUser(parseMessageMarkdown("@zoe", people), "1")).toBe(false);
  });

  it("are not read after a backslash", () => {
    expect(inlines("\\@paul")).toEqual([text("@"), text("paul")]);
  });
});

describe("blocks", () => {
  it("reads fenced code blocks with an optional language, literally", () => {
    expect(parseMessageMarkdown("before\n```ts\nconst a = **1**;\n# no heading\n```\nafter", people)).toEqual([
      paragraph(text("before")),
      { kind: "codeBlock", language: "ts", text: "const a = **1**;\n# no heading" },
      paragraph(text("after")),
    ]);
    expect(parseMessageMarkdown("```one line```", people)).toEqual([
      { kind: "codeBlock", language: null, text: "one line" },
    ]);
  });

  it("leaves an unclosed fence literal", () => {
    expect(parseMessageMarkdown("```ts\nno end", people)).toEqual([paragraph(text("```ts\nno end"))]);
  });

  it("reads headings at line start only", () => {
    expect(parseMessageMarkdown("# One\n## Two\n### Three\n#### Four\n#nospace", people)).toEqual([
      { kind: "heading", level: 1, children: [text("One")] },
      { kind: "heading", level: 2, children: [text("Two")] },
      { kind: "heading", level: 3, children: [text("Three")] },
      paragraph(text("#### Four\n#nospace")),
    ]);
  });

  it("reads bullet lists", () => {
    expect(parseMessageMarkdown("Shopping:\n- milk\n* **eggs**\nthanks", people)).toEqual([
      paragraph(text("Shopping:")),
      bullets(item(text("milk")), item({ kind: "strong", children: [text("eggs")] })),
      paragraph(text("thanks")),
    ]);
  });

  it("reads numbered lists from their first number, with dots or parentheses", () => {
    expect(parseMessageMarkdown("1. un\n2. deux", people)).toEqual([numbered(1, item(text("un")), item(text("deux")))]);
    expect(parseMessageMarkdown("3) trois\n3) quatre", people)).toEqual([numbered(3, item(text("trois")), item(text("quatre")))]);
    expect(parseMessageMarkdown("0. zéro", people)).toEqual([numbered(0, item(text("zéro")))]);
  });

  it("starts a new list when bullets and numbers alternate", () => {
    expect(parseMessageMarkdown("- a\n1. b\n- c", people)).toEqual([
      bullets(item(text("a"))),
      numbered(1, item(text("b"))),
      bullets(item(text("c"))),
    ]);
  });

  it("leaves numbers without a space, past nine digits or with nothing after them as text", () => {
    expect(parseMessageMarkdown("1.5 kg\n1234567890. trop\n2. ", people)).toEqual([paragraph(text("1.5 kg\n1234567890. trop\n2. "))]);
  });

  it("nests an item indented by two spaces or a tab under the one before, once", () => {
    expect(parseMessageMarkdown("- a\n  - b\n\t1. c\n      2. d\n- e", people)).toEqual([
      bullets(
        item(text("a"), bullets(item(text("b"))), numbered(1, item(text("c")), item(text("d")))),
        item(text("e")),
      ),
    ]);
  });

  it("keeps an indented item without a parent, or indented by one space, out of a list", () => {
    expect(parseMessageMarkdown("  - orphan", people)).toEqual([paragraph(text("  - orphan"))]);
    expect(parseMessageMarkdown("- a\n - b", people)).toEqual([bullets(item(text("a"))), paragraph(text(" - b"))]);
  });

  it("reads subtext at line start, with formatting inside", () => {
    expect(parseMessageMarkdown("hello\n-# **petit** mot\n-#nospace\n-# ", people)).toEqual([
      paragraph(text("hello")),
      { kind: "subtext", children: [{ kind: "strong", children: [text("petit")] }, text(" mot")] },
      paragraph(text("-#nospace\n-# ")),
    ]);
  });

  it("reads lists and subtext inside quotes", () => {
    expect(parseMessageMarkdown("> 1. a\n>   - b\n> -# note", people)).toEqual([
      {
        kind: "quote",
        children: [numbered(1, item(text("a"), bullets(item(text("b"))))), { kind: "subtext", children: [text("note")] }],
      },
    ]);
  });

  it("reads quote lines, with blocks inside", () => {
    expect(parseMessageMarkdown("> quoted\n> ```\n> code\n> ```\nreply", people)).toEqual([
      { kind: "quote", children: [paragraph(text("quoted")), { kind: "codeBlock", language: null, text: "code" }] },
      paragraph(text("reply")),
    ]);
  });

  it("quotes the rest of the message after >>>, without nested quotes", () => {
    expect(parseMessageMarkdown("intro\n>>> all\nof\n> this", people)).toEqual([
      paragraph(text("intro")),
      { kind: "quote", children: [paragraph(text("all\nof\n> this"))] },
    ]);
  });

  it("keeps line breaks inside paragraphs", () => {
    expect(parseMessageMarkdown("one\ntwo\n\nthree", people)).toEqual([paragraph(text("one\ntwo\n\nthree"))]);
  });

  it("returns nothing for an empty message", () => {
    expect(parseMessageMarkdown("", people)).toEqual([]);
  });
});

/**
 * How many string searches and regular expression runs `action` makes: a measure of the parser's
 * work that a busy machine cannot change, unlike a clock. Backtracking inside one regular
 * expression is not counted, which is why the parser keeps unbounded runs out of its expressions.
 */
function countSearchesDuring(action: () => void): number {
  let searches = 0;
  const countedMethods = [
    [String.prototype, "startsWith"],
    [String.prototype, "indexOf"],
    [RegExp.prototype, "exec"],
    [RegExp.prototype, "test"],
  ] as const;
  const originals = countedMethods.map(([owner, name]) => {
    const original = Reflect.get(owner, name) as (...parameters: unknown[]) => unknown;
    Reflect.set(owner, name, function (this: unknown, ...parameters: unknown[]) {
      searches += 1;
      return original.apply(this, parameters);
    });
    return original;
  });
  try {
    action();
  } finally {
    countedMethods.forEach(([owner, name], index) => Reflect.set(owner, name, originals[index]));
  }
  return searches;
}

describe("hostile text", () => {
  /**
   * Doubling the text may raise the work per character by at most this factor: linear parsing
   * keeps it flat, and a quadratic one (the cases below once took seconds) doubles it.
   */
  const MAXIMUM_GROWTH_OF_WORK_PER_CHARACTER = 1.25;

  it.each<[string, (scale: number) => string]>([
    ["stars", (scale) => "*".repeat(4_000 * scale)],
    ["underscores", (scale) => "_".repeat(4_000 * scale)],
    ["alternating stars", (scale) => "*a".repeat(2_000 * scale)],
    ["spoiler bars", (scale) => "|".repeat(4_000 * scale)],
    ["tildes", (scale) => "~".repeat(4_000 * scale)],
    [
      "backticks of growing length",
      (scale) => Array.from({ length: 80 * scale }, (_, index) => "`".repeat(index + 1)).join("a"),
    ],
    ["brackets", (scale) => "[".repeat(2_000 * scale) + "](".repeat(1_000 * scale)],
    ["backslashes", (scale) => "\\".repeat(4_000 * scale) + "*"],
    ["fences", (scale) => "```".repeat(1_300 * scale)],
    ["quote lines", (scale) => "> ".repeat(2_000 * scale)],
    ["numbered items", (scale) => "1. a\n".repeat(2_000 * scale)],
    ["nested items", (scale) => "- a\n" + "  - b\n\t1. c\n".repeat(1_000 * scale)],
    ["subtext lines", (scale) => "-# a\n".repeat(2_000 * scale)],
    ["links", (scale) => "http://a".repeat(500 * scale) + ".".repeat(2_000 * scale)],
    // Many openers with no closer: each search must be answered once, not once per opener.
    ["unclosed stars", (scale) => "*a ".repeat(15_000 * scale)],
    ["unclosed underscores", (scale) => "_a ".repeat(15_000 * scale)],
    ["spoilers", (scale) => "||a ".repeat(15_000 * scale)],
    ["unclosed masked links", (scale) => "[a](".repeat(15_000 * scale)],
    ["blank lines inside a code block", (scale) => "```\na" + "\n".repeat(50_000 * scale) + "b\n```"],
  ])("parses %s in linear work", (_, contentAtScale) => {
    // Two measures, as neither sees everything: the parser's own loop steps miss scans inside
    // built-in searches, and counted searches miss the parser's plain character loops.
    const workPerCharacterAtScale = (scale: number) => {
      const content = contentAtScale(scale);
      let parsed = { blocks: [] as BlockNode[], steps: 0 };
      const searches = countSearchesDuring(() => {
        parsed = parseMessageMarkdownCountingSteps(content, people);
      });
      expect(parsed.blocks.length).toBeGreaterThan(0);
      return { steps: parsed.steps / content.length, searches: searches / content.length };
    };
    const single = workPerCharacterAtScale(1);
    const double = workPerCharacterAtScale(2);
    expect(double.steps).toBeLessThanOrEqual(single.steps * MAXIMUM_GROWTH_OF_WORK_PER_CHARACTER);
    expect(double.searches).toBeLessThanOrEqual(single.searches * MAXIMUM_GROWTH_OF_WORK_PER_CHARACTER);
  });

  it("caps nesting and shows the rest as text", () => {
    let nodes = inlines("||[**__~~*_`code`_*~~__**](https://example.org)||");
    let depth = 0;
    while (nodes.length === 1 && nodes[0] && "children" in nodes[0]) {
      nodes = nodes[0].children;
      depth += 1;
    }
    expect(depth).toBe(6);
    expect(nodes).toEqual([text("_`code`_")]);
  });
});

describe("custom emoji", () => {
  const party = { id: "500", name: "party", is_animated: false };
  const parrot = { id: "501", name: "par_rot_", is_animated: true };
  const known = new Map([party, parrot].map((emoji) => [emoji.id, emoji]));
  const parse = (content: string) => {
    const [block] = parseMessageMarkdown(content, people, known);
    return block?.kind === "paragraph" ? block.children : [];
  };

  it("show a known emoji by its id, whatever name the markup gives it", () => {
    expect(parse("hi <:fiesta:500>!")).toEqual([
      text("hi "),
      { kind: "customEmoji", emoji: party },
      text("!"),
    ]);
    expect(parse("<a:par_rot_:501>")).toEqual([{ kind: "customEmoji", emoji: parrot }]);
  });

  it("read as :name: when this client cannot show them", () => {
    expect(parse("<:gone:999>")).toEqual([text(":gone:")]);
    expect(parseMessageMarkdown("<:party:500>", people)).toEqual([paragraph(text(":party:"))]);
  });

  it("stay literal in code and when escaped, and keep formatting around them intact", () => {
    expect(parse("`<:party:500>`")).toEqual([{ kind: "code", text: "<:party:500>" }]);
    expect(parse("\\<:party:500>")).toEqual([text("<"), text(":party:500>")]);
    expect(parse("_a <a:par_rot_:501>_")).toEqual([
      { kind: "emphasis", children: [text("a "), { kind: "customEmoji", emoji: parrot }] },
    ]);
    expect(parse("**<:party:500>**")).toEqual([
      { kind: "strong", children: [{ kind: "customEmoji", emoji: party }] },
    ]);
    expect(parse("<:x:500> <:party:abc>")).toEqual([text("<:x:500> <:party:abc>")]);
  });
});

describe("role and @everyone mentions (ADR-0040)", () => {
  const moderators: Role = { id: "201", name: "Modérateurs", color: 0x3498db, position: 1, permissions: "0", is_displayed_separately: false };
  const roles = new Map([[moderators.id, moderators]]);
  const parse = (content: string, isEveryoneMentioned = false) => {
    const [block] = parseMessageMarkdown(content, people, new Map(), { roles, isEveryoneMentioned });
    return block?.kind === "paragraph" ? block.children : [];
  };

  it("show a role by its id, and a role this client does not know as deleted", () => {
    expect(parse("hi <@&201>!")).toEqual([text("hi "), { kind: "roleMention", role: moderators }, text("!")]);
    expect(parse("<@&999>")).toEqual([{ kind: "roleMention", role: null }]);
    expect(parse("**<@&201>**")).toEqual([{ kind: "strong", children: [{ kind: "roleMention", role: moderators }] }]);
  });

  it("stay literal in code, when escaped, and when malformed", () => {
    expect(parse("`<@&201>`")).toEqual([{ kind: "code", text: "<@&201>" }]);
    expect(parse("\\<@&201>")).toEqual([text("<"), text("@&201>")]);
    expect(parse("<@&abc> <@&>")).toEqual([text("<@&abc> <@&>")]);
  });

  it("read @everyone as a mention only when the server kept it, never as a person", () => {
    const everyonePerson = mentionablePeople([{ id: "9", username: "everyone", display_name: "" }]);
    expect(parse("hey @Everyone.", true)).toEqual([text("hey "), { kind: "everyoneMention", text: "@Everyone" }, text(".")]);
    expect(parse("hey @everyone")).toEqual([text("hey @everyone")]);
    expect(parseMessageMarkdown("@everyone", everyonePerson)).toEqual([paragraph(text("@everyone"))]);
    expect(parse("`@everyone`", true)).toEqual([{ kind: "code", text: "@everyone" }]);
    expect(parse("||@everyone||", true)).toEqual([{ kind: "spoiler", id: 0, children: [{ kind: "everyoneMention", text: "@everyone" }] }]);
  });

  it("keep a masked link from holding them, as person mentions do", () => {
    expect(parse("[<@&201>](https://example.com)")[0]?.kind).not.toBe("link");
  });

  it("never count as mentioning the reader by name", () => {
    expect(markdownMentionsUser(parseMessageMarkdown("<@&201> @everyone", people, new Map(), { roles, isEveryoneMentioned: true }), "1")).toBe(false);
  });
});
