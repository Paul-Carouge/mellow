// The messages between the composer and the video compression Worker (ADR-0046), and both ends of
// them, written against the few members they use so tests drive them with fakes.

import { compressedFileName } from "./video-compression";

/** Composer to Worker: one file per Worker, which is terminated afterwards. */
export interface VideoCompressionRequest {
  readonly file: File;
  /** `Ready.maximum_attachment_bytes`; null when the instance does not tell. */
  readonly maximumAttachmentBytes: number | null;
}

/** Worker to composer. */
export type VideoCompressionAnswer =
  | { readonly kind: "progress"; readonly fraction: number }
  /** A faststart MP4, transferred. */
  | { readonly kind: "compressed"; readonly bytes: ArrayBuffer }
  | { readonly kind: "kept-original"; readonly reason: string };

export const VIDEO_COMPRESSION_MIME_TYPE = "video/mp4";

/** The members of `Worker` the composer's end uses. */
export interface CompressionWorker {
  postMessage(request: VideoCompressionRequest): void;
  onmessage: ((event: MessageEvent<VideoCompressionAnswer>) => void) | null;
  onerror: ((event: ErrorEvent) => void) | null;
  terminate(): void;
}

/** The lazy chunk: Vite builds the Worker and mediabunny into their own file, loaded on first use. */
export function startVideoCompressionWorker(): CompressionWorker {
  return new Worker(new URL("./video-compression.worker.ts", import.meta.url), { type: "module" });
}

/** WebCodecs encoding, and Workers to run it in; without them every video goes as it is. */
export function canCompressVideosHere(): boolean {
  return typeof Worker !== "undefined" && typeof VideoEncoder !== "undefined";
}

/**
 * Compresses `file` in a Worker: resolves with the smaller MP4, or with `file` itself when the
 * Worker keeps the original or fails. Rejects with an `AbortError` when `signal` aborts; the
 * Worker is terminated, which stops its encoders and frees its memory.
 */
export function compressVideoInWorker(
  file: File,
  maximumAttachmentBytes: number | null,
  onprogress: (fraction: number) => void,
  signal: AbortSignal,
  startWorker: () => CompressionWorker = startVideoCompressionWorker,
): Promise<File> {
  if (signal.aborted) {
    return Promise.reject(signal.reason);
  }
  const worker = startWorker();
  return new Promise<File>((resolve, reject) => {
    // Answers already queued when it ends are not delivered.
    const finish = () => {
      worker.onmessage = null;
      worker.onerror = null;
      worker.terminate();
      signal.removeEventListener("abort", abort);
    };
    const abort = () => {
      finish();
      reject(signal.reason);
    };
    signal.addEventListener("abort", abort, { once: true });
    worker.onmessage = ({ data: answer }) => {
      switch (answer.kind) {
        case "progress":
          onprogress(answer.fraction);
          return;
        case "compressed":
          finish();
          resolve(new File([answer.bytes], compressedFileName(file.name), { type: VIDEO_COMPRESSION_MIME_TYPE }));
          return;
        case "kept-original":
          finish();
          resolve(file);
          return;
      }
    };
    // The chunk failed to load or the Worker crashed: the original still goes.
    worker.onerror = (event) => {
      console.warn("the video compression worker failed", event);
      finish();
      resolve(file);
    };
    worker.postMessage({ file, maximumAttachmentBytes });
  });
}

/** The members of the Worker's global scope its end uses. */
export interface CompressionWorkerScope {
  onmessage: ((event: MessageEvent<VideoCompressionRequest>) => void) | null;
  postMessage(answer: VideoCompressionAnswer, transfer?: Transferable[]): void;
}

/** What the Worker does with one file: the compressed bytes, or why the original stays. */
export type CompressVideo = (
  request: VideoCompressionRequest,
  onprogress: (fraction: number) => void,
) => Promise<{ bytes: ArrayBuffer } | { keptOriginalBecause: string }>;

/** The Worker's end: answers each request; a failure keeps the original rather than losing the file. */
export function answerVideoCompressionRequests(scope: CompressionWorkerScope, compress: CompressVideo): void {
  scope.onmessage = async ({ data: request }) => {
    try {
      const result = await compress(request, (fraction) => scope.postMessage({ kind: "progress", fraction }));
      if ("bytes" in result) {
        scope.postMessage({ kind: "compressed", bytes: result.bytes }, [result.bytes]);
      } else {
        scope.postMessage({ kind: "kept-original", reason: result.keptOriginalBecause });
      }
    } catch (error) {
      console.warn("compressing a video failed; the original is sent", error);
      scope.postMessage({ kind: "kept-original", reason: "failed" });
    }
  };
}
