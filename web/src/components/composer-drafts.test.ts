import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { ComposerDrafts, compressThenUpload } from "./composer-drafts.svelte";

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

  it("aborts the upload of a removed file and leaves the others running", () => {
    const drafts = new ComposerDrafts();
    drafts.open(GENERAL);
    const removed = drafts.attach(imageFile("one.png"));
    const kept = drafts.attach(imageFile("two.png"));
    const removedSignal = removed.uploadAbort.signal;

    drafts.removeTrayItem(removed.key);

    expect(removedSignal.aborted).toBe(true);
    expect(kept.uploadAbort.signal.aborted).toBe(false);
  });

  it("stops a removed video's compression after its upload, so the original is not sent instead", () => {
    const drafts = new ComposerDrafts();
    drafts.open(GENERAL);
    const video = drafts.attach(new File(["clip"], "clip.mov", { type: "video/quicktime" }));
    let wasUploadAbortedFirst = false;
    video.compressionAbort.signal.addEventListener("abort", () => {
      wasUploadAbortedFirst = video.uploadAbort.signal.aborted;
    });

    drafts.removeTrayItem(video.key);

    expect([video.compressionAbort.signal.aborted, wasUploadAbortedFirst]).toEqual([true, true]);
  });

  it("aborts the open tray's uploads when cleared, and every upload when disposed", () => {
    const drafts = new ComposerDrafts();
    drafts.open(GENERAL);
    const aside = drafts.attach(imageFile("aside.png")).uploadAbort.signal;
    drafts.open(RANDOM);
    const cleared = drafts.attach(imageFile("cleared.png")).uploadAbort.signal;

    drafts.clearTray();
    expect([cleared.aborted, aside.aborted]).toEqual([true, false]);

    const open = drafts.attach(imageFile("open.png")).uploadAbort.signal;
    drafts.dispose();
    expect([open.aborted, aside.aborted]).toEqual([true, true]);
  });

  it("binds each file to the channel open when it was attached, not the one open when it uploads", () => {
    const drafts = new ComposerDrafts();
    drafts.open(GENERAL);
    const item = drafts.attach(imageFile("cat.png"));
    drafts.open(RANDOM);

    expect(item.channelId).toBe(GENERAL);
  });

  it("starts a failed file over with a fresh upload", () => {
    const drafts = new ComposerDrafts();
    drafts.open(GENERAL);
    const item = drafts.attach(imageFile("cat.png"));
    item.uploadAbort.abort();
    item.status = "failed";
    item.error = "Couldn't upload cat.png";

    drafts.restartUpload(item);

    expect([item.status, item.error, item.uploadAbort.signal.aborted]).toEqual(["uploading", "", false]);
  });
});

describe("compressThenUpload", () => {
  function videoItem() {
    const drafts = new ComposerDrafts();
    drafts.open(GENERAL);
    return drafts.attach(new File(["clip"], "clip.mov", { type: "video/quicktime" }));
  }

  /** A compression that runs until `signal` aborts, as the Worker does. */
  function compressUntilAborted(_file: File, _onprogress: (fraction: number) => void, signal: AbortSignal) {
    return new Promise<File>((_resolve, reject) => signal.addEventListener("abort", () => reject(signal.reason)));
  }

  it("uploads the compressed file and reports progress meanwhile", async () => {
    const item = videoItem();
    const compressed = new File(["small"], "clip.mp4", { type: "video/mp4" });
    const uploaded: File[] = [];

    await compressThenUpload(
      item,
      (_file, onprogress) => {
        expect(item.status).toBe("compressing");
        onprogress(0.5);
        expect(item.compressionProgress).toBe(0.5);
        return Promise.resolve(compressed);
      },
      (uploading) => void uploaded.push(uploading.file),
    );

    expect(uploaded).toEqual([compressed]);
  });

  it("uploads the original when the reader picks it during compression", async () => {
    const item = videoItem();
    const original = item.file;
    const uploaded: File[] = [];

    const done = compressThenUpload(item, compressUntilAborted, (uploading) => void uploaded.push(uploading.file));
    item.compressionAbort.abort();
    await done;

    expect([uploaded, item.status]).toEqual([[original], "uploading"]);
  });

  it("uploads nothing when the video leaves the tray during compression", async () => {
    const item = videoItem();
    const upload = vi.fn();

    const done = compressThenUpload(item, compressUntilAborted, upload);
    item.uploadAbort.abort();
    item.compressionAbort.abort();
    await done;

    expect(upload).not.toHaveBeenCalled();
  });

  it("uploads the original when the compression cannot start", async () => {
    vi.spyOn(console, "warn").mockImplementation(() => {});
    const item = videoItem();
    const original = item.file;
    const uploaded: File[] = [];

    await compressThenUpload(
      item,
      () => Promise.reject(new Error("no worker")),
      (uploading) => void uploaded.push(uploading.file),
    );

    expect(uploaded).toEqual([original]);
    vi.restoreAllMocks();
  });

  it("uploads other files, and videos where the browser cannot compress, at once", async () => {
    const compress = vi.fn();
    const uploaded: string[] = [];
    const drafts = new ComposerDrafts();
    drafts.open(GENERAL);

    await compressThenUpload(drafts.attach(imageFile("cat.png")), compress, (item) => void uploaded.push(item.file.name));
    await compressThenUpload(videoItem(), null, (item) => void uploaded.push(item.file.name));

    expect([compress.mock.calls.length, uploaded]).toEqual([0, ["cat.png", "clip.mov"]]);
  });
});
