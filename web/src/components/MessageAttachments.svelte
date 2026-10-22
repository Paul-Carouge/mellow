<script lang="ts">
  import { SvelteSet } from "svelte/reactivity";

  import { strings } from "../i18n";
  import { type Attachment, PREVIEWABLE_IMAGE_TYPES, type Snowflake, attachmentUrl } from "../lib/protocol";
  import { attachmentCountLabel, humanFileSize, middleTruncate } from "./format";
  import Icon from "./Icon.svelte";

  // Attachments under a message body (DESIGN.md §5.4 "Attachments"): previewable images inline,
  // anything else as a download card. A message still in the outbox is not readable by others
  // yet, so its files show as cards without links.
  interface Props {
    attachments: readonly Attachment[];
    isPending: boolean;
  }

  let { attachments, isPending }: Props = $props();

  /** CSS cannot cut a name in the middle; the card's width fits about this many characters. */
  const FILE_NAME_MAXIMUM_CHARACTERS = 40;

  const loadedImageIds = new SvelteSet<Snowflake>();
  // An image the instance could not serve falls back to its download card.
  const failedImageIds = new SvelteSet<Snowflake>();

  function isShownAsImage(attachment: Attachment): boolean {
    return !isPending && PREVIEWABLE_IMAGE_TYPES.includes(attachment.content_type) && !failedImageIds.has(attachment.id);
  }
</script>

<ul aria-label={attachmentCountLabel(attachments.length)} class="flex flex-col items-start gap-(--mw-component-attachment-gap) pt-(--mw-component-attachment-gap)">
  {#each attachments as attachment (attachment.id)}
    {@const size = humanFileSize(attachment.size_bytes)}
    {#if isShownAsImage(attachment)}
      <li class="w-full">
        <a
          href={attachmentUrl(attachment)}
          target="_blank"
          rel="noopener noreferrer"
          title={attachment.file_name}
          class="block w-fit max-w-[min(100%,var(--mw-component-attachment-image-max-width))] overflow-hidden rounded-(--mw-component-attachment-image-radius) bg-(--mw-component-attachment-image-placeholder) focus-visible:focus-ring"
        >
          <!-- Sized as the largest preview until loaded: lazy loading needs a box, and the list
               re-pins to the latest message when it shrinks. -->
          <img
            src={attachmentUrl(attachment)}
            alt={attachment.file_name}
            loading="lazy"
            class={[
              "block",
              loadedImageIds.has(attachment.id)
                ? "h-auto max-h-(--mw-component-attachment-image-max-height) w-auto max-w-full"
                : "h-(--mw-component-attachment-image-max-height) w-(--mw-component-attachment-image-max-width) max-w-full",
            ]}
            onload={() => loadedImageIds.add(attachment.id)}
            onerror={() => failedImageIds.add(attachment.id)}
          />
        </a>
      </li>
    {:else}
      <li
        class="flex w-full max-w-(--mw-component-attachment-file-max-width) items-center gap-(--mw-component-attachment-file-gap) rounded-(--mw-component-attachment-file-radius) border-(length:--mw-component-attachment-file-border-width) border-(--mw-component-attachment-file-border) bg-(--mw-component-attachment-file-background) p-(--mw-component-attachment-file-padding)"
      >
        <Icon name="file" class="size-(--mw-component-attachment-file-icon-size) text-(--mw-component-attachment-file-icon)" />
        <div class="min-w-0 flex-1">
          {#if isPending}
            <p title={attachment.file_name} class="truncate type-body-lg text-(--mw-component-message-pending-text)">
              <span aria-hidden="true">{middleTruncate(attachment.file_name, FILE_NAME_MAXIMUM_CHARACTERS)}</span>
              <span class="sr-only">{attachment.file_name}</span>
            </p>
          {:else}
            <a
              href={attachmentUrl(attachment)}
              download={attachment.file_name}
              title={attachment.file_name}
              class="flex min-h-(--mw-size-hit-desktop-min) items-center type-body-lg text-(--mw-component-attachment-file-name-text) hover:underline pointer-coarse:min-h-(--mw-size-hit-web-min)"
            >
              <span aria-hidden="true" class="truncate">
                {middleTruncate(attachment.file_name, FILE_NAME_MAXIMUM_CHARACTERS)}
              </span>
              <span class="sr-only">{strings.messages.download(attachment.file_name)}</span>
            </a>
          {/if}
          <p class="type-body-sm text-(--mw-component-attachment-file-size-text)">{size}</p>
        </div>
        {#if !isPending}
          <!-- The same download as the name, as a larger pointer target; keyboard and screen
               readers use the name link, so this one stays out of the Tab order. -->
          <a
            href={attachmentUrl(attachment)}
            download={attachment.file_name}
            tabindex="-1"
            aria-hidden="true"
            title={strings.messages.download(attachment.file_name)}
            class="grid size-(--mw-component-attachment-download-button-size) shrink-0 place-items-center rounded-sm text-(--mw-component-attachment-download-icon) transition-colors duration-(--mw-motion-duration-fast) hover:text-(--mw-component-attachment-download-icon-hover) pointer-coarse:size-(--mw-size-hit-web-min)"
          >
            <Icon name="download-simple" class="size-(--mw-component-attachment-download-icon-size)" />
          </a>
        {/if}
      </li>
    {/if}
  {/each}
</ul>
