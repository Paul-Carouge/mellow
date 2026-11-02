<script lang="ts">
  import { onMount } from "svelte";

  import { strings } from "../i18n";
  import type { MellowClient } from "../lib/client.svelte";
  import type { PublicUser, Snowflake } from "../lib/protocol";
  import Avatar from "./Avatar.svelte";
  import { displayName } from "./format";
  import IconButton from "./IconButton.svelte";
  import { LIVE_BADGE_CLASS } from "./live-badge";
  import { watchedStreamStatus } from "./screen-share-look";
  import { durationTokenInMilliseconds } from "./tokens";

  // A shared screen, watched large in the conversation sheet's place (ADR-0037,
  // docs/design/web-screen-sharing.md): the chat header's grammar (who, the live chip, then the
  // actions) over a dark stage where the picture is letterboxed. While the picture comes, the
  // streamer's face and "Connexion à l'écran…"; when they stop or leave, one sentence, then the
  // view closes by itself. Esc closes it (in fullscreen, Esc leaves fullscreen first, as browsers do).
  interface Props {
    client: MellowClient;
    userId: Snowflake;
    /** Phones: the navigator is a drawer, opened from the header as in the chat. */
    hasNavigationDrawer: boolean;
    onopennavigation: () => void;
  }

  let { client, userId, hasNavigationDrawer, onopennavigation }: Props = $props();

  let view = $state<HTMLElement | null>(null);
  let isFullscreen = $state(false);
  let hasPicture = $state(false);
  const canGoFullscreen = typeof document !== "undefined" && document.fullscreenEnabled;

  const voice = $derived(client.voice);
  const track = $derived(voice.streamTracks.get(userId));
  const status = $derived(
    watchedStreamStatus(client.voiceStatesByUserId.get(userId), voice.phase === "idle" ? null : voice.channelId, track !== undefined),
  );
  const person = $derived.by((): Pick<PublicUser, "id" | "display_name" | "username" | "avatar"> => {
    const guildId = client.voiceChannel?.guild.id;
    const member = guildId ? client.membersByGuild.get(guildId)?.find((candidate) => candidate.user.id === userId) : undefined;
    return member?.user ?? { id: userId, username: userId, display_name: strings.voice.someone };
  });
  const name = $derived(displayName(person));
  const stageSentence = $derived(
    status === "ended" ? strings.screenShare.ended(name) : status === "live" && hasPicture ? "" : strings.screenShare.connecting,
  );

  // They stopped: the sentence stays for a reading time, then the view goes. Sharing again
  // meanwhile cancels it.
  $effect(() => {
    if (status !== "ended") {
      return;
    }
    const timer = setTimeout(() => voice.watchStream(null), durationTokenInMilliseconds("component-stream-view-ended-close-delay"));
    return () => clearTimeout(timer);
  });

  onMount(() => {
    // Opened from a live chip: focus follows, and goes back there when the view closes.
    const opener = document.activeElement instanceof HTMLElement ? document.activeElement : null;
    // `view` is unbound before this cleanup runs: keep the element.
    const element = view;
    element?.focus();
    const noteFullscreen = () => (isFullscreen = document.fullscreenElement !== null && document.fullscreenElement === element);
    document.addEventListener("fullscreenchange", noteFullscreen);
    return () => {
      document.removeEventListener("fullscreenchange", noteFullscreen);
      if (element !== null && document.fullscreenElement === element) {
        void document.exitFullscreen().catch(() => {});
      }
      // Focus was in the view (or fell to the page with it): back to the chip that opened it.
      const focused = document.activeElement;
      if ((focused === document.body || (element?.contains(focused) ?? false)) && opener?.isConnected) {
        opener.focus();
      }
    };
  });

  /** Plays `track` in the video element, and lets it go when the track changes or the view closes. */
  function playTrack(video: HTMLVideoElement) {
    if (track === undefined) {
      return;
    }
    hasPicture = false;
    video.srcObject = new MediaStream([track]);
    return () => {
      video.srcObject = null;
    };
  }

  function toggleFullscreen() {
    if (document.fullscreenElement === view) {
      void document.exitFullscreen().catch(() => {});
    } else {
      // Refused without a user gesture or by the page's permissions: the view stays as it is.
      void view?.requestFullscreen().catch(() => {});
    }
  }

  /** Only while focus is in the view, so Esc keeps its meaning everywhere else (drawers, menus). */
  function closeOnEscape(event: KeyboardEvent) {
    const isInView = event.target instanceof Node && view?.contains(event.target) === true;
    if (event.key === "Escape" && isInView && document.fullscreenElement === null && !event.defaultPrevented) {
      event.preventDefault();
      voice.watchStream(null);
    }
  }

  const headerButtonClass =
    "size-(--mw-component-chat-header-button-size) text-(--mw-component-chat-header-icon) hover:bg-state-hover hover:text-text-normal";
