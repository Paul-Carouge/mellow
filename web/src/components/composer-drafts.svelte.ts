import { PREVIEWABLE_IMAGE_TYPES, type Snowflake } from "../lib/protocol";
import type { AttachmentTrayItem } from "./AttachmentTray.svelte";

interface ChannelDraft {
  readonly text: string;
  readonly trayItems: AttachmentTrayItem[];
}

/**
 * Each channel keeps its own unsent text and attached files (DESIGN.md §5.5): the open channel's
 * are live here, the others wait aside. Files stay with their channel: they were uploaded to it.
 */
export class ComposerDrafts {
  text = $state("");
  trayItems = $state<AttachmentTrayItem[]>([]);
  #openChannelId: Snowflake | null = null;
  readonly #draftsAside = new Map<Snowflake, ChannelDraft>();
  // Shared by every channel: a tray brought back never meets a key it already holds.
  #nextTrayItemKey = 0;

  /** Puts the open channel's draft aside and brings back `channelId`'s; false when it is already open. */
  open(channelId: Snowflake | null): boolean {
    if (channelId === this.#openChannelId) {
      return false;
    }
    if (this.#openChannelId !== null) {
      this.#draftsAside.set(this.#openChannelId, { text: this.text, trayItems: this.trayItems });
    }
    this.#openChannelId = channelId;
    const draft = channelId === null ? undefined : this.#draftsAside.get(channelId);
    if (channelId !== null) {
      this.#draftsAside.delete(channelId);
    }
    this.text = draft?.text ?? "";
    this.trayItems = draft?.trayItems ?? [];
    return true;
  }

  /**
   * Adds `file` to the open channel's tray and returns the reactive item: updates to it reach the
   * tray, even after a channel switch put it aside.
   */
  attach(file: File): AttachmentTrayItem {
    const key = this.#nextTrayItemKey++;
    this.trayItems.push({
      key,
      channelId: this.#openChannelId,
      file,
      previewUrl: PREVIEWABLE_IMAGE_TYPES.includes(file.type) ? URL.createObjectURL(file) : null,
      status: "uploading",
      compressionProgress: 0,
      attachment: null,
      error: "",
      uploadAbort: new AbortController(),
      compressionAbort: new AbortController(),
    });
    // `push` stored a proxy of the literal, not the literal itself.
    const item = this.trayItems.at(-1);
    if (item?.key !== key) {
      throw new Error("the attached file is not the tray's last item");
    }
    return item;
  }

  /** Sets a failed file uploading again, with a fresh abort for its new request. */
  restartUpload(item: AttachmentTrayItem): void {
    item.status = "uploading";
    item.error = "";
    item.uploadAbort = new AbortController();
  }

  removeTrayItem(key: number): void {
    letGo(this.trayItems.filter((item) => item.key === key));
    this.trayItems = this.trayItems.filter((item) => item.key !== key);
  }

  clearTray(): void {
    letGo(this.trayItems);
    this.trayItems = [];
  }

  /** Stops every compression and upload and frees every preview, open and aside; for when the chat goes away. */
  dispose(): void {
    letGo(this.trayItems);
    for (const draft of this.#draftsAside.values()) {
      letGo(draft.trayItems);
    }
  }
}

/** Turns a picked file into the one to send: rejects when `signal` aborts. */
export type CompressVideo = (file: File, onprogress: (fraction: number) => void, signal: AbortSignal) => Promise<File>;

/**
 * A video is compressed first (ADR-0046), with `compress` (null where the browser cannot), then
 * uploaded with `upload`: compressed, or the original when the reader asked for it or compression
 * failed, and not at all when the item left the tray meanwhile. Other files upload at once.
 */
export async function compressThenUpload(
  item: AttachmentTrayItem,
  compress: CompressVideo | null,
  upload: (item: AttachmentTrayItem) => Promise<void> | void,
): Promise<void> {
  if (compress !== null && item.file.type.startsWith("video/")) {
    item.status = "compressing";
    try {
      item.file = await compress(
        item.file,
        (fraction) => (item.compressionProgress = fraction),
        item.compressionAbort.signal,
      );
    } catch (error) {
      // Aborted by "Send the original" or a removal, or the Worker could not start: the original goes.
      if (!item.compressionAbort.signal.aborted) {
        console.warn("couldn't start compressing a video", error);
      }
    }
    if (item.uploadAbort.signal.aborted) {
      return;
    }
    item.status = "uploading";
  }
  await upload(item);
}

/** Items leaving the tray: their compressions and uploads stop and their previews are freed. */
function letGo(items: readonly AttachmentTrayItem[]): void {
  for (const item of items) {
    // Upload first: a compression stopping with its upload still live would send the original.
    item.uploadAbort.abort();
    item.compressionAbort.abort();
    if (item.previewUrl) {
      URL.revokeObjectURL(item.previewUrl);
    }
  }
}
