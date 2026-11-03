<script lang="ts">
  import { MediaQuery } from "svelte/reactivity";

  import { strings } from "../i18n";
  import type { MellowClient } from "../lib/client.svelte";
  import { Permission, type Snowflake } from "../lib/protocol";
  import {
    SCREEN_SHARE_QUALITIES,
    type ScreenShareQuality,
    shownScreenSoundDeviceId,
    soundlessShareAdvice,
  } from "../lib/screen-share";
  import { screenSoundPlatform } from "../lib/voice-browser";
  import Button from "./Button.svelte";
  import ContextMenu from "./ContextMenu.svelte";
  import Icon from "./Icon.svelte";
  import IconButton from "./IconButton.svelte";
  import LazyComponent from "./LazyComponent.svelte";
  import { createPopoverToggle } from "./popover-toggle";
  import { holds } from "./server-settings";
  import SoundboardPopover from "./SoundboardPopover.svelte";
  import { minimumWidthQueryForPanes } from "./tokens";
  import { problemLook, signalLook } from "./voice-panel";
  import { LIVE_BADGE_CLASS } from "./live-badge";

  // The voice panel (DESIGN.md §5.3, ADR-0030): a card like the user card, directly above it, in
  // the connection dock's slot (§4.2). While connected: the state with its signal bars, the channel
  // and server (a link to the server), and Disconnect. After a problem: what happened, how to fix
  // it, Try again when it may pass, and Close. Nothing while idle; the live regions always exist,
  // so their first change is read out.
  interface Props {
    client: MellowClient;
    /** After the channel line opened its server (the narrow layout closes its drawer). */
    onnavigate: () => void;
    /** The soundboard's "Add sounds": Server settings at Soundboard. */
    onaddsounds: (guildId: Snowflake) => void;
  }

  let { client, onnavigate, onaddsounds }: Props = $props();

  // The soundboard sits beside the navigator when the window has room for both, else it rises
  // from the bottom of the screen like the emoji picker.
  const hasRoomForSoundboardBeside = new MediaQuery(
    minimumWidthQueryForPanes(["component-navigator-strip-width", "component-sidebar-width", "component-emoji-picker-width"]),
  );
  let soundboardButton = $state<HTMLButtonElement | null>(null);
  let isSoundboardOpen = $state(false);
  const soundboardToggle = createPopoverToggle(() => isSoundboardOpen);

  // Screen sharing (ADR-0037): a split button on its own row, as Discord's panel has its video
  // buttons: the share half starts or stops, the quality half opens the share's options (its
  // quality, then where its sound comes from), which also apply while sharing.
  let qualityButton = $state<HTMLButtonElement | null>(null);
  let shareRow = $state<HTMLElement | null>(null);
  let qualityMenu = $state<ContextMenu | null>(null);
  let isQualityMenuOpen = $state(false);
  let isSoundHelpOpen = $state(false);
  const qualityToggle = createPopoverToggle(() => isQualityMenuOpen);
  // Only Chromium shares a picker's sound, and only Linux gets the commands' help.
  const soundPlatform = screenSoundPlatform();
  const soundAdvice = soundlessShareAdvice(soundPlatform.chromiumPlatform);
  function openQualityMenu() {
    if (shareRow) {
      void voice.refreshScreenSoundDevices();
      qualityMenu?.open({ kind: "anchor", bounds: shareRow.getBoundingClientRect(), align: "stretch" }, qualityButton);
    }
  }
  function chooseQuality(quality: ScreenShareQuality) {
    qualityMenu?.close();
    voice.setScreenShareQuality(quality);
  }
  function chooseSound(deviceId: string | null) {
    qualityMenu?.close();
    voice.setScreenSoundDevice(deviceId);
  }
  function openSoundHelp() {
    qualityMenu?.close();
    isSoundHelpOpen = true;
  }

  const voice = $derived(client.voice);
  const place = $derived(client.voiceChannel);
  const signal = $derived(signalLook(voice.roundTripMilliseconds));
  const problem = $derived(voice.problem);
  const problemChannelName = $derived.by(() => {
    const channelId = problem && "channelId" in problem ? problem.channelId : null;
    const channel = client.guilds.flatMap((guild) => guild.channels).find((candidate) => candidate.id === channelId);
    return channel?.name ?? strings.voice.voiceChannel;
  });
  const look = $derived(problem ? problemLook(problem, problemChannelName) : null);
  const soundNoticeSentence = $derived(
    voice.screenSoundNotice === null
      ? ""
      : voice.screenSoundNotice === "no-sound"
        ? strings.screenShare.soundNotices["no-sound"][soundAdvice]
        : strings.screenShare.soundNotices["source-unavailable"],
  );
  const shownSoundDeviceId = $derived(shownScreenSoundDeviceId(voice.screenSoundDevices, voice.screenSoundDeviceId));
  // What your share sends as sound, said under the live line so it is never a guess.
  const sharedSoundLine = $derived.by(() => {
    switch (voice.sharedSound) {
      case "screen":
        return strings.screenShare.sharedSound.screen;
      case "input": {
        const source = voice.screenSoundDevices.find((device) => device.deviceId === voice.screenSoundDeviceId);
        return strings.screenShare.sharedSound.input(source?.label ?? strings.screenShare.audioInputHint);
      }
      case "none":
        return strings.screenShare.sharedSound.none;
      default:
        return "";
    }
  });
  // Leaving voice takes the soundboard with it.
  $effect(() => {
    if (voice.phase === "idle") {
      isSoundboardOpen = false;
    }
  });
  const statusLabel = $derived(
    voice.phase === "connected" ? strings.voice.connected : voice.phase === "reconnecting" ? strings.voice.reconnecting : strings.voice.connecting,
  );
  const phaseSentence = $derived.by(() => {
    const name = place?.channel.name;
    if (!name) {
      return "";
    }
    switch (voice.phase) {
      case "connecting":
        return strings.voice.connectingTo(name);
      case "connected":
        // Push-to-talk said once on connecting: otherwise silence would be a mystery.
        return voice.settings.inputMode === "push-to-talk"
          ? `${strings.voice.connectedTo(name)} ${pushToTalkSentence}`
          : strings.voice.connectedTo(name);
      case "reconnecting":
        return strings.voice.reconnectingTo(name);
      default:
        return "";
    }
  });
  const pushToTalkKey = $derived(voice.settings.pushToTalkKey);
  const pushToTalkSentence = $derived(
    pushToTalkKey ? `${strings.voice.pushToTalkHint(pushToTalkKey.label)}.` : strings.voice.pushToTalkNoKey,
  );
  const deviceNotices = $derived(
    [
      voice.isInputDeviceMissing && strings.voice.inputDeviceMissing,
      voice.isOutputDeviceMissing && strings.voice.outputDeviceMissing,
    ].filter((notice): notice is string => typeof notice === "string"),
  );
  const signalClass = $derived(
    voice.phase !== "connected" || signal.quality === null
      ? "text-(--mw-component-voice-panel-signal-unknown)"
      : {
          good: "text-(--mw-component-voice-panel-signal-good)",
          medium: "text-(--mw-component-voice-panel-signal-medium)",
          poor: "text-(--mw-component-voice-panel-signal-poor)",
        }[signal.quality],
  );

  function openServer() {
    if (place) {
      client.selectGuild(place.guild.id, client.selectedGuildId === place.guild.id ? client.selectedChannelId : null);
      onnavigate();
    }
  }

  const cardClass =
    "mx-(--mw-component-voice-panel-margin) mb-2xs rounded-(--mw-component-voice-panel-radius) bg-(--mw-component-voice-panel-background) px-(--mw-component-voice-panel-padding-inline) py-(--mw-component-voice-panel-padding-block)";
  const menuItemClass =
    "group flex min-h-(--mw-component-context-menu-item-height) w-full items-center gap-xs rounded-(--mw-component-context-menu-item-radius) px-(--mw-component-context-menu-item-padding-inline) py-2xs text-start text-(--mw-component-context-menu-item-text) hover:bg-(--mw-component-context-menu-item-background-focus) hover:text-(--mw-component-context-menu-item-text-focus) pointer-coarse:min-h-(--mw-size-hit-web-min)";
  const menuHintClass =
    "truncate type-body-sm text-(--mw-component-context-menu-item-hint-text) group-hover:text-(--mw-component-context-menu-item-hint-text-focus)";
  const shareHalfClass =
    "flex h-(--mw-component-voice-panel-share-height) min-w-0 items-center gap-2xs rounded-(--mw-component-voice-panel-share-radius) type-button transition-colors duration-(--mw-motion-duration-fast) ease-(--mw-motion-easing-standard) pointer-coarse:h-(--mw-size-hit-web-min)";
  const menuGroupLabelClass =
    "px-(--mw-component-context-menu-item-padding-inline) pt-2xs pb-3xs type-category text-(--mw-component-context-menu-item-hint-text)";
  const menuSeparatorClass =
    "mx-(--mw-component-context-menu-separator-margin-inline) my-(--mw-component-context-menu-separator-margin-block) h-(--mw-component-context-menu-separator-height) shrink-0 border-0 bg-(--mw-component-context-menu-separator)";
  // Underlined like the channel link above: these are actions inside a sentence's block.
  const noticeActionClass =
    "rounded-xs text-start underline text-(--mw-component-voice-panel-channel-text) hover:text-(--mw-component-voice-panel-channel-text-hover) pointer-coarse:min-h-(--mw-size-hit-web-min)";
  const buttonClass =
    "size-(--mw-component-user-panel-button-size) text-(--mw-component-user-panel-button-icon) hover:bg-(--mw-component-user-panel-button-background-hover)";
