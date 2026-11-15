<script lang="ts">
  import { strings } from "../i18n";
  import type { MellowClient } from "../lib/client.svelte";
  import type { Snowflake } from "../lib/protocol";
  import { buttonVariantClasses } from "./button-variants";
  import Icon from "./Icon.svelte";

  // "Appel en cours · Rejoindre" (DESIGN.md §5.6.3, ADR-0062): over the conversation, under its
  // header like the unread bar, so the rows being read do not move. It shows while the
  // conversation's call goes on without the reader (declined, missed, or left), and while they are
  // in it with the call view closed ("Afficher l'appel"). A call ringing them has its own card.
  interface Props {
    client: MellowClient;
    channelId: Snowflake;
  }

  let { client, channelId }: Props = $props();

  const call = $derived(client.callsByChannelId.get(channelId) ?? null);
  const phase = $derived(client.callPhaseOf(channelId));
  const isInIt = $derived(client.voice.channelId === channelId && client.voice.phase !== "idle");
  const isShown = $derived(phase === "joinable" || (isInIt && !client.voice.isCallViewOpen && client.voice.watchedStreamUserId === null));

  // As tall as the banner allows with a mouse; the whole banner's height under a finger.
  const actionClass =
    "inline-flex h-[calc(var(--mw-component-call-banner-height)_-_2*var(--mw-space-2xs))] shrink-0 items-center gap-2xs rounded-sm px-sm type-button whitespace-nowrap transition-colors duration-(--mw-motion-duration-fast) ease-(--mw-motion-easing-standard) pointer-coarse:h-(--mw-size-hit-web-min)";

  let isJoining = $state(false);
  async function join() {
    if (isJoining) {
      return;
    }
    isJoining = true;
    try {
      await client.joinCall(channelId);
    } finally {
      isJoining = false;
    }
  }
</script>

{#if call && isShown}
  <div
    role="region"
    aria-label={strings.calls.ongoing}
    class="call-banner absolute inset-x-(--mw-component-call-banner-margin) top-[calc(var(--mw-component-chat-header-height)_+_var(--mw-space-3xs))] z-10 flex h-(--mw-component-call-banner-height) items-center gap-xs rounded-(--mw-component-call-banner-radius) bg-(--mw-component-call-banner-background) ps-(--mw-component-call-banner-padding-inline) pe-2xs pointer-coarse:pe-0"
  >
    <Icon name="phone-call" class="size-(--mw-component-call-banner-icon-size) shrink-0 text-(--mw-component-call-banner-icon)" />
    <p class="min-w-0 flex-1 truncate type-body">
      <span class="font-semibold text-(--mw-component-call-banner-text)">{strings.calls.ongoing}</span>
      <span class="text-(--mw-component-call-banner-secondary-text)">· {strings.calls.peopleInCall(call.participants.length)}</span>
    </p>
    {#if isInIt}
      <button type="button" class="{actionClass} text-(--mw-component-call-banner-text) underline-offset-2 hover:underline" onclick={() => client.voice.openCallView()}>
        {strings.calls.showCall}
      </button>
    {:else}
      <button
        type="button"
        aria-disabled={isJoining || undefined}
        class="{actionClass} {buttonVariantClasses('primary', false)}"
        onclick={() => void join()}
      >
        <Icon name="phone" class="size-(--mw-component-call-message-icon-size) shrink-0" />
        {strings.calls.join}
      </button>
    {/if}
  </div>
{/if}
