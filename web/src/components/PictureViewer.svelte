<script lang="ts">
  import { onMount, tick } from "svelte";

  import { strings } from "../i18n";
  import { type Attachment, attachmentPreviewUrl, attachmentUrl } from "../lib/protocol";
  import { fileNameStartAndEnd, humanFileSize, messageTimestampLabel } from "./format";
  import Icon from "./Icon.svelte";
  import IconButton from "./IconButton.svelte";
  import {
    type PixelSize,
    actualSizeScrollStart,
    attachmentPixelSize,
    fittedPictureSize,
    isPictureLargerThanBox,
    wrappedPictureIndex,
  } from "./picture-viewer";
  import { IMAGE_RETRY_DELAY_MILLISECONDS } from "./image-retry";
  import { durationTokenInMilliseconds, lengthTokenInPixels } from "./tokens";

  // docs/design/web-picture-viewer.md: a message's pictures large over the app, on the opaque
  // media stage, with the controls in a band under it. A native modal <dialog> keeps the app inert.
  interface Props {
    pictures: readonly Attachment[];
    initialIndex: number;
    authorName: string;
    sentAtUnixMilliseconds: number;
    /** The picture link that opened the viewer; focus goes back to it when it closes. */
    opener: HTMLElement | null;
    onclose: () => void;
  }

  let { pictures, initialIndex, authorName, sentAtUnixMilliseconds, opener, onclose }: Props = $props();

  const id = $props.id();
  let dialog: HTMLDialogElement;
  let stage: HTMLDivElement;
  let closeButton = $state<HTMLButtonElement | null>(null);
  // The viewer keeps the pictures it opened with; the list behind it may change meanwhile.
  // svelte-ignore state_referenced_locally
  let index = $state(Math.min(Math.max(initialIndex, 0), pictures.length - 1));
  const picture = $derived(pictures[index]);

  let stageWidth = $state(0);
  let stageHeight = $state(0);
  let isActualSize = $state(false);
  // Per shown picture: reset by `showPicture` and by "Réessayer".
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
  const naturalSize = $derived(picture ? (attachmentPixelSize(picture) ?? loadedSize) : null);
  const box = $derived({ width: Math.max(1, stageWidth - 2 * stagePadding), height: Math.max(1, stageHeight - 2 * stagePadding) });
  const isZoomable = $derived(naturalSize !== null && isPictureLargerThanBox(naturalSize, box));
  // A window grown until the picture fits leaves actual size by itself.
  const isShownAtActualSize = $derived(isActualSize && isZoomable);
  const shownSize = $derived(naturalSize === null ? null : isShownAtActualSize ? naturalSize : fittedPictureSize(naturalSize, box));
  const caption = $derived(
    picture
      ? strings.pictureViewer.caption(authorName, messageTimestampLabel(sentAtUnixMilliseconds, new Date()), humanFileSize(picture.size_bytes))
      : "",
  );
  const hasNothingToShow = $derived(isFullFailed && isPreviewFailed);

  async function showPicture(nextIndex: number) {
    index = nextIndex;
    isActualSize = false;
    clearTimeout(copiedTimer);
    isCopied = false;
    resetLoading();
    const shown = pictures[index];
    if (shown) {
      announcement = strings.pictureViewer.pictureChanged(index + 1, pictures.length, shown.file_name);
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
    void showPicture(wrappedPictureIndex(index, direction, pictures.length));
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
    if (!picture) {
      return;
    }
    try {
      await navigator.clipboard.writeText(new URL(attachmentUrl(picture), window.location.origin).href);
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
    // At actual size the arrows pan the stage, as the browser does by itself.
    if (isShownAtActualSize || pictures.length < 2 || event.altKey || event.ctrlKey || event.metaKey) {
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
      {#if picture && !hasNothingToShow}
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
          {#key `${picture.id}:${loadGeneration}`}
            {@const shownGeneration = loadGeneration}
            <!-- The list's preview, already in the cache, holds the place until the full file arrives. -->
            {#if !isFullLoaded && !isPreviewFailed}
              <img
                src={attachmentPreviewUrl(picture)}
                alt={picture.file_name}
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
            {#if !isFullFailed}
              <img
                src={attachmentUrl(picture)}
                alt={picture.file_name}
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
                : hasNothingToShow
                  ? strings.pictureViewer.pictureFailed
                  : strings.pictureViewer.fullSizeFailed}
            </span>
            {#if isFullFailed}
              <button
                type="button"
                class="min-h-(--mw-component-picture-viewer-control-size) rounded-(--mw-component-picture-viewer-control-radius) border-(length:--mw-component-picture-viewer-retry-border-width) border-(--mw-component-picture-viewer-retry-border) px-(--mw-component-picture-viewer-status-padding) font-semibold hover:bg-(--mw-component-picture-viewer-control-background-hover) focus-visible:focus-ring"
                onclick={retry}>{strings.common.tryAgain}</button
              >
            {/if}
          </p>
        {:else if picture && !isFullLoaded && isLoadingNoticeDue}
          <!-- Shown only when the full file is slow: a cached picture never flashes it. -->
          <p
            class="animate-[mw-fade-in_var(--mw-motion-duration-base)_var(--mw-motion-easing-standard)_both] rounded-(--mw-component-picture-viewer-status-radius) bg-(--mw-component-picture-viewer-status-background) p-(--mw-component-picture-viewer-status-padding) type-body text-(--mw-component-picture-viewer-status-text)"
          >
            {strings.pictureViewer.loading}
          </p>
        {/if}
      </div>
    </div>
  </div>

  {#if picture}
    {@const nameParts = fileNameStartAndEnd(picture.file_name)}
    <div
      class="flex min-h-(--mw-component-picture-viewer-band-min-height) flex-wrap items-center justify-between gap-x-(--mw-component-picture-viewer-band-gap) gap-y-(--mw-component-picture-viewer-band-row-gap) px-(--mw-component-picture-viewer-band-padding-inline) pt-(--mw-component-picture-viewer-band-padding-block) pb-[calc(var(--mw-component-picture-viewer-band-padding-block)+env(safe-area-inset-bottom))]"
    >
      <div class="min-w-0 flex-[1_1_var(--mw-component-picture-viewer-caption-min-width)]">
        <h2 id="{id}-name" title={picture.file_name} class="flex type-body-lg text-(--mw-component-picture-viewer-name-text)">
          <span aria-hidden="true" class="flex min-w-0"
            ><span class="truncate">{nameParts.start}</span><span class="shrink-0 whitespace-pre">{nameParts.end}</span></span
          >
          <span class="sr-only">{picture.file_name}</span>
        </h2>
        <p id="{id}-caption" class="truncate type-body-sm text-(--mw-component-picture-viewer-meta-text)">{caption}</p>
      </div>

      <div class="flex flex-1 flex-wrap items-center justify-between gap-(--mw-component-picture-viewer-band-gap)">
        {#if pictures.length > 1}
          <div class="flex items-center gap-(--mw-component-picture-viewer-control-gap)">
            {@render control("caret-left", strings.pictureViewer.previous, -1)}
            <span aria-hidden="true" class="type-body text-(--mw-component-picture-viewer-position-text) tabular-nums"
              >{strings.pictureViewer.position(index + 1, pictures.length)}</span
            >
            <span id="{id}-position" class="sr-only">{strings.pictureViewer.positionLabel(index + 1, pictures.length)}</span>
            {@render control("caret-right", strings.pictureViewer.next, 1)}
          </div>
        {/if}

        <div class="ms-auto flex items-center gap-(--mw-component-picture-viewer-control-gap)">
          {#snippet linkIcon(icon: "download-simple" | "arrow-square-out")}
            <Icon name={icon} class="size-(--mw-component-picture-viewer-control-icon-size)" />
          {/snippet}
          <a
            href={attachmentUrl(picture)}
            download={picture.file_name}
            aria-label={strings.messages.download(picture.file_name)}
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
            href={attachmentUrl(picture)}
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
