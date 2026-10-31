<script lang="ts">
  import { strings } from "../i18n";
  import type { MellowClient } from "../lib/client.svelte";
  import Button from "./Button.svelte";
  import Icon from "./Icon.svelte";
  import IconButton from "./IconButton.svelte";
  import { problemLook, signalLook } from "./voice-panel";

  // The voice panel (DESIGN.md §5.3, ADR-0030): a card like the user card, directly above it, in
  // the connection dock's slot (§4.2). While connected: the state with its signal bars, the channel
  // and server (a link to the server), and Disconnect. After a problem: what happened, how to fix
  // it, Try again when it may pass, and Close. Nothing while idle; the live regions always exist,
  // so their first change is read out.
  interface Props {
    client: MellowClient;
    /** After the channel line opened its server (the narrow layout closes its drawer). */
    onnavigate: () => void;
  }

  let { client, onnavigate }: Props = $props();

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
        return strings.voice.connectedTo(name);
      case "reconnecting":
        return strings.voice.reconnectingTo(name);
      default:
        return "";
    }
  });
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
  const buttonClass =
    "size-(--mw-component-user-panel-button-size) text-(--mw-component-user-panel-button-icon) hover:bg-(--mw-component-user-panel-button-background-hover)";
</script>

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
          icon="phone-disconnect"
          label={strings.voice.disconnect}
          class="{buttonClass} hover:text-(--mw-component-voice-panel-hang-up-icon-hover)"
          iconClass="size-(--mw-component-user-panel-button-icon-size)"
          onclick={() => voice.leave()}
        />
      </div>
      {#if voice.microphoneProblem}
        <p class="mt-2xs flex gap-2xs type-body-sm text-(--mw-component-voice-panel-microphone-problem-text)">
          <Icon name="microphone-slash" class="mt-3xs size-(--mw-component-voice-panel-signal-icon-size) shrink-0" />
          <span class="min-w-0">{strings.voice.microphoneProblems[voice.microphoneProblem]}</span>
        </p>
      {/if}
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
  <p class="sr-only" aria-live="polite">{phaseSentence}</p>
  <p class="sr-only" aria-live="polite">{client.voiceAnnouncement}</p>
</div>
