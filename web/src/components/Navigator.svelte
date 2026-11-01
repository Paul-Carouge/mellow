<script lang="ts">
  import type { SvelteSet } from "svelte/reactivity";

  import type { MellowClient } from "../lib/client.svelte";
  import type { Channel, Snowflake } from "../lib/protocol";
  import ChannelSidebar, { type ServerDialog } from "./ChannelSidebar.svelte";
  import ServerRail from "./ServerRail.svelte";
  import UserCard from "./UserCard.svelte";
  import VoicePanel from "./VoicePanel.svelte";

  // The navigator (ADR-0022 §1): one card holding the server strip, the channel list and, across
  // its whole width at the foot, the voice panel while connected, then the user card. In narrow
  // windows it is the drawer's content.
  interface Props {
    client: MellowClient;
    collapsedCategoryIds: SvelteSet<Snowflake>;
    /** In the drawer the card meets the window's leading edge: only its trailing corners round. */
    isInDrawer: boolean;
    onaddserver: () => void;
    /** The "+" beside "Direct messages": the "New message" picker. */
    onnewmessage: () => void;
    onopendialog: (dialog: ServerDialog, guildId: Snowflake) => void;
    oncreatechannelin: (category: Channel) => void;
    oneditchannel: (channel: Channel) => void;
    onnavigate: () => void;
    onopenusersettings: () => void;
  }

  let {
    client,
    collapsedCategoryIds,
    isInDrawer,
    onaddserver,
    onnewmessage,
    onopendialog,
    oncreatechannelin,
    oneditchannel,
    onnavigate,
    onopenusersettings,
  }: Props =
    $props();
</script>

<div
  class={[
    "grid h-full min-h-0 grid-rows-[minmax(0,1fr)_auto_auto] bg-(--mw-component-navigator-background)",
    isInDrawer
      ? // The drawer leaves the sheet's peek visible, so on the narrowest phones the channel list
        // gives up width instead of being cut; the card also keeps clear of notches and the
        // home indicator, its own colour running under them.
        "grid-cols-[var(--mw-component-navigator-strip-width)_minmax(0,var(--mw-component-sidebar-width))] rounded-e-(--mw-component-navigator-radius) pt-[env(safe-area-inset-top)] pb-[env(safe-area-inset-bottom)] pl-[env(safe-area-inset-left)]"
      : "grid-cols-[var(--mw-component-navigator-strip-width)_var(--mw-component-sidebar-width)] rounded-(--mw-component-navigator-radius)",
  ]}
>
  <ServerRail {client} {onaddserver} {onopendialog} {onnavigate} />
  <ChannelSidebar {client} {collapsedCategoryIds} {onnewmessage} {onopendialog} {oncreatechannelin} {oneditchannel} {onnavigate} />
  <!-- Voice stays connected wherever the reader goes: its panel lives here, above the user card. -->
  <VoicePanel {client} {onnavigate} onaddsounds={(guildId) => onopendialog("server-soundboard", guildId)} />
  {#if client.user}
    <UserCard {client} user={client.user} {onopenusersettings} />
  {/if}
</div>
