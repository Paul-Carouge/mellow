<script lang="ts">
  import { tick } from "svelte";
  import { SvelteSet } from "svelte/reactivity";

  import { strings } from "../i18n";
  import { type Attachment, PLAYABLE_AUDIO_TYPES, type Snowflake, attachmentPreviewUrl, attachmentUrl } from "../lib/protocol";
  import { attachmentDisplay, imagePreviewWidth, pauseOtherMediaWhenOneStarts, videoBoxSize } from "./attachment-preview";
  import { attachmentCountLabel, fileNameStartAndEnd, humanFileSize } from "./format";
  import Icon from "./Icon.svelte";
  import { retryImageOnceThenMarkFailed } from "./image-retry";
  import { isPlainPrimaryClick, viewablePictures } from "./picture-viewer";
  import { openPictureViewer } from "./picture-viewer-request.svelte";

  // Attachments under a message body (DESIGN.md §5.4 "Attachments"): previewable images inline,
  // videos and sounds as players (ADR-0038), anything else as a download card. A message still in the outbox is not readable by others
  // yet, so its files show as cards without links.
  interface Props {
    attachments: readonly Attachment[];
    isPending: boolean;
    /** For the picture viewer's caption: the sender as the message header shows them. */
    authorName: string;
    /** Null only while the message is still in the outbox, when no picture is shown. */
    sentAtUnixMilliseconds: number | null;
  }

  let { attachments, isPending, authorName, sentAtUnixMilliseconds }: Props = $props();

  const id = $props.id();
  const viewerPictures = $derived(viewablePictures(attachments));

  function openViewer(event: MouseEvent, attachment: Attachment) {
    const index = viewerPictures.findIndex((picture) => picture.id === attachment.id);
    // A modified or middle click keeps the link's own behaviour (a new tab, a download).
    if (index < 0 || sentAtUnixMilliseconds === null || !isPlainPrimaryClick(event) || !(event.currentTarget instanceof HTMLElement)) {
      return;
    }
    event.preventDefault();
    openPictureViewer({ pictures: viewerPictures, index, authorName, sentAtUnixMilliseconds, opener: event.currentTarget });
  }

  const loadedImageIds = new SvelteSet<Snowflake>();
  // An image the instance could not serve, even on its retry, falls back to its download card.
  const retriedImageIds = new SvelteSet<Snowflake>();
  const failedImageIds = new SvelteSet<Snowflake>();

  // A video or sound this browser cannot play (its codec, or the instance did not serve it) becomes
  // its download card with a note; nothing retries, since each try decrypts the whole file.
  const failedMediaIds = new SvelteSet<Snowflake>();

  function showDownloadCardAfterPlaybackFailed(event: Event, attachment: Attachment) {
    const hadFocus = event.currentTarget === document.activeElement;
    failedMediaIds.add(attachment.id);
    // The player that held the focus is gone: hand it to the card's download link.
    if (hadFocus) {
      void tick().then(() => document.getElementById(downloadLinkId(attachment))?.focus());
    }
  }

  function downloadLinkId(attachment: Attachment): string {
    return `${id}-download-${attachment.id}`;
  }
</script>

