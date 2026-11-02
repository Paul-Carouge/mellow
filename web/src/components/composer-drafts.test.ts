import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { ComposerDrafts } from "./composer-drafts.svelte";

const GENERAL = "100";
const RANDOM = "200";

function imageFile(name: string): File {
  return new File(["pixels"], name, { type: "image/png" });
}

describe("ComposerDrafts", () => {
  let previewCount = 0;
  const revokedPreviews: string[] = [];

  beforeEach(() => {
    previewCount = 0;
    revokedPreviews.length = 0;
    vi.spyOn(URL, "createObjectURL").mockImplementation(() => `blob:preview-${previewCount++}`);
    vi.spyOn(URL, "revokeObjectURL").mockImplementation((url) => void revokedPreviews.push(url));
  });

  afterEach(() => {
    vi.restoreAllMocks();
  });

  it("brings each channel's text and files back after a switch", () => {
    const drafts = new ComposerDrafts();
    drafts.open(GENERAL);
    drafts.text = "half a thought";
    const item = drafts.attach(imageFile("cat.png"));

    drafts.open(RANDOM);
    expect(drafts.text).toBe("");
    expect(drafts.trayItems).toEqual([]);
    drafts.text = "elsewhere";

    drafts.open(GENERAL);
    expect(drafts.text).toBe("half a thought");
    expect(drafts.trayItems).toEqual([item]);
    drafts.open(RANDOM);
    expect(drafts.text).toBe("elsewhere");
  });

  it("keeps the drafts through a moment without a channel", () => {
    const drafts = new ComposerDrafts();
    drafts.open(GENERAL);
    drafts.text = "kept";

    drafts.open(null);
    expect(drafts.text).toBe("");
    drafts.open(GENERAL);
    expect(drafts.text).toBe("kept");
  });

  it("tells whether the channel changed, and leaves the draft alone when it did not", () => {
    const drafts = new ComposerDrafts();
    expect(drafts.open(null)).toBe(false);
    expect(drafts.open(GENERAL)).toBe(true);
    drafts.text = "typing";
    expect(drafts.open(GENERAL)).toBe(false);
    expect(drafts.text).toBe("typing");
  });

  it("updates a file put aside, so its upload result shows when its channel comes back", () => {
    const drafts = new ComposerDrafts();
    drafts.open(GENERAL);
    const item = drafts.attach(imageFile("cat.png"));
    drafts.open(RANDOM);

    item.status = "uploaded";
    drafts.open(GENERAL);

    expect(drafts.trayItems[0]?.status).toBe("uploaded");
  });

  it("never gives two files the same key, across channels", () => {
    const drafts = new ComposerDrafts();
    drafts.open(GENERAL);
    const first = drafts.attach(new File(["a"], "a.txt"));
    drafts.open(RANDOM);
    const second = drafts.attach(new File(["b"], "b.txt"));

    expect(first.key).not.toBe(second.key);
  });

  it("makes previews for images only", () => {
    const drafts = new ComposerDrafts();
    drafts.open(GENERAL);

    expect(drafts.attach(imageFile("cat.png")).previewUrl).toBe("blob:preview-0");
    expect(drafts.attach(new File(["text"], "notes.txt", { type: "text/plain" })).previewUrl).toBeNull();
  });

  it("frees the preview of a removed file and keeps the others", () => {
    const drafts = new ComposerDrafts();
    drafts.open(GENERAL);
    const removed = drafts.attach(imageFile("one.png"));
    const kept = drafts.attach(imageFile("two.png"));

    drafts.removeTrayItem(removed.key);

    expect(drafts.trayItems).toEqual([kept]);
    expect(revokedPreviews).toEqual(["blob:preview-0"]);
  });

  it("empties the open tray after sending, leaving the other channels' trays", () => {
    const drafts = new ComposerDrafts();
    drafts.open(GENERAL);
    drafts.attach(imageFile("aside.png"));
    drafts.open(RANDOM);
    drafts.attach(imageFile("sent.png"));

    drafts.clearTray();

    expect(drafts.trayItems).toEqual([]);
    expect(revokedPreviews).toEqual(["blob:preview-1"]);
    drafts.open(GENERAL);
    expect(drafts.trayItems).toHaveLength(1);
  });

  it("frees every preview, open and aside, when disposed", () => {
    const drafts = new ComposerDrafts();
    drafts.open(GENERAL);
    drafts.attach(imageFile("aside.png"));
    drafts.open(RANDOM);
    drafts.attach(imageFile("open.png"));

    drafts.dispose();

    expect([...revokedPreviews].sort()).toEqual(["blob:preview-0", "blob:preview-1"]);
  });
});
