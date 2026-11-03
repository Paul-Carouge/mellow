<script lang="ts">
  import { onMount, tick } from "svelte";

  import { strings } from "../i18n";
  import { type Attachment, attachmentPreviewUrl, attachmentUrl } from "../lib/protocol";
  import { forgetMediaWhenItStops, pauseOtherMediaWhenOneStarts } from "./attachment-preview";
  import { fileNameStartAndEnd, humanFileSize, messageTimestampLabel } from "./format";
  import Icon from "./Icon.svelte";
  import IconButton from "./IconButton.svelte";
  import {
    type PixelSize,
    actualSizeScrollStart,
    attachmentPixelSize,
    FALLBACK_VIDEO_PIXEL_SIZE,
    fittedPictureSize,
    fittedVideoSize,
    isPictureLargerThanBox,
    isViewerVideo,
    wrappedPictureIndex,
  } from "./picture-viewer";
  import { IMAGE_RETRY_DELAY_MILLISECONDS } from "./image-retry";
  import { durationTokenInMilliseconds, lengthTokenInPixels } from "./tokens";

  // docs/design/web-picture-viewer.md: a message's pictures and videos large over the app, on the opaque
  // media stage, with the controls in a band under it. A native modal <dialog> keeps the app inert.
  interface Props {
    media: readonly Attachment[];
    initialIndex: number;
    /** Where the first video shown goes on from, when it was playing in the list. */
    initialVideoStartSeconds?: number;
    authorName: string;
    sentAtUnixMilliseconds: number;
    /** The picture link or "Agrandir" button that opened the viewer; focus goes back to it when it closes. */
    opener: HTMLElement | null;
    onclose: () => void;
  }

  let { media, initialIndex, initialVideoStartSeconds, authorName, sentAtUnixMilliseconds, opener, onclose }: Props = $props();

  const id = $props.id();
  let dialog: HTMLDialogElement;
  let stage: HTMLDivElement;
  let closeButton = $state<HTMLButtonElement | null>(null);
  // The viewer keeps the pictures and videos it opened with; the list behind it may change meanwhile.
  // svelte-ignore state_referenced_locally
  let index = $state(Math.min(Math.max(initialIndex, 0), media.length - 1));
  const shownMedia = $derived(media[index]);
  const isVideo = $derived(shownMedia !== undefined && isViewerVideo(shownMedia));
  // Only the video the viewer opened on goes on from the list's position.
  // svelte-ignore state_referenced_locally
  let videoStartSeconds = initialVideoStartSeconds ?? 0;
  // Nothing plays by itself (ADR-0038): only the video opened from the list's "Agrandir" does.
  // svelte-ignore state_referenced_locally
  let shouldAutoplayVideo = $state(initialVideoStartSeconds !== undefined);

  let stageWidth = $state(0);
  let stageHeight = $state(0);
  let isActualSize = $state(false);
  // Per shown picture: reset by `showMediaAt` and by "Réessayer".
  let loadedSize = $state<PixelSize | null>(null);
  let isFullLoaded = $state(false);
  let hasFullRetried = $state(false);
  let isFullFailed = $state(false);
  let isPreviewFailed = $state(false);
  // Bumped for every new attempt: a load or error from an older <img> (the browser keeps
  // downloading a removed one) must not touch the picture shown now.
  let loadGeneration = $state(0);
  let isLoadingNoticeDue = $state(false);
  let loadingNoticeTimer: ReturnType<typeof setTimeout> | undefined;
  let retryTimer: ReturnType<typeof setTimeout> | undefined;

  let announcement = $state("");
  let isCopied = $state(false);
  let isCopyFailed = $state(false);
  let isPressStartedOnStage = false;
  let copiedTimer: ReturnType<typeof setTimeout> | undefined;

  const stagePadding = lengthTokenInPixels("component-picture-viewer-stage-padding");
  // A video that has not loaded yet holds the list's 16:9 box, so its controls are not a 300 × 150 stamp.
  const naturalSize = $derived(
    shownMedia ? (attachmentPixelSize(shownMedia) ?? loadedSize ?? (isVideo ? FALLBACK_VIDEO_PIXEL_SIZE : null)) : null,
  );
  const box = $derived({ width: Math.max(1, stageWidth - 2 * stagePadding), height: Math.max(1, stageHeight - 2 * stagePadding) });
  const isZoomable = $derived(!isVideo && naturalSize !== null && isPictureLargerThanBox(naturalSize, box));
  // A window grown until the picture fits leaves actual size by itself.
  const isShownAtActualSize = $derived(isActualSize && isZoomable);
  const shownSize = $derived(
    naturalSize === null
      ? null
      : isVideo
        ? fittedVideoSize(naturalSize, box)
        : isShownAtActualSize
          ? naturalSize
          : fittedPictureSize(naturalSize, box),
  );
  const caption = $derived(
    shownMedia
      ? strings.pictureViewer.caption(authorName, messageTimestampLabel(sentAtUnixMilliseconds, new Date()), humanFileSize(shownMedia.size_bytes))
      : "",
  );
  const hasNothingToShow = $derived(isFullFailed && isPreviewFailed);

  async function showMediaAt(nextIndex: number) {
    index = nextIndex;
    videoStartSeconds = 0;
    shouldAutoplayVideo = false;
    isActualSize = false;
    clearTimeout(copiedTimer);
    isCopied = false;
    resetLoading();
    const shown = media[index];
    if (shown) {
      announcement = strings.pictureViewer.pictureChanged(index + 1, media.length, shown.file_name);
    }
    await keepFocusInViewer();
  }

  /** A control that held focus may be gone (the zoom toggle on a small picture, "Réessayer"):
   * focus would fall to the page, where the arrow keys no longer reach the dialog. */
  async function keepFocusInViewer() {
    await tick();
    if (!dialog.contains(document.activeElement)) {
      closeButton?.focus();
    }
  }

  function retry() {
    resetLoading();
    void keepFocusInViewer();
  }

  function resetLoading() {
    loadedSize = null;
    isFullLoaded = false;
    hasFullRetried = false;
    isFullFailed = false;
    isPreviewFailed = false;
    isCopyFailed = false;
    loadGeneration += 1;
    clearTimeout(retryTimer);
    scheduleLoadingNotice();
  }

  /** The notice (and its announcement) only for a slow picture: a cached one never shows it. */
  function scheduleLoadingNotice() {
    isLoadingNoticeDue = false;
    clearTimeout(loadingNoticeTimer);
    loadingNoticeTimer = setTimeout(
      () => (isLoadingNoticeDue = true),
      durationTokenInMilliseconds("component-picture-viewer-status-delay"),
    );
  }

  function naturalSizeOf(image: EventTarget): PixelSize | null {
    return image instanceof HTMLImageElement && image.naturalWidth > 0 ? { width: image.naturalWidth, height: image.naturalHeight } : null;
  }

  function step(direction: -1 | 1) {
    void showMediaAt(wrappedPictureIndex(index, direction, media.length));
  }

  function onFullError(generation: number) {
    if (generation !== loadGeneration) {
      return;
    }
    // One retry with a new element (the same src on the old one would not load again), after the
    // pause a busy instance needs, as the message list does.
    if (hasFullRetried) {
      isFullFailed = true;
    } else {
      hasFullRetried = true;
      retryTimer = setTimeout(() => {
        if (generation === loadGeneration) {
          loadGeneration += 1;
        }
      }, IMAGE_RETRY_DELAY_MILLISECONDS);
    }
  }

  /** As in the list: a network error keeps the player (play resumes), anything else cannot play here. */
  function onVideoError(event: Event, generation: number) {
    const player = event.currentTarget;
    if (generation !== loadGeneration) {
      return;
    }
    if (player instanceof HTMLMediaElement && player.error?.code === MediaError.MEDIA_ERR_NETWORK) {
      isFullLoaded = true;
      return;
    }
    isFullFailed = true;
    isPreviewFailed = true;
    void keepFocusInViewer();
  }

  async function toggleActualSize(event: MouseEvent | KeyboardEvent) {
    if (!isZoomable || naturalSize === null) {
      return;
    }
    const pictureBox = (event.currentTarget as HTMLElement).getBoundingClientRect();
    const stageBox = stage.getBoundingClientRect();
    // A click keeps its point under the pointer; the keyboard centres the picture.
    const isPointer = event instanceof MouseEvent && event.detail > 0;
    const ratioX = isPointer ? (event.clientX - pictureBox.left) / pictureBox.width : 0.5;
    const ratioY = isPointer ? (event.clientY - pictureBox.top) / pictureBox.height : 0.5;
    const pointerX = isPointer ? event.clientX - stageBox.left : stage.clientWidth / 2;
    const pointerY = isPointer ? event.clientY - stageBox.top : stage.clientHeight / 2;
    isActualSize = !isShownAtActualSize;
    if (isActualSize) {
      await tick();
      stage.scrollTo({
        left: actualSizeScrollStart(ratioX, naturalSize.width, stagePadding, pointerX),
        top: actualSizeScrollStart(ratioY, naturalSize.height, stagePadding, pointerY),
      });
    }
  }

  async function copyLink() {
    if (!shownMedia) {
      return;
    }
    try {
      await navigator.clipboard.writeText(new URL(attachmentUrl(shownMedia), window.location.origin).href);
    } catch (error) {
      console.warn("copying the picture link failed", error);
      isCopyFailed = true;
      return;
    }
    isCopyFailed = false;
    isCopied = true;
    // Emptied first, so a second copy is announced again.
    announcement = "";
    await tick();
    announcement = strings.pictureViewer.linkCopied;
    clearTimeout(copiedTimer);
    copiedTimer = setTimeout(() => (isCopied = false), durationTokenInMilliseconds("component-picture-viewer-copied-label-duration"));
  }

  function onkeydown(event: KeyboardEvent) {
    // At actual size the arrows pan the stage, as the browser does by itself; on a focused video they seek.
    if (isShownAtActualSize || event.target instanceof HTMLMediaElement || media.length < 2 || event.altKey || event.ctrlKey || event.metaKey) {
      return;
    }
    if (event.key === "ArrowLeft" || event.key === "ArrowRight") {
      event.preventDefault();
      step(event.key === "ArrowLeft" ? -1 : 1);
    }
  }

  onMount(() => {
    dialog.showModal();
    closeButton?.focus();
    scheduleLoadingNotice();
    return () => {
      clearTimeout(copiedTimer);
      clearTimeout(loadingNoticeTimer);
      clearTimeout(retryTimer);
      dialog.close();
      if (opener?.isConnected) {
        opener.focus();
      }
    };
  });
