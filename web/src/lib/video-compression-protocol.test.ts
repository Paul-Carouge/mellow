import { afterEach, describe, expect, it, vi } from "vitest";

import {
  type CompressionWorker,
  type CompressionWorkerScope,
  type VideoCompressionAnswer,
  type VideoCompressionRequest,
  answerVideoCompressionRequests,
  compressVideoInWorker,
} from "./video-compression-protocol";

/** A Worker the test answers for. */
class FakeWorker implements CompressionWorker {
  onmessage: ((event: MessageEvent<VideoCompressionAnswer>) => void) | null = null;
  onerror: ((event: ErrorEvent) => void) | null = null;
  readonly requests: VideoCompressionRequest[] = [];
  isTerminated = false;

  postMessage(request: VideoCompressionRequest): void {
    this.requests.push(request);
  }

  terminate(): void {
    this.isTerminated = true;
  }

  answer(answer: VideoCompressionAnswer): void {
    this.onmessage?.(new MessageEvent("message", { data: answer }));
  }
}

const ORIGINAL = new File([new Uint8Array(1000)], "IMG_0042.MOV", { type: "video/quicktime" });
const LIMIT = 25 * 1024 * 1024;

function start(signal = new AbortController().signal) {
  const worker = new FakeWorker();
  const progress: number[] = [];
  const result = compressVideoInWorker(ORIGINAL, LIMIT, (fraction) => progress.push(fraction), signal, () => worker);
  return { worker, progress, result };
}

describe("compressVideoInWorker", () => {
  afterEach(() => {
    vi.restoreAllMocks();
  });

  it("sends the file and the limit, reports progress and resolves with the MP4", async () => {
    const { worker, progress, result } = start();
    expect(worker.requests).toEqual([{ file: ORIGINAL, maximumAttachmentBytes: LIMIT }]);
    worker.answer({ kind: "progress", fraction: 0.5 });
    worker.answer({ kind: "compressed", bytes: new Uint8Array(300).buffer });
    const compressed = await result;
    expect(progress).toEqual([0.5]);
    expect(compressed.name).toBe("IMG_0042.mp4");
    expect(compressed.type).toBe("video/mp4");
    expect(compressed.size).toBe(300);
    expect(worker.isTerminated).toBe(true);
  });

  it("resolves with the original when the Worker keeps it", async () => {
    const { worker, result } = start();
    worker.answer({ kind: "kept-original", reason: "high-dynamic-range" });
    expect(await result).toBe(ORIGINAL);
    expect(worker.isTerminated).toBe(true);
  });

  it("resolves with the original when the Worker fails", async () => {
    const warn = vi.spyOn(console, "warn").mockImplementation(() => {});
    const { worker, result } = start();
    worker.onerror?.(new ErrorEvent("error"));
    expect(await result).toBe(ORIGINAL);
    expect(worker.isTerminated).toBe(true);
    expect(warn).toHaveBeenCalledOnce();
  });

  it("terminates the Worker and rejects when aborted, and ignores later answers", async () => {
    const abort = new AbortController();
    const { worker, progress, result } = start(abort.signal);
    abort.abort();
    await expect(result).rejects.toBeInstanceOf(DOMException);
    expect(worker.isTerminated).toBe(true);
    worker.answer({ kind: "progress", fraction: 0.9 });
    expect(progress).toEqual([]);
  });

  it("starts no Worker for an already aborted signal", async () => {
    const abort = new AbortController();
    abort.abort();
    const startWorker = vi.fn(() => new FakeWorker());
    await expect(compressVideoInWorker(ORIGINAL, null, () => {}, abort.signal, startWorker)).rejects.toBeInstanceOf(
      DOMException,
    );
    expect(startWorker).not.toHaveBeenCalled();
  });
});

/** The Worker's global scope, as the test sees it. */
class FakeScope implements CompressionWorkerScope {
  onmessage: ((event: MessageEvent<VideoCompressionRequest>) => void) | null = null;
  readonly answers: { answer: VideoCompressionAnswer; transfer: Transferable[] | undefined }[] = [];

  postMessage(answer: VideoCompressionAnswer, transfer?: Transferable[]): void {
    this.answers.push({ answer, transfer });
  }

  async request(request: VideoCompressionRequest): Promise<void> {
    await this.onmessage?.(new MessageEvent("message", { data: request }));
  }
}

describe("answerVideoCompressionRequests", () => {
  afterEach(() => {
    vi.restoreAllMocks();
  });

  it("forwards progress and transfers the compressed bytes", async () => {
    const scope = new FakeScope();
    const bytes = new Uint8Array(8).buffer;
    answerVideoCompressionRequests(scope, async (request, onprogress) => {
      expect(request.maximumAttachmentBytes).toBe(LIMIT);
      onprogress(0.25);
      return { bytes };
    });
    await scope.request({ file: ORIGINAL, maximumAttachmentBytes: LIMIT });
    expect(scope.answers).toEqual([
      { answer: { kind: "progress", fraction: 0.25 }, transfer: undefined },
      { answer: { kind: "compressed", bytes }, transfer: [bytes] },
    ]);
  });

  it("posts progress once per whole percent, not once per frame", async () => {
    const scope = new FakeScope();
    answerVideoCompressionRequests(scope, async (_request, onprogress) => {
      for (let frame = 1; frame <= 1000; frame++) {
        onprogress(frame / 1000);
      }
      return { keptOriginalBecause: "not-smaller" };
    });
    await scope.request({ file: ORIGINAL, maximumAttachmentBytes: null });
    const fractions = scope.answers.flatMap(({ answer }) => (answer.kind === "progress" ? [answer.fraction] : []));
    expect(fractions.length).toBeLessThanOrEqual(101);
    expect(fractions.at(-1)).toBe(1);
  });

  it("says why the original stays", async () => {
    const scope = new FakeScope();
    answerVideoCompressionRequests(scope, async () => ({ keptOriginalBecause: "not-smaller" }));
    await scope.request({ file: ORIGINAL, maximumAttachmentBytes: null });
    expect(scope.answers).toEqual([{ answer: { kind: "kept-original", reason: "not-smaller" }, transfer: undefined }]);
  });

  it("keeps the original when compressing throws", async () => {
    const warn = vi.spyOn(console, "warn").mockImplementation(() => {});
    const scope = new FakeScope();
    answerVideoCompressionRequests(scope, async () => {
      throw new Error("the encoder closed");
    });
    await scope.request({ file: ORIGINAL, maximumAttachmentBytes: null });
    expect(scope.answers).toEqual([{ answer: { kind: "kept-original", reason: "failed" }, transfer: undefined }]);
    expect(warn).toHaveBeenCalledOnce();
  });
});
