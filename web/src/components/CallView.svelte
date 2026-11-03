<script lang="ts">
  import { onMount, tick, untrack } from "svelte";

  import { strings } from "../i18n";
  import type { MellowClient } from "../lib/client.svelte";
  import type { PublicUser, Snowflake } from "../lib/protocol";
  import { prepareChimeSound } from "./chime-sound";
  import Avatar from "./Avatar.svelte";
  import CallTile from "./CallTile.svelte";
  import {
    type CallLayout,
    type CallPerson,
    arrangeCallTiles,
    featuredPersonOf,
    gridShapeFor,
    nextSpeaker,
    watchedCamerasOf,
  } from "./call-view";
  import { displayName } from "./format";
  import Icon from "./Icon.svelte";
  import IconButton from "./IconButton.svelte";
  import { lengthTokenInPixels } from "./tokens";

  // The call view (ADR-0035, docs/design/web-cameras.md), in the conversation sheet's place like the
  // stream view, the chat kept underneath: the chat header's grammar (the channel, then the layout
  // and Back to the chat), a dark stage with one tile per person (nine at most, the others as
  // avatars under them), and a band of call controls as tall as the header. The grid is the
  // default; the speaker view puts one person large, the one picked or else whoever talks, over a
  // strip of the others. A shared screen is a tile too; opening it watches it in the stream view.
  // Esc goes back to the chat.
  interface Props {
    client: MellowClient;
    /** Phones: the navigator is a drawer, opened from the header as in the chat. */
    hasNavigationDrawer: boolean;
    onopennavigation: () => void;
  }

  let { client, hasNavigationDrawer, onopennavigation }: Props = $props();

  let view = $state<HTMLElement | null>(null);
  let layout = $state<CallLayout>("grid");
  let pickedUserId = $state<Snowflake | null>(null);
  let speakerUserId = $state<Snowflake | null>(null);
  let gridWidth = $state(0);
  let gridHeight = $state(0);
  let featuredWidth = $state(0);
  let featuredHeight = $state(0);

  const voice = $derived(client.voice);
  const place = $derived(client.voiceChannel);
  const ownUserId = $derived(client.user?.id ?? null);
  const states = $derived(place ? client.voiceParticipants(place.channel.id) : []);
  const ownVideo = $derived(voice.isSharingScreen ? "screen" : voice.isCameraOn || voice.isCameraStarting ? "camera" : null);
  const people = $derived.by((): CallPerson[] => {
    const listed = states.map(
      (state): CallPerson => ({
        userId: state.user_id,
        video: state.user_id === ownUserId ? ownVideo : state.self_stream === true ? "screen" : state.self_video === true ? "camera" : null,
      }),
    );
    // Your own state comes back from the instance a moment after joining: you are in the call already.
    return ownUserId === null || listed.some((person) => person.userId === ownUserId) ? listed : [...listed, { userId: ownUserId, video: ownVideo }];
  });
  const presentUserIds = $derived(people.map((person) => person.userId));

  // Whoever talks last stays large through silences; it is history, not a derivation of now.
  $effect(() => {
    const next = nextSpeaker(untrack(() => speakerUserId), client.speakingUserIds, ownUserId, presentUserIds);
    if (next !== untrack(() => speakerUserId)) {
      speakerUserId = next;
    }
  });

  const featuredUserId = $derived(layout === "speaker" ? featuredPersonOf(people, pickedUserId, speakerUserId, ownUserId) : null);
  const arrangement = $derived(arrangeCallTiles(people, featuredUserId));
  const tiles = $derived(arrangement.tiles);
  const stripTiles = $derived(layout === "speaker" ? tiles.slice(1) : []);
  const gridShape = $derived(gridShapeFor(tiles.length, gridWidth, gridHeight, tileGap()));
  const featuredTileWidth = $derived(gridShapeFor(1, featuredWidth, featuredHeight, 0).tileWidth);

  // The pictures shown, and how large (the session sends a list only when it changes).
  $effect(() => {
    const watched = watchedCamerasOf(tiles, layout, ownUserId);
    untrack(() => voice.watchCameras(watched));
  });

  function tileGap(): number {
    return typeof document === "undefined" ? 0 : lengthTokenInPixels("component-call-view-tile-gap");
  }

  function personOf(userId: Snowflake): Pick<PublicUser, "id" | "display_name" | "username" | "avatar"> {
    const guildId = place?.guild.id;
    const member = guildId ? client.membersByGuild.get(guildId)?.find((candidate) => candidate.user.id === userId) : undefined;
    if (member) {
      return member.user;
    }
    return userId === ownUserId && client.user ? client.user : { id: userId, username: userId, display_name: strings.voice.someone };
  }

  function isSpeaking(userId: Snowflake): boolean {
    return client.speakingUserIds.has(userId) || (userId === ownUserId && voice.isSelfSpeaking);
  }

  function isMicrophoneOff(userId: Snowflake): boolean {
    if (userId === ownUserId) {
      return voice.isMicrophoneMuted;
    }
    const state = states.find((candidate) => candidate.user_id === userId);
    return state !== undefined && (state.self_mute || state.suppressed);
  }

  function actionOf(person: CallPerson) {
    const name = displayName(personOf(person.userId));
    if (person.video === "screen" && person.userId !== ownUserId) {
      return { label: strings.screenShare.watch(name), isPressed: undefined, onclick: () => voice.watchStream(person.userId) };
    }
    const isPicked = layout === "speaker" && pickedUserId === person.userId;
    const label = strings.call.feature(person.userId === ownUserId ? strings.call.you(name) : name);
    return {
      label,
      isPressed: isPicked,
      onclick: async () => {
        if (isPicked) {
          pickedUserId = null;
        } else {
          layout = "speaker";
          pickedUserId = person.userId;
        }
        // The tile may move (to the large place, or back to the strip): focus follows it instead of falling to the page.
        await tick();
        view?.querySelector<HTMLButtonElement>(`[data-call-tile="${person.userId}"] button`)?.focus();
      },
    };
  }

  function chooseLayout(chosen: CallLayout) {
    layout = chosen;
    if (chosen === "grid") {
      pickedUserId = null;
    }
  }

  onMount(() => {
    // Opened from a click (a channel row, the camera button): focus follows, and goes back there.
    const opener = document.activeElement instanceof HTMLElement ? document.activeElement : null;
    const element = view;
    element?.focus();
    return () => {
      voice.watchCameras([]);
      const focused = document.activeElement;
      if ((focused === document.body || (element?.contains(focused) ?? false)) && opener?.isConnected) {
        opener.focus();
      }
    };
  });

  /** Only while focus is in the view, so Esc keeps its meaning everywhere else (drawers, menus). */
  function closeOnEscape(event: KeyboardEvent) {
    const isInView = event.target instanceof Node && view?.contains(event.target) === true;
    if (event.key === "Escape" && isInView && !event.defaultPrevented) {
      event.preventDefault();
      voice.closeCallView();
    }
  }

  function toggleMicrophone() {
    void prepareChimeSound();
    void voice.toggleMicrophone();
  }

  const headerButtonClass =
    "size-(--mw-component-chat-header-button-size) text-(--mw-component-chat-header-icon) hover:bg-state-hover hover:text-text-normal";
  const controlClass =
    "size-(--mw-component-call-view-control-size) rounded-(--mw-component-call-view-control-radius) pointer-coarse:size-(--mw-component-call-view-control-size)";
  const restingControlClass =
    "bg-(--mw-component-call-view-control-background) text-(--mw-component-call-view-control-text) hover:bg-(--mw-component-call-view-control-background-hover) hover:text-(--mw-component-call-view-control-text-hover)";
  const activeControlClass =
    "bg-(--mw-component-call-view-control-background-active) text-(--mw-component-call-view-control-text-active) hover:bg-(--mw-component-call-view-control-background-active-hover) hover:text-(--mw-component-call-view-control-text-active-hover)";