</script>

<svelte:window onkeydown={closeOnEscape} />

<section
  bind:this={view}
  tabindex="-1"
  aria-label={strings.screenShare.screenOf(name)}
  class={[
    "absolute inset-0 flex min-w-0 flex-col overflow-hidden bg-(--mw-component-stream-view-background) outline-none",
    !hasNavigationDrawer && !isFullscreen && "rounded-(--mw-component-sheet-radius)",
  ]}
>
  <header class="flex h-(--mw-component-chat-header-height) shrink-0 items-center gap-xs ps-(--mw-component-chat-header-padding-inline) pe-xs">
    {#if hasNavigationDrawer && !isFullscreen}
      <IconButton
        icon="list"
        label={strings.chat.openNavigation}
        class={headerButtonClass}
        iconClass="size-(--mw-component-chat-header-icon-size)"
        onclick={onopennavigation}
      />
    {/if}
    <Avatar user={person} sizeToken="--mw-component-chat-header-icon-size" />
    <h1 class="min-w-0 truncate type-heading-md text-(--mw-component-chat-header-name-text)">{strings.screenShare.screenOf(name)}</h1>
    {#if status !== "ended"}
      <span aria-hidden="true" class={LIVE_BADGE_CLASS}>{strings.screenShare.live}</span>
    {/if}
    <span class="flex-1"></span>
    {#if canGoFullscreen}
      <IconButton
        icon={isFullscreen ? "corners-in" : "corners-out"}
        label={isFullscreen ? strings.screenShare.exitFullscreen : strings.screenShare.fullscreen}
        aria-keyshortcuts={isFullscreen ? "Escape" : undefined}
        class={headerButtonClass}
        iconClass="size-(--mw-component-chat-header-icon-size)"
        onclick={toggleFullscreen}
      />
    {/if}
    <IconButton
      icon="x"
      label={strings.screenShare.close}
      aria-keyshortcuts="Escape"
      class={headerButtonClass}
      iconClass="size-(--mw-component-chat-header-icon-size)"
      onclick={() => voice.watchStream(null)}
    />
  </header>
  <div
    class={[
      "relative min-h-0 flex-1 overflow-hidden bg-(--mw-component-stream-view-stage)",
      !isFullscreen && "mx-(--mw-component-stream-view-stage-inset) mb-(--mw-component-stream-view-stage-inset) rounded-(--mw-component-stream-view-stage-radius)",
    ]}
  >
    {#if track !== undefined && status !== "ended"}
      <!-- Muted: the stream has no sound in this version (ADR-0037). The name is the view's. -->
      <video
        {@attach playTrack}
        autoplay
        playsinline
        muted
        aria-hidden="true"
        class={["size-full object-contain", !hasPicture && "invisible"]}
        onloadeddata={() => (hasPicture = true)}
      ></video>
    {/if}
    {#if stageSentence}
      <div class="absolute inset-0 flex flex-col items-center justify-center gap-sm p-md text-center">
        <Avatar user={person} sizeToken="--mw-component-stream-view-avatar-size" />
        <p class="type-body text-(--mw-component-stream-view-stage-text)">{stageSentence}</p>
      </div>
    {/if}
  </div>
  <!-- Exists from the start, so "connecting" then "stopped" are read out as they come. -->
  <p class="sr-only" aria-live="polite">{stageSentence}</p>
</section>