</script>

{#snippet control(icon: "caret-left" | "caret-right", label: string, direction: -1 | 1)}
  <IconButton
    {icon}
    {label}
    class="size-(--mw-component-picture-viewer-control-size) rounded-(--mw-component-picture-viewer-control-radius) text-(--mw-component-picture-viewer-control-icon) hover:bg-(--mw-component-picture-viewer-control-background-hover) focus-visible:focus-ring"
    iconClass="size-(--mw-component-picture-viewer-control-icon-size)"
    onclick={() => step(direction)}
  />
{/snippet}

<dialog
  bind:this={dialog}
  aria-labelledby="{id}-name"
  aria-describedby="{id}-caption {id}-position"
  class="picture-viewer fixed inset-0 m-0 flex h-dvh max-h-none w-dvw max-w-none flex-col bg-(--mw-component-picture-viewer-background) p-0 text-(--mw-component-picture-viewer-name-text) backdrop:bg-transparent"
  oncancel={(event) => {
    event.preventDefault();
    onclose();
  }}
  {onkeydown}
>
  <div class="flex min-h-0 flex-1 pt-[env(safe-area-inset-top)]">
    <!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions (Esc and the close button do the same from the keyboard) -->
    <div
      bind:this={stage}
      bind:clientWidth={stageWidth}
      bind:clientHeight={stageHeight}
      class={[
        "stage relative grid min-w-0 flex-1 p-(--mw-component-picture-viewer-stage-padding)",
        isShownAtActualSize ? "overflow-auto" : "overflow-hidden",
      ]}
      onpointerdown={(event) => (isPressStartedOnStage = event.target === stage)}
      onclick={(event) => {
        // A drag that started on the picture and ended beside it is not a click on the room.
        if (event.target === stage && isPressStartedOnStage) {
          onclose();
        }
      }}
    >
      {#if shownMedia && !hasNothingToShow}
        <svelte:element
          this={isZoomable ? "button" : "div"}
          type={isZoomable ? "button" : undefined}
          aria-label={isZoomable ? strings.pictureViewer.actualSize : undefined}
          aria-pressed={isZoomable ? isShownAtActualSize : undefined}
          style:width={shownSize ? `${shownSize.width}px` : undefined}
          style:height={shownSize ? `${shownSize.height}px` : undefined}
          class={[
            "picture relative m-auto block overflow-hidden rounded-(--mw-component-picture-viewer-picture-radius) p-0",
            isZoomable && (isShownAtActualSize ? "cursor-zoom-out" : "cursor-zoom-in"),
            isZoomable && "focus-visible:focus-ring",
          ]}
          onclick={isZoomable ? toggleActualSize : undefined}
        >
          {#key `${shownMedia.id}:${loadGeneration}`}
            {@const shownGeneration = loadGeneration}
            {#if isVideo}
              {#if !isFullFailed}
                <!-- svelte-ignore a11y_media_has_caption (uploads come without caption tracks) -->
                <video
                  src={attachmentUrl(shownMedia)}
                  controls
                  autoplay={shouldAutoplayVideo}
                  preload={shouldAutoplayVideo ? "auto" : "none"}
                  playsinline
                  aria-label={strings.messages.videoPlayer(shownMedia.file_name)}
                  class={["block bg-(--mw-component-picture-viewer-background)", shownSize ? "absolute inset-0 size-full" : "max-h-full max-w-full"]}
                  onloadedmetadata={(event) => {
                    if (shownGeneration === loadGeneration) {
                      const player = event.currentTarget;
                      loadedSize = player.videoWidth > 0 ? { width: player.videoWidth, height: player.videoHeight } : loadedSize;
                      isFullLoaded = true;
                      if (videoStartSeconds > 0) {
                        player.currentTime = videoStartSeconds;
                      }
                    }
                  }}
                  onplay={(event) => pauseOtherMediaWhenOneStarts(event.currentTarget)}
                  onpause={(event) => forgetMediaWhenItStops(event.currentTarget)}
                  onerror={(event) => onVideoError(event, shownGeneration)}
                ></video>
              {/if}
            <!-- The list's preview, already in the cache, holds the place until the full file arrives. -->
            {:else if !isFullLoaded && !isPreviewFailed}
              <img
                src={attachmentPreviewUrl(shownMedia)}
                alt={shownMedia.file_name}
                class={["block", shownSize ? "absolute inset-0 size-full" : "max-h-full max-w-full"]}
                onload={(event) => {
                  if (shownGeneration === loadGeneration) {
                    loadedSize ??= naturalSizeOf(event.currentTarget);
                  }
                }}
                onerror={() => {
                  if (shownGeneration === loadGeneration) {
                    isPreviewFailed = true;
                  }
                }}
              />
            {/if}
            {#if !isVideo && !isFullFailed}
              <img
                src={attachmentUrl(shownMedia)}
                alt={shownMedia.file_name}
                decoding="async"
                class={["block", shownSize ? "absolute inset-0 size-full" : "max-h-full max-w-full", !isFullLoaded && "invisible"]}
                onload={(event) => {
                  if (shownGeneration === loadGeneration) {
                    loadedSize = naturalSizeOf(event.currentTarget) ?? loadedSize;
                    isFullLoaded = true;
                  }
                }}
                onerror={() => onFullError(shownGeneration)}
              />
            {/if}
          {/key}
        </svelte:element>
      {/if}

      <div role="status" class="pointer-events-none absolute inset-x-0 bottom-(--mw-component-picture-viewer-stage-padding) flex justify-center px-(--mw-component-picture-viewer-stage-padding)">
        {#if isFullFailed || isCopyFailed}
          <p
            class="pointer-events-auto flex flex-wrap items-center gap-(--mw-component-picture-viewer-status-gap) rounded-(--mw-component-picture-viewer-status-radius) bg-(--mw-component-picture-viewer-status-background) p-(--mw-component-picture-viewer-status-padding) type-body text-(--mw-component-picture-viewer-status-text)"
          >
            <Icon name="warning-circle" class="size-(--mw-component-picture-viewer-status-icon-size)" />
            <span>
              {isCopyFailed && !isFullFailed
                ? strings.pictureViewer.copyLinkFailed
                : isVideo
                  ? strings.pictureViewer.videoFailed
                  : hasNothingToShow
                  ? strings.pictureViewer.pictureFailed
                  : strings.pictureViewer.fullSizeFailed}
            </span>
            <!-- A video that cannot play here would fail again: download is the way out. -->
            {#if isFullFailed && !isVideo}
              <button
                type="button"
                class="min-h-(--mw-component-picture-viewer-control-size) rounded-(--mw-component-picture-viewer-control-radius) border-(length:--mw-component-picture-viewer-retry-border-width) border-(--mw-component-picture-viewer-retry-border) px-(--mw-component-picture-viewer-status-padding) font-semibold hover:bg-(--mw-component-picture-viewer-control-background-hover) focus-visible:focus-ring"
                onclick={retry}>{strings.common.tryAgain}</button
              >
            {/if}
          </p>
        {:else if shownMedia && !isFullLoaded && isLoadingNoticeDue && (!isVideo || shouldAutoplayVideo)}
          <!-- Shown only when the full file is slow: a cached picture never flashes it. -->
          <p
            class="animate-[mw-fade-in_var(--mw-motion-duration-base)_var(--mw-motion-easing-standard)_both] rounded-(--mw-component-picture-viewer-status-radius) bg-(--mw-component-picture-viewer-status-background) p-(--mw-component-picture-viewer-status-padding) type-body text-(--mw-component-picture-viewer-status-text)"
          >
            {isVideo ? strings.pictureViewer.loadingVideo : strings.pictureViewer.loading}
          </p>
        {/if}
      </div>
    </div>
  </div>

  {#if shownMedia}
    {@const nameParts = fileNameStartAndEnd(shownMedia.file_name)}
    <div
      class="flex min-h-(--mw-component-picture-viewer-band-min-height) flex-wrap items-center justify-between gap-x-(--mw-component-picture-viewer-band-gap) gap-y-(--mw-component-picture-viewer-band-row-gap) px-(--mw-component-picture-viewer-band-padding-inline) pt-(--mw-component-picture-viewer-band-padding-block) pb-[calc(var(--mw-component-picture-viewer-band-padding-block)+env(safe-area-inset-bottom))]"
    >
      <div class="min-w-0 flex-[1_1_var(--mw-component-picture-viewer-caption-min-width)]">
        <h2 id="{id}-name" title={shownMedia.file_name} class="flex type-body-lg text-(--mw-component-picture-viewer-name-text)">
          <span aria-hidden="true" class="flex min-w-0"
            ><span class="truncate">{nameParts.start}</span><span class="shrink-0 whitespace-pre">{nameParts.end}</span></span
          >
          <span class="sr-only">{shownMedia.file_name}</span>
        </h2>
        <p id="{id}-caption" class="truncate type-body-sm text-(--mw-component-picture-viewer-meta-text)">{caption}</p>
      </div>

      <div class="flex flex-1 flex-wrap items-center justify-between gap-(--mw-component-picture-viewer-band-gap)">
        {#if media.length > 1}
          <div class="flex items-center gap-(--mw-component-picture-viewer-control-gap)">
            {@render control("caret-left", strings.pictureViewer.previous, -1)}
            <span aria-hidden="true" class="type-body text-(--mw-component-picture-viewer-position-text) tabular-nums"
              >{strings.pictureViewer.position(index + 1, media.length)}</span
            >
            <span id="{id}-position" class="sr-only">{strings.pictureViewer.positionLabel(index + 1, media.length)}</span>
            {@render control("caret-right", strings.pictureViewer.next, 1)}
          </div>
        {/if}

        <div class="ms-auto flex items-center gap-(--mw-component-picture-viewer-control-gap)">
          {#snippet linkIcon(icon: "download-simple" | "arrow-square-out")}
            <Icon name={icon} class="size-(--mw-component-picture-viewer-control-icon-size)" />
          {/snippet}
          <a
            href={attachmentUrl(shownMedia)}
            download={shownMedia.file_name}
            aria-label={strings.messages.download(shownMedia.file_name)}
            title={strings.pictureViewer.downloadTooltip}
            class="control">{@render linkIcon("download-simple")}</a
          >
          <IconButton
            icon={isCopied ? "check" : "copy"}
            label={isCopied ? strings.pictureViewer.linkCopied : strings.pictureViewer.copyLink}
            class="size-(--mw-component-picture-viewer-control-size) rounded-(--mw-component-picture-viewer-control-radius) text-(--mw-component-picture-viewer-control-icon) hover:bg-(--mw-component-picture-viewer-control-background-hover) focus-visible:focus-ring"
            iconClass="size-(--mw-component-picture-viewer-control-icon-size)"
            onclick={copyLink}
          />
          <a
            href={attachmentUrl(shownMedia)}
            target="_blank"
            rel="noopener noreferrer"
            aria-label={strings.pictureViewer.openOriginal}
            title={strings.pictureViewer.openOriginalTooltip}
            class="control">{@render linkIcon("arrow-square-out")}</a
          >
          <IconButton
            bind:element={closeButton}
            icon="x"
            label={strings.common.close}
            class="size-(--mw-component-picture-viewer-control-size) rounded-(--mw-component-picture-viewer-control-radius) text-(--mw-component-picture-viewer-control-icon) hover:bg-(--mw-component-picture-viewer-control-background-hover) focus-visible:focus-ring"
            iconClass="size-(--mw-component-picture-viewer-control-icon-size)"
            onclick={onclose}
          />
        </div>
      </div>
    </div>
  {/if}

  <p class="sr-only" aria-live="polite">{announcement}</p>
</dialog>

<style>
  /* The stage is dark in both themes, so its focus ring is the dark theme's sky (DESIGN.md v0.43). */
  .picture-viewer {
    --mw-color-focus: var(--mw-component-picture-viewer-focus-ring);
  }
  .control {
    display: inline-grid;
    place-items: center;
    width: var(--mw-component-picture-viewer-control-size);
    height: var(--mw-component-picture-viewer-control-size);
    border-radius: var(--mw-component-picture-viewer-control-radius);
    color: var(--mw-component-picture-viewer-control-icon);
    transition: background-color var(--mw-motion-duration-fast) var(--mw-motion-easing-standard);
  }
  .control:hover {
    background-color: var(--mw-component-picture-viewer-control-background-hover);
  }
  .control:focus-visible {
    outline: var(--mw-size-focus-ring-width) solid var(--mw-color-focus);
    outline-offset: var(--mw-size-focus-ring-offset);
  }
  /* The room fades in and the picture grows a little (no settle overshoot). Reduced motion: at
     once, through the motion token (component aliases keep their value). On the stage, which is
     never recreated, so changing pictures never replays it. */
  .picture-viewer[open] {
    animation: mw-fade-in var(--mw-motion-duration-base) var(--mw-motion-easing-enter) both;
  }
  .picture-viewer[open] .stage {
    animation: picture-viewer-in var(--mw-motion-duration-base) var(--mw-motion-easing-enter) both;
  }
  @keyframes picture-viewer-in {
    from {
      transform: scale(var(--mw-component-picture-viewer-enter-scale));
    }
  }
</style>
