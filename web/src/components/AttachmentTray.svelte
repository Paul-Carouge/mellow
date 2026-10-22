<script lang="ts" module>
  import type { Attachment } from "../lib/protocol";

  /** A file chosen for the next message: it uploads at once, then waits for Send (ADR-0020). */
  export interface AttachmentTrayItem {
    key: number;
    file: File;
    /** Object URL of a previewable image; revoked when the item leaves the tray. */
    previewUrl: string | null;
    status: "uploading" | "uploaded" | "failed";
    attachment: Attachment | null;
    /** Why the upload failed, as a sentence; empty otherwise. */
    error: string;
  }
</script>

<script lang="ts">
  import { strings } from "../i18n";
  import { humanFileSize, middleTruncate } from "./format";
  import Icon from "./Icon.svelte";
  import IconButton from "./IconButton.svelte";

  // The attachment tray inside the composer, above the field (DESIGN.md §5.5, §6.27).
  interface Props {
    items: readonly AttachmentTrayItem[];
    onremove: (key: number) => void;
  }

  let { items, onremove }: Props = $props();

  /** CSS cannot cut a name in the middle; an item's width fits about this many characters. */
  const FILE_NAME_MAXIMUM_CHARACTERS = 16;
</script>

<ul
  aria-label={strings.attachmentTray.filesToSend}
  class="flex gap-(--mw-component-composer-tray-gap) overflow-x-auto border-b-(length:--mw-component-composer-tray-item-border-width) border-(--mw-component-composer-tray-divider) p-(--mw-component-composer-tray-padding)"
>
  {#each items as item (item.key)}
    <li
      class={[
        "relative flex w-(--mw-component-composer-tray-item-width) shrink-0 flex-col gap-(--mw-component-composer-tray-item-gap) rounded-(--mw-component-composer-tray-item-radius) border-(length:--mw-component-composer-tray-item-border-width) bg-(--mw-component-composer-tray-item-background) p-(--mw-component-composer-tray-item-padding)",
        item.status === "failed" ? "border-(--mw-component-composer-tray-item-error-border)" : "border-transparent",
      ]}
    >
      <div
        class={[
          "grid h-(--mw-component-composer-tray-item-preview-height) place-items-center overflow-hidden rounded-(--mw-component-composer-tray-item-radius) bg-(--mw-component-composer-tray-item-preview-background)",
          item.status === "uploading" && "opacity-(--mw-component-composer-tray-item-uploading-opacity)",
        ]}
      >
        {#if item.previewUrl}
          <!-- The name below says what it is. -->
          <img src={item.previewUrl} alt="" class="size-full object-contain" />
        {:else}
          <Icon
            name="file"
            class="size-(--mw-component-composer-tray-item-file-icon-size) text-(--mw-component-composer-tray-item-file-icon)"
          />
        {/if}
      </div>
      <p title={item.file.name} class="truncate type-body text-(--mw-component-composer-tray-item-name-text)">
        <span aria-hidden="true">{middleTruncate(item.file.name, FILE_NAME_MAXIMUM_CHARACTERS)}</span>
        <span class="sr-only">{item.file.name}</span>
      </p>
      {#if item.status === "failed"}
        <p class="flex gap-(--mw-component-composer-tray-item-gap) type-body-sm text-(--mw-component-composer-tray-item-error-text)">
          <Icon name="warning-circle" class="size-(--mw-component-message-status-icon-size)" />
          <span class="min-w-0 break-words">{item.error}</span>
        </p>
      {:else}
        <p class="type-body-sm text-(--mw-component-composer-tray-item-status-text)">
          {item.status === "uploading" ? strings.attachmentTray.uploading : humanFileSize(item.file.size)}
        </p>
      {/if}
      <span
        class="absolute top-(--mw-component-composer-tray-item-remove-offset) right-(--mw-component-composer-tray-item-remove-offset) flex rounded-(--mw-component-action-bar-radius) border border-(--mw-component-action-bar-border) bg-(--mw-component-action-bar-background) shadow-(--mw-component-action-bar-shadow)"
      >
        <IconButton
          icon="trash"
          label={strings.attachmentTray.remove(item.file.name)}
          class="size-(--mw-component-action-bar-button-size) text-(--mw-component-action-bar-icon) hover:bg-(--mw-component-action-bar-button-background-hover) hover:text-(--mw-component-action-bar-icon-danger-hover)"
          iconClass="size-(--mw-component-action-bar-icon-size)"
          onclick={() => onremove(item.key)}
        />
      </span>
    </li>
  {/each}
</ul>
