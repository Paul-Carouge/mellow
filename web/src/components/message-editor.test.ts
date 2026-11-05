import { describe, expect, it } from "vitest";

import { MAXIMUM_MESSAGE_LENGTH } from "../lib/protocol";
import { MessageEditor } from "./message-editor.svelte";

function message(id: string, content: string) {
  return { id, content };
}

const asTyped = (text: string) => text;

describe("MessageEditor", () => {
  it("opens on the message's text and hands the typed text over when saved", () => {
    const editor = new MessageEditor(asTyped);
    editor.open(message("1", "helo"));
    expect([editor.messageId, editor.text]).toEqual(["1", "helo"]);

    editor.text = "hello";
    expect(editor.takeTextToSave()).toBe("hello");
    expect(editor.messageId).toBeNull();
  });

  it("reopens with the typed text when the instance refuses the edit", () => {
    const editor = new MessageEditor(asTyped);
    const edited = message("1", "helo");
    editor.open(edited);
    editor.text = "hello, fixed";
    const text = editor.takeTextToSave() ?? "";

    editor.giveBack(edited, text, true);

    expect([editor.messageId, editor.text]).toEqual(["1", "hello, fixed"]);
  });

  it("keeps a refused text for the message's next edit when the editor is busy or the chat moved on", () => {
    const editor = new MessageEditor(asTyped);
    const first = message("1", "one");
    const second = message("2", "two");
    editor.open(first);
    editor.text = "one, edited";
    const text = editor.takeTextToSave() ?? "";
    editor.open(second);

    editor.giveBack(first, text, true);
    expect([editor.messageId, editor.text]).toEqual(["2", "two"]);

    editor.close();
    editor.giveBack(second, "two, edited", false);
    expect(editor.messageId).toBeNull();

    editor.open(first);
    expect(editor.text).toBe("one, edited");
    editor.close();
    editor.open(first);
    expect(editor.text).toBe("one");
    editor.open(second);
    expect(editor.text).toBe("two, edited");
  });

  it("counts characters as sent and refuses to save a text that is too long, staying open", () => {
    const editor = new MessageEditor((text) => text.replaceAll(":wave:", "<:wave:123>"));
    editor.open(message("1", ""));
    editor.text = ":wave:";
    expect(editor.charactersLeft).toBe(MAXIMUM_MESSAGE_LENGTH - "<:wave:123>".length);

    editor.text = "a".repeat(MAXIMUM_MESSAGE_LENGTH + 1);
    expect(editor.isTooLong).toBe(true);
    expect(editor.hasTriedSavingTooLong).toBe(false);
    expect(editor.takeTextToSave()).toBeNull();
    expect([editor.messageId, editor.hasTriedSavingTooLong]).toEqual(["1", true]);

    editor.text = "short";
    expect(editor.takeTextToSave()).toBe("short");
    editor.open(message("1", ""));
    expect(editor.hasTriedSavingTooLong).toBe(false);
  });
});
