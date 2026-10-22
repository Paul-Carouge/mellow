import { describe, expect, it } from "vitest";

import type { PublicUser } from "../lib/protocol";
import { type BlockNode, type InlineNode, markdownMentionsUser, parseMessageMarkdown, safeHttpUrl } from "./markdown";
import { mentionablePeople } from "./mentions";

const paul: PublicUser = { id: "1", username: "paul", display_name: "Paul Carouge" };
const people = mentionablePeople([paul]);

const text = (value: string): InlineNode => ({ kind: "text", text: value });
const paragraph = (...children: InlineNode[]): BlockNode => ({ kind: "paragraph", children });

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
      { kind: "list", items: [[text("milk")], [{ kind: "strong", children: [text("eggs")] }]] },
      paragraph(text("thanks")),
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

describe("hostile text", () => {
  const MAXIMUM_PARSE_MILLISECONDS = 100;

  it.each([
    ["stars", "*".repeat(4_000)],
    ["underscores", "_".repeat(4_000)],
    ["alternating stars", "*a".repeat(2_000)],
    ["spoiler bars", "|".repeat(4_000)],
    ["tildes", "~".repeat(4_000)],
    ["backticks of growing length", Array.from({ length: 80 }, (_, index) => "`".repeat(index + 1)).join("a")],
    ["brackets", "[".repeat(2_000) + "](".repeat(1_000)],
    ["backslashes", "\\".repeat(4_000) + "*"],
    ["fences", "```".repeat(1_300)],
    ["quote lines", "> ".repeat(2_000)],
    ["links", "http://a".repeat(500) + ".".repeat(2_000)],
  ])("parses %s quickly", (_, content) => {
    const started = performance.now();
    const blocks = parseMessageMarkdown(content, people);
    expect(performance.now() - started).toBeLessThan(MAXIMUM_PARSE_MILLISECONDS);
    expect(blocks.length).toBeGreaterThan(0);
  });

  it("caps nesting and shows the rest as text", () => {
    let nodes = inlines("[||**__~~*_`code`_*~~__**||](https://example.org)");
    let depth = 0;
    while (nodes.length === 1 && nodes[0] && "children" in nodes[0]) {
      nodes = nodes[0].children;
      depth += 1;
    }
    expect(depth).toBe(6);
    expect(nodes).toEqual([text("_`code`_")]);
  });
});
