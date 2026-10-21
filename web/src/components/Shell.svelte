<script lang="ts">
  import { tick } from "svelte";
  import { MediaQuery, SvelteSet } from "svelte/reactivity";

  import type { MellowClient } from "../lib/client.svelte";
  import type { Snowflake } from "../lib/protocol";
  import AddServerDialog from "./AddServerDialog.svelte";
  import ChannelSidebar, { type ServerDialog } from "./ChannelSidebar.svelte";
  import ChatPane from "./ChatPane.svelte";
  import ConnectionBanner from "./ConnectionBanner.svelte";
  import CreateChannelDialog from "./CreateChannelDialog.svelte";
  import Drawer from "./Drawer.svelte";
  import InvitePeopleDialog from "./InvitePeopleDialog.svelte";
  import LeaveServerDialog from "./LeaveServerDialog.svelte";
  import MemberList from "./MemberList.svelte";
  import ServerRail from "./ServerRail.svelte";
  import { minimumWidthQueryForPanes } from "./tokens";

  // The signed-in shell (DESIGN.md §4.2): rail, channel sidebar, chat and member list. Each
  // breakpoint is the sum of the panes it must fit: below the full width the member list opens
  // over the chat; below the sidebar width, rail and sidebar become a drawer.
  interface Props {
    client: MellowClient;
  }

  let { client }: Props = $props();

  const NAVIGATION_PANES = ["size-rail-width", "size-sidebar-width", "size-chat-min-width"];
  const hasRoomForSidebar = new MediaQuery(minimumWidthQueryForPanes(NAVIGATION_PANES));
  const hasRoomForMemberList = new MediaQuery(
    minimumWidthQueryForPanes([...NAVIGATION_PANES, "size-member-list-width"]),
  );

  let chatPane = $state<{ focusComposer: () => void } | null>(null);
  const collapsedCategoryIds = new SvelteSet<Snowflake>();
  let isMemberListShownInline = $state(true);
  let isMemberOverlayOpen = $state(false);
  let isNavigationDrawerOpen = $state(false);
  let openDialog = $state<"add-server" | ServerDialog | null>(null);

  const guild = $derived(client.selectedGuild);
  const isMemberListOpen = $derived(hasRoomForMemberList.current ? isMemberListShownInline : isMemberOverlayOpen);

  // A drawer or overlay left open while the window widens would come back on the next shrink.
  $effect(() => {
    if (hasRoomForSidebar.current) {
      isNavigationDrawerOpen = false;
    }
    if (hasRoomForMemberList.current) {
      isMemberOverlayOpen = false;
    }
  });

  function focusComposer() {
    chatPane?.focusComposer();
  }

  /** After choosing a channel in the drawer, the chat comes back with the composer focused. */
  async function closeDrawerAfterNavigation() {
    if (!isNavigationDrawerOpen) {
      return;
    }
    isNavigationDrawerOpen = false;
    await tick();
    focusComposer();
  }

  function toggleMemberList() {
    if (hasRoomForMemberList.current) {
      isMemberListShownInline = !isMemberListShownInline;
    } else {
      isMemberOverlayOpen = !isMemberOverlayOpen;
    }
  }

  async function closeMemberOverlayAfterMessage() {
    isMemberOverlayOpen = false;
    await tick();
    focusComposer();
  }
</script>

{#snippet navigation()}
  <ServerRail {client} onaddserver={() => (openDialog = "add-server")} onnavigate={() => void closeDrawerAfterNavigation()} />
  <ChannelSidebar
    {client}
    {collapsedCategoryIds}
    onopendialog={(dialog) => (openDialog = dialog)}
    onnavigate={() => void closeDrawerAfterNavigation()}
  />
{/snippet}

<div class="flex h-dvh flex-col bg-surface-chat font-sans text-text-normal">
  <ConnectionBanner status={client.status} />
  <div class="flex min-h-0 flex-1">
    {#if hasRoomForSidebar.current}
      {@render navigation()}
    {:else if isNavigationDrawerOpen}
      <Drawer label="Servers and channels" side="start" onclose={() => (isNavigationDrawerOpen = false)}>
        {@render navigation()}
      </Drawer>
    {/if}

    <main class="flex min-w-0 flex-1">
      <ChatPane
        bind:this={chatPane}
        {client}
        hasNavigationDrawer={!hasRoomForSidebar.current}
        onopennavigation={() => (isNavigationDrawerOpen = true)}
        {isMemberListOpen}
        ontogglememberlist={toggleMemberList}
        onaddserver={() => (openDialog = "add-server")}
      />
    </main>

    {#if guild && client.selectedChannel}
      {#if hasRoomForMemberList.current}
        {#if isMemberListShownInline}
          <MemberList {client} {guild} onopenedconversation={focusComposer} />
        {/if}
      {:else if isMemberOverlayOpen}
        <Drawer label="Members of {guild.name}" side="end" onclose={() => (isMemberOverlayOpen = false)}>
          <MemberList {client} {guild} onopenedconversation={() => void closeMemberOverlayAfterMessage()} />
        </Drawer>
      {/if}
    {/if}
  </div>
</div>

{#if openDialog === "add-server"}
  <AddServerDialog {client} onclose={() => (openDialog = null)} fallbackFocus={focusComposer} />
{:else if openDialog === "invite-people" && guild}
  <InvitePeopleDialog {client} serverName={guild.name} onclose={() => (openDialog = null)} fallbackFocus={focusComposer} />
{:else if openDialog === "create-channel"}
  <CreateChannelDialog {client} onclose={() => (openDialog = null)} fallbackFocus={focusComposer} />
{:else if openDialog === "leave-server" && guild}
  <LeaveServerDialog {client} serverName={guild.name} onclose={() => (openDialog = null)} fallbackFocus={focusComposer} />
{/if}
