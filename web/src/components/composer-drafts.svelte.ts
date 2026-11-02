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
      file,
      previewUrl: PREVIEWABLE_IMAGE_TYPES.includes(file.type) ? URL.createObjectURL(file) : null,
      status: "uploading",
      attachment: null,
      error: "",
    });
    // `push` stored a proxy of the literal, not the literal itself.
    const item = this.trayItems.at(-1);
    if (item?.key !== key) {
      throw new Error("the attached file is not the tray's last item");
    }
    return item;
  }

  removeTrayItem(key: number): void {
    revokePreviews(this.trayItems.filter((item) => item.key === key));
    this.trayItems = this.trayItems.filter((item) => item.key !== key);
  }

  clearTray(): void {
    revokePreviews(this.trayItems);
    this.trayItems = [];
  }

  /** Frees every preview, the open channel's and the ones aside; for when the chat goes away. */
  dispose(): void {
    revokePreviews(this.trayItems);
    for (const draft of this.#draftsAside.values()) {
      revokePreviews(draft.trayItems);
    }
  }
}

function revokePreviews(items: readonly AttachmentTrayItem[]): void {
  for (const item of items) {
    if (item.previewUrl) {
      URL.revokeObjectURL(item.previewUrl);
    }
  }
}