<!-- Cut in the middle at any width: the start gives way, the extension stays (DESIGN.md §5.4). -->
{#snippet fileName(name: string)}
  {@const parts = fileNameStartAndEnd(name)}
  <span aria-hidden="true" class="flex min-w-0"
    ><span class="truncate">{parts.start}</span><span class="shrink-0 whitespace-pre">{parts.end}</span></span
  >
{/snippet}

<ul aria-label={attachmentCountLabel(attachments.length)} class="flex flex-col items-start gap-(--mw-component-attachment-gap) pt-(--mw-component-attachment-gap)">
  {#each attachments as attachment (attachment.id)}
    {@const size = humanFileSize(attachment.size_bytes)}
    {@const display = attachmentDisplay(attachment, isPending, failedImageIds.has(attachment.id) || failedMediaIds.has(attachment.id))}
    {#if display === "image"}
      {@const previewWidth = imagePreviewWidth(attachment)}
      <li class="w-full">
        <a
          href={attachmentUrl(attachment)}
          target="_blank"
          rel="noopener noreferrer"
          title={attachment.file_name}
          onclick={(event) => openViewer(event, attachment)}
          style:width={previewWidth}
          class="block w-fit max-w-[min(100%,var(--mw-component-attachment-image-max-width))] overflow-hidden rounded-(--mw-component-attachment-image-radius) bg-(--mw-component-attachment-image-placeholder) focus-visible:focus-ring"
        >
          <!-- With its size known, the box is the loaded picture's from the start, so nothing
               moves. Without it (older uploads), sized as the largest preview until loaded: lazy
               loading needs a box, and the list re-pins to the latest message when it shrinks. -->
          <!-- A new element once retried: the same src on the old one would not load again. The
               list shows the preview; the link opens the full picture. -->
          {#key retriedImageIds.has(attachment.id)}
            <img
              src={attachmentPreviewUrl(attachment)}
              alt={attachment.file_name}
              width={previewWidth === undefined ? undefined : attachment.width}
              height={previewWidth === undefined ? undefined : attachment.height}
              loading="lazy"
              decoding="async"
              class={[
                "block",
                previewWidth !== undefined
                  ? "h-auto w-full"
                  : loadedImageIds.has(attachment.id)
                  ? "h-auto max-h-(--mw-component-attachment-image-max-height) w-auto max-w-full"
                  : "h-(--mw-component-attachment-image-max-height) w-(--mw-component-attachment-image-max-width) max-w-full",
              ]}
              onload={() => loadedImageIds.add(attachment.id)}
              onerror={() => retryImageOnceThenMarkFailed(attachment.id, retriedImageIds, failedImageIds)}
            />
          {/key}
        </a>
      </li>
    {:else if display === "video"}
      {@const box = videoBoxSize(attachment)}
      <!-- A small screen of the media stage: the picture letterboxed, then a band under it, never
           over it (docs/design/web-inline-media.md). Nothing loads until it plays. -->
      <li class="w-full">
        <figure
          style:width={box.width}
          class="max-w-full overflow-hidden rounded-(--mw-component-attachment-image-radius) bg-(--mw-component-attachment-video-background) [--mw-color-focus:var(--mw-component-attachment-video-focus-ring)]"
        >
          <!-- svelte-ignore a11y_media_has_caption (uploads come without caption tracks) -->
          <video
            src={attachmentUrl(attachment)}
            controls
            preload="none"
            playsinline
            aria-label={strings.messages.videoPlayer(attachment.file_name)}
            style:aspect-ratio={box.aspectRatio}
            class="block w-full object-contain focus-visible:focus-ring-inset"
            onplay={(event) => pauseOtherMediaWhenOneStarts(event.currentTarget)}
            onerror={(event) => showDownloadCardAfterPlaybackFailed(event, attachment)}
          ></video>
          <figcaption
            class="flex items-center gap-(--mw-component-attachment-file-gap) px-(--mw-component-attachment-video-band-padding-inline) py-(--mw-component-attachment-video-band-padding-block)"
          >
            <a
              href={attachmentUrl(attachment)}
              download={attachment.file_name}
              title={attachment.file_name}
              class="flex min-h-(--mw-size-hit-desktop-min) min-w-0 flex-1 items-center type-body text-(--mw-component-attachment-video-name-text) hover:underline focus-visible:focus-ring-inset pointer-coarse:min-h-(--mw-size-hit-web-min)"
            >
              {@render fileName(attachment.file_name)}
              <span class="sr-only">{strings.messages.download(attachment.file_name)}</span>
            </a>
            <span class="shrink-0 type-body-sm text-(--mw-component-attachment-video-size-text)">{size}</span>
            <a
              href={attachmentUrl(attachment)}
              download={attachment.file_name}
              tabindex="-1"
              aria-hidden="true"
              title={strings.messages.download(attachment.file_name)}
              class="grid size-(--mw-component-attachment-download-button-size) shrink-0 place-items-center rounded-sm text-(--mw-component-attachment-video-download-icon) transition-colors duration-(--mw-motion-duration-fast) hover:bg-(--mw-component-attachment-video-download-background-hover) hover:text-(--mw-component-attachment-video-download-icon-hover) pointer-coarse:size-(--mw-size-hit-web-min)"
            >
              <Icon name="download-simple" class="size-(--mw-component-attachment-download-icon-size)" />
            </a>
          </figcaption>
        </figure>
      </li>
    {:else}
      {@const hasPlaybackFailed = failedMediaIds.has(attachment.id)}
      <li
        class="w-full max-w-(--mw-component-attachment-file-max-width) rounded-(--mw-component-attachment-file-radius) border-(length:--mw-component-attachment-file-border-width) border-(--mw-component-attachment-file-border) bg-(--mw-component-attachment-file-background) p-(--mw-component-attachment-file-padding)"
      >
        <div class="flex items-center gap-(--mw-component-attachment-file-gap)">
          <Icon
            name={PLAYABLE_AUDIO_TYPES.includes(attachment.content_type) ? "file-audio" : "file"}
            class="size-(--mw-component-attachment-file-icon-size) shrink-0 text-(--mw-component-attachment-file-icon)"
          />
          <div class="min-w-0 flex-1">
            {#if isPending}
              <p title={attachment.file_name} class="flex type-body-lg text-(--mw-component-message-pending-text)">
                {@render fileName(attachment.file_name)}
                <span class="sr-only">{attachment.file_name}</span>
              </p>
            {:else}
              <a
                id={downloadLinkId(attachment)}
                href={attachmentUrl(attachment)}
                download={attachment.file_name}
                title={attachment.file_name}
                aria-describedby={hasPlaybackFailed ? `${downloadLinkId(attachment)}-note` : undefined}
                class="flex min-h-(--mw-size-hit-desktop-min) items-center type-body-lg text-(--mw-component-attachment-file-name-text) hover:underline pointer-coarse:min-h-(--mw-size-hit-web-min)"
              >
                {@render fileName(attachment.file_name)}
                <span class="sr-only">{strings.messages.download(attachment.file_name)}</span>
              </a>
            {/if}
            <p class="type-body-sm text-(--mw-component-attachment-file-size-text)">{size}</p>
            {#if hasPlaybackFailed}
              <p id={`${downloadLinkId(attachment)}-note`} class="type-body-sm text-(--mw-component-attachment-playback-note-text)">
                {strings.messages.cannotPlay}
              </p>
            {/if}
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
        </div>
        {#if display === "audio"}
          <audio
            src={attachmentUrl(attachment)}
            controls
            preload="none"
            aria-label={strings.messages.audioPlayer(attachment.file_name)}
            class="mt-(--mw-component-attachment-file-gap) block w-full focus-visible:focus-ring"
            onplay={(event) => pauseOtherMediaWhenOneStarts(event.currentTarget)}
            onerror={(event) => showDownloadCardAfterPlaybackFailed(event, attachment)}
          ></audio>
        {/if}
      </li>
    {/if}
  {/each}
</ul>
