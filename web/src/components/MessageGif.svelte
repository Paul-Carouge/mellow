<script lang="ts">
  import { strings } from "../i18n";
  import type { MessageGifMedia } from "../lib/klipy";
  import { isPictureAnimationWanted } from "../lib/motion.svelte";
  import { containedFrame } from "./gif-picker";
  import Icon from "./Icon.svelte";

  // A message whose whole text is one Klipy media address shows the GIF itself (ADR-0042,
  // docs/design/web-gif-picker.md): always `component.gif.message-height` high, whatever its size
  // and the column's width, so the list never moves when it loads; the width follows its shape
  // and a picture wider than the column is letterboxed. It plays as custom emoji do;
  // while animated pictures are kept still it shows its first frame and a "GIF" key that plays it.
  // A GIF Klipy no longer serves (or a page without Klipy in its policy) gives the link back.
  interface Props {
    media: MessageGifMedia;
    onfailed: () => void;
  }

  let { media, onfailed }: Props = $props();

  /** Width over height, from the picture itself once it loaded; a square until then. */
  let aspectRatio = $state(1);
  let isPlayingByChoice = $state(false);
  const isAnimated = $derived(isPictureAnimationWanted(true) || isPlayingByChoice);
  let picture = $state<HTMLImageElement | null>(null);
  let video = $state<HTMLVideoElement | null>(null);
  let stillFrame = $state<HTMLCanvasElement | null>(null);
  let hasLoaded = $state(false);
  let boxWidth = $state(0);
  let boxHeight = $state(0);

  function takeSize(width: number, height: number) {
    if (width > 0 && height > 0) {
      aspectRatio = width / height;
    }
    hasLoaded = true;
  }

  // An <img> cannot pause: the first frame is painted on a canvas instead, which never moves. It
  // is drawn at the box's size in device pixels (not the picture's, which may be far larger), and
  // again when the box changes width.
  $effect(() => {
    if (isAnimated || !hasLoaded || !picture || !stillFrame) {
      return;
    }
    const pixelRatio = window.devicePixelRatio || 1;
    const width = Math.round(boxWidth * pixelRatio);
    const height = Math.round(boxHeight * pixelRatio);
    const frame = containedFrame(width, height, picture.naturalWidth, picture.naturalHeight);
    if (!frame) {
      return;
    }
    stillFrame.width = width;
    stillFrame.height = height;
    stillFrame.getContext("2d")?.drawImage(picture, frame.x, frame.y, frame.width, frame.height);
  });

  $effect(() => {
    if (!video) {
      return;
    }
    if (isAnimated) {
      // A refused play (power saving, autoplay rules) leaves the first frame, as kept still.
      video.play().catch((error: unknown) => console.debug("GIF video did not play", error));
    } else {
      video.pause();
    }
  });
</script>

<div
  bind:clientWidth={boxWidth}
  bind:clientHeight={boxHeight}
  class="relative h-(--mw-component-gif-message-height) max-w-full overflow-hidden rounded-(--mw-component-gif-message-radius) bg-(--mw-component-gif-message-placeholder)"
  style:width="min(100%, var(--mw-component-gif-message-max-width), calc(var(--mw-component-gif-message-height) * {aspectRatio}))"
>
  {#if media.isVideo}
    <video
      bind:this={video}
      src={media.url}
      aria-label={strings.gifs.messageGif}
      muted
      loop
      playsinline
      disablepictureinpicture
      preload="metadata"
      class="block size-full object-contain"
      onloadedmetadata={(event) => takeSize(event.currentTarget.videoWidth, event.currentTarget.videoHeight)}
      onerror={onfailed}
    ></video>
  {:else}
    <img
      bind:this={picture}
      src={media.url}
      alt={strings.gifs.messageGif}
      loading="lazy"
      decoding="async"
      referrerpolicy="no-referrer"
      draggable="false"
      class={["block size-full object-contain", !isAnimated && hasLoaded && "invisible"]}
      onload={() => takeSize(picture?.naturalWidth ?? 0, picture?.naturalHeight ?? 0)}
      onerror={onfailed}
    />
    {#if !isAnimated && hasLoaded}
      <!-- The hidden <img> leaves the accessibility tree: the frame takes its name. -->
      <div role="img" aria-label={strings.gifs.messageGif} class="absolute inset-0">
        <canvas bind:this={stillFrame} aria-hidden="true" class="block size-full"></canvas>
      </div>
    {/if}
  {/if}
  {#if !isPictureAnimationWanted(true)}
    <!-- Kept still by reduced motion: the reader may still play this one. -->
    <button
      type="button"
      aria-label={strings.gifs.playGif}
      title={strings.gifs.playGif}
      aria-pressed={isPlayingByChoice}
      class="absolute start-(--mw-component-gif-star-inset) bottom-(--mw-component-gif-star-inset) inline-flex h-(--mw-component-gif-play-button-size) items-center gap-3xs rounded-full bg-(--mw-component-gif-play-button-background) px-xs type-body-sm font-semibold text-(--mw-component-gif-play-button-text) [--mw-color-focus:var(--mw-component-gif-tile-focus-ring)] focus-visible:focus-ring pointer-coarse:h-(--mw-size-hit-web-min)"
      onclick={() => (isPlayingByChoice = !isPlayingByChoice)}
    >
      <Icon name="gif" class="size-(--mw-component-gif-star-icon-size)" />
      {#if !isPlayingByChoice}
        <Icon name="play" class="size-(--mw-component-gif-star-icon-size)" />
      {/if}
    </button>
  {/if}
</div>
