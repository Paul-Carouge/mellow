<script lang="ts">
  import { strings } from "../i18n";
  import type { PublicUser } from "../lib/protocol";
  import Avatar from "./Avatar.svelte";
  import { displayName } from "./format";
  import Icon from "./Icon.svelte";
  import { LIVE_BADGE_CLASS } from "./live-badge";
  import type { CallVideo } from "./call-view";

  // One person in the call view (docs/design/web-cameras.md): their picture filling the tile (a
  // camera cropped to fill, a screen letterboxed), else their avatar on the tile, with a sentence
  // while your camera starts or the instance paused their picture. Their name sits in a chip at the
  // lower start corner, after a crossed microphone when muted; a screen adds the live chip. Whoever
  // talks gets Dusk's lamp, a ring drawn inside the edge. The whole tile is one toggle: put them
  // forward in the speaker view (or, for a screen, watch it large).
  interface Props {
    person: Pick<PublicUser, "id" | "display_name" | "username" | "avatar">;
    video: CallVideo;
    track: MediaStreamTrack | undefined;
    /** Your own camera: mirrored, as a mirror shows you. */
    isOwn: boolean;
    isSpeaking: boolean;
    isMicrophoneOff: boolean;
    /** Paused by the instance for this connection (`VOICE_VIDEO_PAUSED`). */
    isPaused: boolean;
    /** Said on the tile while no picture shows, e.g. your camera starting. */
    sentence: string;
    isSmall: boolean;
    /** The speaker view's strip: too short for a sentence, so a paused picture is a mark in the name chip. */
    isInStrip: boolean;
    /** The toggle's name, and whether it is on; `null` when the tile does nothing. */
    action: { label: string; isPressed: boolean | undefined; onclick: () => void } | null;
  }

  let { person, video, track, isOwn, isSpeaking, isMicrophoneOff, isPaused, sentence, isSmall, isInStrip, action }: Props = $props();

  let hasPicture = $state(false);
  const name = $derived(displayName(person));
  const shownName = $derived(isOwn ? strings.call.you(name) : name);
  const isPictureShown = $derived(track !== undefined && hasPicture && !isPaused);
  const stageSentence = $derived(isPaused ? strings.call.paused : sentence);
  const states = $derived(
    [
      isMicrophoneOff && strings.voice.microphoneOff,
      video === "camera" && strings.camera.cameraOn,
      video === "screen" && strings.screenShare.sharesScreen,
      isPaused && strings.call.paused,
    ].filter((label): label is string => typeof label === "string"),
  );

  /** Plays `track`, and lets it go when it changes or the tile goes. */
  function playTrack(element: HTMLVideoElement) {
    if (track === undefined) {
      return;
    }
    hasPicture = false;
    element.srcObject = new MediaStream([track]);
    return () => {
      element.srcObject = null;
    };
  }
</script>

<div
  data-call-tile={person.id}
  class={[
    "call-tile relative size-full overflow-hidden rounded-(--mw-component-call-view-tile-radius) bg-(--mw-component-call-view-tile-background) text-(--mw-component-call-view-tile-text)",
    isSpeaking && "is-speaking",
  ]}
>
  {#if track !== undefined}
    <!-- Muted: voices play through the session. The tile's toggle carries the name. -->
    <video
      {@attach playTrack}
      autoplay
      playsinline
      muted
      aria-hidden="true"
      width="1280"
      height="720"
      class={[
        "absolute inset-0 size-full",
        video === "screen" ? "object-contain" : "object-cover",
        isOwn && "-scale-x-100",
        !isPictureShown && "invisible",
      ]}
      onloadeddata={() => (hasPicture = true)}
    ></video>
  {/if}
  {#if !isPictureShown}
    <div class="absolute inset-0 flex flex-col items-center justify-center gap-xs p-xs text-center">
      <Avatar
        user={person}
        sizeToken={isSmall ? "--mw-component-call-view-avatar-size-small" : "--mw-component-call-view-avatar-size-large"}
      />
      {#if stageSentence && !isInStrip}
        <p class="flex items-center gap-2xs type-body-sm">
          {#if isPaused}<Icon name="wifi-slash" class="size-(--mw-component-call-view-label-icon-size) shrink-0" />{/if}
          <span>{stageSentence}</span>
        </p>
      {/if}
    </div>
  {/if}
  {#if video === "screen"}
    <span aria-hidden="true" class="{LIVE_BADGE_CLASS} absolute start-xs top-xs">{strings.screenShare.live}</span>
  {/if}
  <span
    aria-hidden="true"
    class="absolute start-xs bottom-xs flex h-(--mw-component-call-view-label-height) max-w-[calc(100%-2*var(--mw-space-xs))] items-center gap-2xs rounded-(--mw-component-call-view-label-radius) bg-(--mw-component-call-view-label-background) px-(--mw-component-call-view-label-padding-inline) type-body-sm"
  >
    {#if isPaused && isInStrip}<Icon name="wifi-slash" class="size-(--mw-component-call-view-label-icon-size) shrink-0" />{/if}
    {#if isMicrophoneOff}<Icon name="microphone-slash" class="size-(--mw-component-call-view-label-icon-size) shrink-0" />{/if}
    <span class="truncate">{shownName}</span>
  </span>
  {#if action}
    <button
      type="button"
      aria-label={action.label}
      aria-pressed={action.isPressed}
      aria-describedby={states.length > 0 ? `call-tile-${person.id}-states` : undefined}
      title={action.label}
      class="absolute inset-0 z-1 rounded-(--mw-component-call-view-tile-radius) focus-visible:focus-ring-inset"
      onclick={action.onclick}
    ></button>
  {/if}
  <span id="call-tile-{person.id}-states" class="sr-only">{[shownName, ...states].join(", ")}</span>
</div>

<style>
  /* Dusk's lamp on whoever is talking, inside the tile's edge so nothing shifts; it fades over
     motion.duration.fast (instant under reduced motion, where the token is 0). */
  .call-tile::after {
    content: "";
    position: absolute;
    inset: 0;
    border-radius: inherit;
    pointer-events: none;
    box-shadow: inset 0 0 0 var(--mw-component-call-view-speaking-ring-width) transparent;
    transition: box-shadow var(--mw-motion-duration-fast) var(--mw-motion-easing-standard);
  }
  .call-tile.is-speaking::after {
    box-shadow: inset 0 0 0 var(--mw-component-call-view-speaking-ring-width) var(--mw-component-call-view-speaking-ring);
  }
</style>