</script>

{#snippet choice(isChecked: boolean, label: string, hint: string, onchoose: () => void, isFigure = false)}
  <button type="button" role="menuitemradio" aria-checked={isChecked} class={menuItemClass} onclick={onchoose}>
    <span class="flex min-w-0 flex-1 flex-col">
      <!-- Device names can be long ("Monitor of Built-in Audio Analog Stereo"): whole in the tooltip. -->
      <span class={["truncate type-menu-item", isFigure && "tabular-nums"]} title={label}>{label}</span>
      {#if hint}<span class={menuHintClass}>{hint}</span>{/if}
    </span>
    <!-- The check says which one is chosen without relying on colour. -->
    {#if isChecked}<Icon name="check" class="size-(--mw-component-context-menu-item-icon-size) shrink-0" />{/if}
  </button>
{/snippet}

<div class="col-span-full min-w-0">
  {#if voice.phase !== "idle" && place}
    <section aria-label={strings.voice.panel} class={cardClass}>
      <div class="flex min-w-0 items-center gap-xs">
        <div class="flex min-w-0 flex-1 flex-col">
          <!-- The bars and the words change together; the round trip is the tooltip and is read out. -->
          <p
            class={[
              "flex min-w-0 items-center gap-2xs type-user-panel-name",
              voice.phase === "connected"
                ? "text-(--mw-component-voice-panel-status-connected-text)"
                : "text-(--mw-component-voice-panel-status-pending-text)",
            ]}
            title={voice.phase === "connected" ? signal.description : undefined}
          >
            <Icon name={signal.icon} class="size-(--mw-component-voice-panel-signal-icon-size) shrink-0 {signalClass}" />
            <span class="truncate">{statusLabel}</span>
            {#if voice.phase === "connected"}<span class="sr-only">, {signal.description}</span>{/if}
          </p>
          <button
            type="button"
            aria-label={strings.voice.openServer(place.channel.name, place.guild.name)}
            class="min-w-0 self-start truncate rounded-xs text-start type-body-sm text-(--mw-component-voice-panel-channel-text) hover:text-(--mw-component-voice-panel-channel-text-hover) hover:underline pointer-coarse:min-h-(--mw-size-hit-web-min)"
            onclick={openServer}
          >
            {place.channel.name} / {place.guild.name}
          </button>
        </div>
        <IconButton
          bind:element={soundboardButton}
          icon="speaker-high"
          label={strings.soundboard.open}
          aria-haspopup="dialog"
          aria-expanded={isSoundboardOpen}
          class={[buttonClass, isSoundboardOpen && "bg-(--mw-component-user-panel-button-background-hover)"].filter(Boolean).join(" ")}
          iconClass="size-(--mw-component-user-panel-button-icon-size)"
          onpointerdown={soundboardToggle.notePress}
          onclick={() => {
            if (soundboardToggle.shouldOpenOnClick()) {
              isSoundboardOpen = true;
            }
          }}
        />
        <IconButton
          icon="phone-disconnect"
          label={strings.voice.disconnect}
          class="{buttonClass} hover:text-(--mw-component-voice-panel-hang-up-icon-hover)"
          iconClass="size-(--mw-component-user-panel-button-icon-size)"
          onclick={() => voice.leave()}
        />
      </div>
      {#if voice.canShareScreen || voice.isSharingScreen}
        <div bind:this={shareRow} class="mt-xs flex gap-(--mw-component-voice-panel-share-split-gap)">
          <!-- The label says the action; the apricot wash and the live line below say the state. -->
          <button
            type="button"
            class={[
              shareHalfClass,
              "flex-1 px-xs",
              voice.isSharingScreen
                ? "bg-(--mw-component-voice-panel-share-background-active) text-(--mw-component-voice-panel-share-text-active) hover:bg-(--mw-component-voice-panel-share-background-active-hover) hover:text-(--mw-component-voice-panel-share-text-active-hover)"
                : "bg-(--mw-component-voice-panel-share-background) text-(--mw-component-voice-panel-share-text) hover:bg-(--mw-component-voice-panel-share-background-hover) hover:text-(--mw-component-voice-panel-share-text-hover)",
            ]}
            onclick={() => (voice.isSharingScreen ? voice.stopScreenShare() : void voice.startScreenShare())}
          >
            <Icon name="monitor-arrow-up" class="size-(--mw-component-voice-panel-signal-icon-size) shrink-0" />
            <span class="truncate">{voice.isSharingScreen ? strings.screenShare.stop : strings.screenShare.share}</span>
          </button>
          <button
            bind:this={qualityButton}
            type="button"
            aria-haspopup="menu"
            aria-expanded={isQualityMenuOpen}
            aria-controls="screen-share-options-menu"
            aria-label={strings.screenShare.optionsButton(voice.screenShareQuality)}
            title={strings.screenShare.optionsMenu}
            class={[
              shareHalfClass,
              "shrink-0 px-2xs tabular-nums pointer-coarse:min-w-(--mw-size-hit-web-min) hover:bg-(--mw-component-voice-panel-share-background-hover) hover:text-(--mw-component-voice-panel-share-text-hover)",
              isQualityMenuOpen
                ? "bg-(--mw-component-voice-panel-share-background-hover) text-(--mw-component-voice-panel-share-text-hover)"
                : "bg-(--mw-component-voice-panel-share-background) text-(--mw-component-voice-panel-share-text)",
            ]}
            onpointerdown={qualityToggle.notePress}
            onclick={() => (qualityToggle.shouldOpenOnClick() ? openQualityMenu() : qualityMenu?.close())}
          >
            <span>{voice.screenShareQuality}</span>
            <Icon name="caret-up" class="size-(--mw-component-voice-panel-signal-icon-size) shrink-0" />
          </button>
        </div>
        {#if voice.isSharingScreen}
          <p class="mt-2xs flex min-w-0 items-center gap-2xs type-body-sm text-(--mw-component-voice-panel-hint-text)">
            <span aria-hidden="true" class={LIVE_BADGE_CLASS}>{strings.screenShare.live}</span>
            <span class="min-w-0 truncate">{strings.screenShare.youAreSharing}</span>
          </p>
          {#if sharedSoundLine}
            <!-- A microphone sent as the share's sound is not a voice: muting does not stop it, so say so. -->
            <p class="mt-3xs flex min-w-0 items-center gap-2xs type-body-sm text-(--mw-component-voice-panel-hint-text)" title={sharedSoundLine}>
              {#if voice.isMutedMicrophoneHeardInShare}
                <Icon
                  name="warning-circle"
                  class="size-(--mw-component-voice-panel-signal-icon-size) shrink-0 text-(--mw-component-voice-panel-problem-icon)"
                />
              {:else}
                <Icon
                  name={voice.sharedSound === "none" ? "speaker-slash" : "speaker-high"}
                  class="size-(--mw-component-voice-panel-signal-icon-size) shrink-0"
                />
              {/if}
              <span class="min-w-0 truncate">{sharedSoundLine}</span>
            </p>
            {#if voice.isMutedMicrophoneHeardInShare}
              <p class="mt-3xs type-body-sm text-(--mw-component-voice-panel-hint-text)">{strings.screenShare.mutedMicrophoneHeard}</p>
            {/if}
          {/if}
        {/if}
      {/if}
      {#if voice.screenSoundNotice}
        <!-- Advice, not an alarm: no warning sign unless the chosen source failed. -->
        <div class="mt-2xs flex items-start gap-2xs type-body-sm text-(--mw-component-voice-panel-hint-text)">
          {#if voice.screenSoundNotice === "source-unavailable"}
            <Icon name="warning-circle" class="mt-3xs size-(--mw-component-voice-panel-signal-icon-size) shrink-0 text-(--mw-component-voice-panel-problem-icon)" />
          {/if}
          <div class="flex min-w-0 flex-1 flex-col items-start gap-3xs">
            <p>{soundNoticeSentence}</p>
            <button type="button" class={noticeActionClass} onclick={openQualityMenu}>{strings.screenShare.chooseSoundSource}</button>
            {#if soundPlatform.isLinux}
              <button type="button" class={noticeActionClass} onclick={openSoundHelp}>{strings.screenShare.soundHelp.open}</button>
            {/if}
          </div>
          <IconButton
            icon="x"
            label={strings.common.close}
            class="-me-2xs -my-2xs {buttonClass}"
            iconClass="size-(--mw-component-voice-panel-signal-icon-size)"
            onclick={() => voice.dismissScreenSoundNotice()}
          />
        </div>
      {/if}
      {#if voice.screenShareProblem}
        <!-- A cancelled picker is your own doing: no warning sign, just what happened. -->
        <div class="mt-2xs flex items-start gap-2xs type-body-sm text-(--mw-component-voice-panel-hint-text)">
          {#if voice.screenShareProblem !== "denied"}
            <Icon name="warning-circle" class="mt-3xs size-(--mw-component-voice-panel-signal-icon-size) shrink-0 text-(--mw-component-voice-panel-problem-icon)" />
          {/if}
          <p class="min-w-0 flex-1 self-center">{strings.screenShare.problems[voice.screenShareProblem]}</p>
          <IconButton
            icon="x"
            label={strings.common.close}
            class="-me-2xs -my-2xs {buttonClass}"
            iconClass="size-(--mw-component-voice-panel-signal-icon-size)"
            onclick={() => voice.dismissScreenShareProblem()}
          />
        </div>
      {/if}
      {#if voice.microphoneProblem}
        <p class="mt-2xs flex gap-2xs type-body-sm text-(--mw-component-voice-panel-microphone-problem-text)">
          <Icon name="microphone-slash" class="mt-3xs size-(--mw-component-voice-panel-signal-icon-size) shrink-0" />
          <span class="min-w-0">{strings.voice.microphoneProblems[voice.microphoneProblem]}</span>
        </p>
      {/if}
      {#if voice.settings.inputMode === "push-to-talk"}
        <p class="mt-2xs flex gap-2xs type-body-sm text-(--mw-component-voice-panel-hint-text)">
          {#if !pushToTalkKey}
            <Icon name="warning-circle" class="mt-3xs size-(--mw-component-voice-panel-signal-icon-size) shrink-0" />
          {/if}
          <span class="min-w-0">{pushToTalkKey ? strings.voice.pushToTalkHint(pushToTalkKey.label) : strings.voice.pushToTalkNoKey}</span>
        </p>
      {/if}
      {#each deviceNotices as notice (notice)}
        <p class="mt-2xs flex gap-2xs type-body-sm text-(--mw-component-voice-panel-hint-text)">
          <Icon name="warning-circle" class="mt-3xs size-(--mw-component-voice-panel-signal-icon-size) shrink-0" />
          <span class="min-w-0">{notice}</span>
        </p>
      {/each}
    </section>
  {:else if look}
    <section aria-label={strings.voice.panel} class={cardClass}>
      <div role="alert" class="flex min-w-0 items-start gap-xs">
        <Icon
          name={look.tone === "error" ? "warning-circle" : "speaker-high"}
          class="mt-3xs size-(--mw-component-voice-panel-signal-icon-size) shrink-0 {look.tone === 'error'
            ? 'text-(--mw-component-voice-panel-problem-icon)'
            : 'text-(--mw-component-voice-panel-notice-icon)'}"
        />
        <div class="flex min-w-0 flex-1 flex-col gap-3xs">
          <p class="type-user-panel-name text-(--mw-component-voice-panel-problem-title-text)">{look.title}</p>
          <p class="type-body-sm text-(--mw-component-voice-panel-problem-text)">{look.sentence}</p>
        </div>
        <IconButton
          icon="x"
          label={strings.common.close}
          class="-me-2xs -mt-2xs {buttonClass}"
          iconClass="size-(--mw-component-voice-panel-signal-icon-size)"
          onclick={() => voice.dismissProblem()}
        />
      </div>
      {#if look.canRetry}
        <div class="mt-xs flex">
          <Button variant="secondary" isFullWidth onclick={() => voice.retry()}>{strings.common.tryAgain}</Button>
        </div>
      {/if}
    </section>
  {/if}
  {#if isSoundboardOpen && soundboardButton && place}
    {@const guildId = place.guild.id}
    <SoundboardPopover
      {client}
      anchor={soundboardButton}
      isCompact={!hasRoomForSoundboardBeside.current}
      onaddsounds={holds(place.guild, Permission.MANAGE_EXPRESSIONS)
        ? () => {
            isSoundboardOpen = false;
            onaddsounds(guildId);
          }
        : null}
      onclose={(shouldReturnFocus) => {
        isSoundboardOpen = false;
        if (shouldReturnFocus) {
          soundboardButton?.focus();
        }
      }}
    />
  {/if}
  {#if voice.phase !== "idle"}
    <ContextMenu
      bind:this={qualityMenu}
      bind:isOpen={isQualityMenuOpen}
      id="screen-share-options-menu"
      label={strings.screenShare.optionsMenu}
      gapToken="space-2xs"
    >
      <div role="group" aria-labelledby="screen-share-quality-group">
        <p id="screen-share-quality-group" class={menuGroupLabelClass}>{strings.screenShare.qualityGroup}</p>
        {#each SCREEN_SHARE_QUALITIES as quality (quality)}
          {@render choice(quality === voice.screenShareQuality, quality, strings.screenShare.qualityHints[quality], () => chooseQuality(quality), true)}
        {/each}
      </div>
      <hr class={menuSeparatorClass} />
      <div role="group" aria-labelledby="screen-share-sound-group">
        <p id="screen-share-sound-group" class={menuGroupLabelClass}>{strings.screenShare.soundGroup}</p>
        {@render choice(
          shownSoundDeviceId === null,
          strings.screenShare.soundFromScreen,
          soundAdvice === "sound-source" ? "" : strings.screenShare.soundFromScreenHint,
          () => chooseSound(null),
        )}
        {#each voice.screenSoundDevices as device (device.deviceId)}
          {@render choice(
            device.deviceId === shownSoundDeviceId,
            device.label,
            device.isSystemSound ? strings.screenShare.systemSoundHint : strings.screenShare.heardWhenMutedHint,
            () => chooseSound(device.deviceId),
          )}
        {/each}
      </div>
      {#if soundPlatform.isLinux}
        <hr class={menuSeparatorClass} />
        <button type="button" role="menuitem" class={menuItemClass} onclick={openSoundHelp}>
          <span class="type-menu-item">{strings.screenShare.soundHelp.open}</span>
        </button>
      {/if}
    </ContextMenu>
  {/if}
  {#if isSoundHelpOpen}
    <LazyComponent load={() => import("./ScreenSoundHelpDialog.svelte")} onclose={() => (isSoundHelpOpen = false)}>
      {#snippet children({ default: ScreenSoundHelpDialog })}
        <ScreenSoundHelpDialog onclose={() => (isSoundHelpOpen = false)} />
      {/snippet}
    </LazyComponent>
  {/if}
  <p class="sr-only" aria-live="polite">{phaseSentence}</p>
  <p class="sr-only" aria-live="polite">{voice.isSharingScreen ? strings.screenShare.youAreSharing : ""}</p>
  <p class="sr-only" aria-live="polite">{voice.phase === "idle" ? "" : (voice.screenShareProblem && strings.screenShare.problems[voice.screenShareProblem]) || ""}</p>
  <p class="sr-only" aria-live="polite">{soundNoticeSentence && `${sharedSoundLine}. ${soundNoticeSentence}`}</p>
  <p class="sr-only" aria-live="polite">{voice.isMutedMicrophoneHeardInShare ? strings.screenShare.mutedMicrophoneHeard : ""}</p>
  <p class="sr-only" aria-live="polite">{client.voiceAnnouncement}</p>
  <p class="sr-only" aria-live="polite">{voice.phase === "idle" ? "" : deviceNotices.join(" ")}</p>
</div>