</script>

<svelte:window onkeydown={closeOnEscape} />

{#snippet tile(person: CallPerson, isSmall: boolean, isInStrip = false)}
  {@const isOwn = person.userId === ownUserId}
  <CallTile
    person={personOf(person.userId)}
    video={person.video}
    track={isOwn ? (voice.isSharingScreen ? undefined : (voice.cameraTrack ?? undefined)) : voice.videoTracks.get(person.userId)}
    isOwn={isOwn && !voice.isSharingScreen}
    isSpeaking={isSpeaking(person.userId)}
    isMicrophoneOff={isMicrophoneOff(person.userId)}
    isPaused={!isOwn && voice.pausedVideoUserIds.has(person.userId)}
    sentence={isOwn && voice.isCameraStarting ? strings.call.startingCamera : ""}
    {isSmall}
    {isInStrip}
    action={actionOf(person)}
  />
{/snippet}

<section
  bind:this={view}
  tabindex="-1"
  aria-label={strings.call.viewOf(place?.channel.name ?? strings.voice.voiceChannel)}
  class={[
    "absolute inset-0 flex min-w-0 flex-col overflow-hidden bg-(--mw-component-call-view-background) outline-none",
    !hasNavigationDrawer && "rounded-(--mw-component-sheet-radius)",
  ]}
>
  <header class="flex h-(--mw-component-chat-header-height) shrink-0 items-center gap-xs ps-(--mw-component-chat-header-padding-inline) pe-xs">
    {#if hasNavigationDrawer}
      <IconButton
        icon="list"
        label={strings.chat.openNavigation}
        class={headerButtonClass}
        iconClass="size-(--mw-component-chat-header-icon-size)"
        onclick={onopennavigation}
      />
    {/if}
    <Icon name="speaker-high" class="size-(--mw-component-chat-header-icon-size) shrink-0 text-(--mw-component-chat-header-icon)" />
    <h1 class="min-w-0 truncate type-heading-md text-(--mw-component-chat-header-name-text)">{place?.channel.name ?? ""}</h1>
    <span class="flex-1"></span>
    <!-- Two toggles, the one in use pressed; their names say the layout, the icons draw it. -->
    <div role="group" aria-label={strings.call.layout} class="flex shrink-0 gap-3xs">
      <IconButton
        icon="squares-four"
        label={strings.call.grid}
        aria-pressed={layout === "grid"}
        class="{headerButtonClass} {layout === 'grid' ? 'bg-(--mw-component-call-view-layout-background-selected) text-text-heading' : ''}"
        iconClass="size-(--mw-component-chat-header-icon-size)"
        onclick={() => chooseLayout("grid")}
      />
      <IconButton
        icon="user-focus"
        label={strings.call.speaker}
        aria-pressed={layout === "speaker"}
        class="{headerButtonClass} {layout === 'speaker' ? 'bg-(--mw-component-call-view-layout-background-selected) text-text-heading' : ''}"
        iconClass="size-(--mw-component-chat-header-icon-size)"
        onclick={() => chooseLayout("speaker")}
      />
    </div>
    <IconButton
      icon="x"
      label={strings.call.close}
      aria-keyshortcuts="Escape"
      class={headerButtonClass}
      iconClass="size-(--mw-component-chat-header-icon-size)"
      onclick={() => voice.closeCallView()}
    />
  </header>
  <div
    class="mx-(--mw-component-call-view-stage-inset) flex min-h-0 flex-1 flex-col gap-(--mw-component-call-view-tile-gap) overflow-hidden rounded-(--mw-component-call-view-stage-radius) bg-(--mw-component-call-view-stage) p-(--mw-component-call-view-stage-padding)"
  >
    {#if layout === "grid"}
      <ul
        bind:clientWidth={gridWidth}
        bind:clientHeight={gridHeight}
        aria-label={strings.voice.participantsIn(place?.channel.name ?? "")}
        class="flex min-h-0 flex-1 flex-wrap content-center justify-center gap-(--mw-component-call-view-tile-gap)"
      >
        <!-- Rows of the computed width, the last one centred as on Discord. -->
        {#each tiles as person (person.userId)}
          <li class="aspect-video" style:width="{gridShape.tileWidth}px">{@render tile(person, tiles.length > 4)}</li>
        {/each}
      </ul>
    {:else}
      {@const featured = tiles[0]}
      {#if featured}
        <div bind:clientWidth={featuredWidth} bind:clientHeight={featuredHeight} class="flex min-h-0 flex-1 items-center justify-center">
          <div class="aspect-video" style:width="{featuredTileWidth}px">{@render tile(featured, false)}</div>
        </div>
      {/if}
      {#if stripTiles.length > 0}
        <ul
          aria-label={strings.voice.participantsIn(place?.channel.name ?? "")}
          class="flex h-(--mw-component-call-view-strip-tile-height) shrink-0 justify-center-safe gap-(--mw-component-call-view-tile-gap) overflow-x-auto"
        >
          {#each stripTiles as person (person.userId)}
            <li class="aspect-video h-full shrink-0">{@render tile(person, true, true)}</li>
          {/each}
        </ul>
      {/if}
    {/if}
    {#if arrangement.overflow.length > 0}
      <!-- Past nine tiles (ADR-0035), the others as avatars, their names in tooltips and for screen readers. -->
      <ul
        aria-label={strings.call.overflow(arrangement.overflow.length)}
        class="flex shrink-0 flex-wrap items-center justify-center gap-xs text-(--mw-component-call-view-tile-text)"
      >
        {#each arrangement.overflow as person (person.userId)}
          {@const shown = personOf(person.userId)}
          <li class={["call-overflow-avatar rounded-full", isSpeaking(person.userId) && "is-speaking"]} title={displayName(shown)}>
            <Avatar user={shown} sizeToken="--mw-component-call-view-overflow-avatar-size" />
            <span class="sr-only">{displayName(shown)}</span>
          </li>
        {/each}
      </ul>
    {/if}
  </div>
  <!-- The call's own controls, so a phone needs no drawer to mute or turn the camera on. -->
  <div
    role="group"
    aria-label={strings.call.controls}
    class="flex h-(--mw-component-call-view-control-bar-height) shrink-0 items-center justify-center gap-(--mw-component-call-view-control-gap) px-xs"
  >
    <IconButton
      icon={voice.isMicrophoneMuted ? "microphone-slash" : "microphone"}
      label={strings.userCard.muteMicrophone}
      title={voice.isMicrophoneMuted ? strings.userCard.unmuteMicrophone : strings.userCard.muteMicrophone}
      aria-pressed={voice.isMicrophoneMuted}
      aria-keyshortcuts="Control+Shift+M"
      class="{controlClass} {restingControlClass}"
      iconClass="size-(--mw-component-chat-header-icon-size)"
      onclick={toggleMicrophone}
    />
    {#if voice.canUseCamera || voice.isCameraOn}
      <IconButton
        icon={voice.isCameraOn || voice.isCameraStarting ? "video-camera" : "video-camera-slash"}
        label={voice.isCameraStarting ? strings.camera.starting : voice.isCameraOn ? strings.camera.turnOff : strings.camera.turnOn}
        aria-busy={voice.isCameraStarting}
        class="{controlClass} {voice.isCameraOn || voice.isCameraStarting ? activeControlClass : restingControlClass}"
        iconClass="size-(--mw-component-chat-header-icon-size)"
        onclick={() => (voice.isCameraOn || voice.isCameraStarting ? voice.stopCamera() : void voice.startCamera())}
      />
    {/if}
    {#if voice.canShareScreen || voice.isSharingScreen}
      <IconButton
        icon="monitor-arrow-up"
        label={voice.isSharingScreen ? strings.screenShare.stop : strings.call.shareScreen}
        class="{controlClass} {voice.isSharingScreen ? activeControlClass : restingControlClass}"
        iconClass="size-(--mw-component-chat-header-icon-size)"
        onclick={() => (voice.isSharingScreen ? voice.stopScreenShare() : void voice.startScreenShare())}
      />
    {/if}
    <IconButton
      icon="phone-disconnect"
      label={strings.voice.disconnect}
      class="{controlClass} {restingControlClass} hover:text-(--mw-component-call-view-hang-up-text-hover)"
      iconClass="size-(--mw-component-chat-header-icon-size)"
      onclick={() => voice.leave()}
    />
  </div>
</section>

<style>
  /* The same lamp as in the navigator, cut from the stage. */
  .call-overflow-avatar {
    display: inline-flex;
    box-shadow:
      0 0 0 var(--mw-component-voice-participant-speaking-ring-gap) transparent,
      0 0 0 calc(var(--mw-component-voice-participant-speaking-ring-gap) + var(--mw-component-call-view-speaking-ring-width)) transparent;
    transition: box-shadow var(--mw-motion-duration-fast) var(--mw-motion-easing-standard);
  }
  .call-overflow-avatar.is-speaking {
    box-shadow:
      0 0 0 var(--mw-component-voice-participant-speaking-ring-gap) var(--mw-component-call-view-stage),
      0 0 0 calc(var(--mw-component-voice-participant-speaking-ring-gap) + var(--mw-component-call-view-speaking-ring-width)) var(--mw-component-call-view-speaking-ring);
  }
</style>
