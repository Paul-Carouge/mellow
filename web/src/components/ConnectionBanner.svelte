<script lang="ts">
  import type { ConnectionStatus } from "../lib/client.svelte";
  import Icon from "./Icon.svelte";

  // Full-width notice while the gateway connects or reconnects, as on desktop (DESIGN.md §6.37).
  interface Props {
    status: ConnectionStatus;
  }

  let { status }: Props = $props();

  const notice = $derived(
    status === "connecting" ? "Connecting…" : status === "reconnecting" ? "Connection lost. Reconnecting…" : "",
  );
</script>

<!-- Always in the page: screen readers announce a live region's changes, not its insertion. -->
<div role="status" class="contents">
  {#if notice}
    <p
      class="flex min-h-(--mw-component-notice-bar-min-height) items-center justify-center gap-(--mw-component-notice-bar-icon-gap) bg-(--mw-component-notice-bar-warning-background) px-(--mw-component-notice-bar-padding-inline) py-(--mw-component-notice-bar-padding-block) type-button text-(--mw-component-notice-bar-warning-text)"
    >
      <Icon name="wifi-slash" class="size-(--mw-component-notice-bar-icon-size)" />
      {notice}
    </p>
  {/if}
</div>
