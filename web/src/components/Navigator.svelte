<script lang="ts">
  import type { SvelteSet } from "svelte/reactivity";

  import type { MellowClient } from "../lib/client.svelte";
  import type { Snowflake } from "../lib/protocol";
  import ChannelSidebar, { type ServerDialog } from "./ChannelSidebar.svelte";
  import ServerRail from "./ServerRail.svelte";
  import UserCard from "./UserCard.svelte";

  // The navigator (ADR-0022 §1): one card holding the server strip, the channel list and, across
  // its whole width at the foot, the user card. In narrow windows it is the drawer's content.
  interface Props {
    client: MellowClient;
    collapsedCategoryIds: SvelteSet<Snowflake>;
    /** In the drawer the card meets the window's leading edge: only its trailing corners round. */
    isInDrawer: boolean;
    onaddserver: () => void;
    onopendialog: (dialog: ServerDialog) => void;
    onnavigate: () => void;
    onopenusersettings: () => void;
  }

  let { client, collapsedCategoryIds, isInDrawer, onaddserver, onopendialog, onnavigate, onopenusersettings }: Props =
    $props();
</script>

<div
  class={[
    "grid h-full min-h-0 grid-cols-[var(--mw-component-navigator-strip-width)_var(--mw-component-sidebar-width)] grid-rows-[minmax(0,1fr)_auto] bg-(--mw-component-navigator-background)",
    isInDrawer ? "rounded-e-(--mw-component-navigator-radius)" : "rounded-(--mw-component-navigator-radius)",
  ]}
>
  <ServerRail {client} {onaddserver} {onnavigate} />
  <ChannelSidebar {client} {collapsedCategoryIds} {onopendialog} {onnavigate} />
  {#if client.user}
    <UserCard user={client.user} {onopenusersettings} />
  {/if}
</div>
