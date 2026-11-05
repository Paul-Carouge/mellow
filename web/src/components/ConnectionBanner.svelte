<script lang="ts">
  import { tick, untrack } from "svelte";
  import { online } from "svelte/reactivity/window";

  import { strings } from "../i18n";
  import type { ConnectionStatus } from "../lib/client.svelte";
  import {
    BACK_ONLINE_SHOWN_MILLISECONDS,
    type ConnectionTrouble,
    STILL_RECONNECTING_AFTER_MILLISECONDS,
    connectionTrouble,
    isComingBackOnline,
    troubleToRemember,
  } from "./connection-notice";
  import Icon from "./Icon.svelte";

  // Full-width notice at the top of the window while the device is offline or the gateway
  // connects or reconnects, then "Back online" for a moment (DESIGN.md §5.2 connection dock, §6.37).
  interface Props {
    status: ConnectionStatus;
    /** The gateway checks its socket after a network change: not live until it answers. */
    isCheckingLink: boolean;
    /** Skips the backoff wait: offered after reconnecting for long. */
    onretry: () => void;
    /** Takes the focus when "Retry now" goes away under it, so it does not fall to the page. */
    fallbackFocus: () => void;
  }

  let { status, isCheckingLink, onretry, fallbackFocus }: Props = $props();

  let hasBeenReconnectingLong = $state(false);
  $effect(() => {
    if (status !== "reconnecting") {
      return;
    }
    const timer = setTimeout(() => (hasBeenReconnectingLong = true), STILL_RECONNECTING_AFTER_MILLISECONDS);
    return () => {
      clearTimeout(timer);
      hasBeenReconnectingLong = false;
    };
  });

  // `online.current` is undefined only outside a browser.
  const trouble = $derived(connectionTrouble(status, online.current ?? true, hasBeenReconnectingLong, isCheckingLink));

  let retryButton: HTMLButtonElement | undefined = $state();
  // Before the DOM update: once the button is gone, the focus has already fallen to the page.
  $effect.pre(() => {
    const current = trouble;
    untrack(() => {
      if (current !== "still-reconnecting" && retryButton && document.activeElement === retryButton) {
        void tick().then(fallbackFocus);
      }
    });
  });

  let isBackOnlineShown = $state(false);
  let previousTrouble: ConnectionTrouble | null = null;
  let backOnlineTimer: ReturnType<typeof setTimeout> | undefined;
  $effect(() => {
    const current = trouble;
    untrack(() => {
      clearTimeout(backOnlineTimer);
      isBackOnlineShown = isComingBackOnline(previousTrouble, current);
      if (isBackOnlineShown) {
        backOnlineTimer = setTimeout(() => (isBackOnlineShown = false), BACK_ONLINE_SHOWN_MILLISECONDS);
      }
      previousTrouble = troubleToRemember(previousTrouble, current);
    });
  });
  $effect(() => () => clearTimeout(backOnlineTimer));

  const troubleText: Record<ConnectionTrouble, string> = {
    offline: strings.connection.offline,
    connecting: strings.connection.connecting,
    checking: strings.connection.connecting,
    reconnecting: strings.connection.reconnecting,
    "still-reconnecting": strings.connection.stillReconnecting,
  };
</script>

<!-- Always in the page: screen readers announce a live region's changes, not its insertion. -->
<div role="status" class="contents">
  {#if trouble}
    <div
      class="flex min-h-(--mw-component-notice-bar-min-height) flex-wrap items-center justify-center gap-x-(--mw-component-notice-bar-icon-gap) gap-y-2xs bg-(--mw-component-notice-bar-warning-background) px-(--mw-component-notice-bar-padding-inline) py-(--mw-component-notice-bar-padding-block) text-(--mw-component-notice-bar-warning-text)"
    >
      <p class="flex min-w-0 items-center gap-(--mw-component-notice-bar-icon-gap) type-button">
        <Icon name="wifi-slash" class="size-(--mw-component-notice-bar-icon-size) shrink-0" />
        <span class="min-w-0">{troubleText[trouble]}</span>
      </p>
      {#if trouble === "still-reconnecting"}
        <!-- The bar's own ink as the fill: a second colour on the amber would compete with it. The
             focus ring goes inside: the blue ring is 1.2:1 on the amber in dark. -->
        <button
          bind:this={retryButton}
          type="button"
          class="inline-flex h-(--mw-component-button-height-small) shrink-0 items-center rounded-(--mw-component-button-radius) bg-(--mw-component-notice-bar-warning-text) px-sm type-button text-(--mw-component-notice-bar-warning-background) hover:underline focus-visible:focus-ring-inset pointer-coarse:h-(--mw-size-hit-web-min)"
          onclick={onretry}>{strings.connection.retryNow}</button
        >
      {/if}
    </div>
  {:else if isBackOnlineShown}
    <p
      class="flex min-h-(--mw-component-notice-bar-min-height) items-center justify-center gap-(--mw-component-notice-bar-icon-gap) bg-positive-wash px-(--mw-component-notice-bar-padding-inline) py-(--mw-component-notice-bar-padding-block) type-button text-text-positive"
    >
      <Icon name="check" class="size-(--mw-component-notice-bar-icon-size) shrink-0" />
      {strings.connection.backOnline}
    </p>
  {/if}
</div>
