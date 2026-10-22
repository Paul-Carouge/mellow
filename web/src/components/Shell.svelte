<script lang="ts">
  import { tick } from "svelte";
  import { MediaQuery, SvelteSet } from "svelte/reactivity";

  import type { MellowClient } from "../lib/client.svelte";
  import type { Snowflake } from "../lib/protocol";
  import AddServerDialog from "./AddServerDialog.svelte";
  import type { ServerDialog } from "./ChannelSidebar.svelte";
  import ChatPane from "./ChatPane.svelte";
  import ConnectionBanner from "./ConnectionBanner.svelte";
  import CreateChannelDialog from "./CreateChannelDialog.svelte";
  import Drawer from "./Drawer.svelte";
  import InvitePeopleDialog from "./InvitePeopleDialog.svelte";
  import LeaveServerDialog from "./LeaveServerDialog.svelte";
  import MemberList from "./MemberList.svelte";
  import Navigator from "./Navigator.svelte";
  import QuickSwitcher from "./QuickSwitcher.svelte";
  import { minimumWidthQueryForPanes } from "./tokens";
  import UserSettings from "./UserSettings.svelte";

  // The signed-in shell (ADR-0022 §1): cards on the dusk ground. The navigator card and the
  // conversation sheet sit side by side inside the inset; the members side sheet opens on demand,
  // docked when it fits and over the sheet otherwise. Each breakpoint is the sum of what it must
  // fit: below it the navigator becomes a drawer (816 px) or the side sheet an overlay (1064 px).
  interface Props {
    client: MellowClient;
  }

  let { client }: Props = $props();

  const NAVIGATOR_AND_SHEET = [
    "size-sheet-inset",
    "component-navigator-strip-width",
    "component-sidebar-width",
    "size-sheet-inset",
    "size-chat-min-width",
    "size-sheet-inset",
  ];
  const hasRoomForNavigator = new MediaQuery(minimumWidthQueryForPanes(NAVIGATOR_AND_SHEET));
  const hasRoomForSideSheet = new MediaQuery(
    minimumWidthQueryForPanes([...NAVIGATOR_AND_SHEET, "component-side-sheet-width", "size-sheet-inset"]),
  );

  let chatPane = $state<{ focusComposer: () => void } | null>(null);
  const collapsedCategoryIds = new SvelteSet<Snowflake>();
  // Closed by default, remembered per server while the tab lives (ADR-0022 §1).
  const serverIdsWithMemberSheet = new SvelteSet<Snowflake>();
  let isMemberOverlayOpen = $state(false);
  let isNavigationDrawerOpen = $state(false);
  let openDialog = $state<"add-server" | ServerDialog | null>(null);
  let isUserSettingsOpen = $state(false);
  let isQuickSwitcherOpen = $state(false);

  const guild = $derived(client.selectedGuild);
  const hasMemberSheet = $derived(guild !== null && client.selectedChannel !== null);
  const isMemberSheetOpen = $derived(
    hasMemberSheet &&
      (hasRoomForSideSheet.current ? guild !== null && serverIdsWithMemberSheet.has(guild.id) : isMemberOverlayOpen),
  );

  // A drawer or overlay left open while the window widens would come back on the next shrink.
  $effect(() => {
    if (hasRoomForNavigator.current) {
      isNavigationDrawerOpen = false;
    }
    if (hasRoomForSideSheet.current) {
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

  function toggleMemberSheet() {
    if (!guild) {
      return;
    }
    if (!hasRoomForSideSheet.current) {
      isMemberOverlayOpen = !isMemberOverlayOpen;
    } else if (serverIdsWithMemberSheet.has(guild.id)) {
      serverIdsWithMemberSheet.delete(guild.id);
    } else {
      serverIdsWithMemberSheet.add(guild.id);
    }
  }

  async function closeMemberSheet() {
    if (hasRoomForSideSheet.current && guild) {
      serverIdsWithMemberSheet.delete(guild.id);
    }
    isMemberOverlayOpen = false;
    await tick();
    // The close button is gone with the sheet.
    focusComposer();
  }

  function toggleMemberSheetWithShortcut(event: KeyboardEvent) {
    // Ctrl+U, as on Discord; not while a dialog or the settings cover the shell.
    const isShortcut =
      (event.ctrlKey || event.metaKey) && !event.shiftKey && !event.altKey && event.key.toLowerCase() === "u";
    if (!isShortcut || !hasMemberSheet || isShellCovered()) {
      return;
    }
    event.preventDefault();
    toggleMemberSheet();
  }

  /** Ctrl+K, as on Discord, from anywhere in the shell (the composer included). */
  function openQuickSwitcherWithShortcut(event: KeyboardEvent) {
    const isShortcut =
      (event.ctrlKey || event.metaKey) && !event.shiftKey && !event.altKey && event.key.toLowerCase() === "k";
    if (!isShortcut || isShellCovered()) {
      return;
    }
    event.preventDefault();
    isQuickSwitcherOpen = true;
  }

  /** A dialog, the settings or a drawer covers the shell: its shortcuts wait. */
  function isShellCovered(): boolean {
    return openDialog !== null || isUserSettingsOpen || isNavigationDrawerOpen || isQuickSwitcherOpen;
  }
</script>

<svelte:window
  onkeydown={(event) => {
    toggleMemberSheetWithShortcut(event);
    openQuickSwitcherWithShortcut(event);
  }}
/>

{#snippet navigator(isInDrawer: boolean)}
  <Navigator
    {client}
    {collapsedCategoryIds}
    {isInDrawer}
    onaddserver={() => (openDialog = "add-server")}
    onopendialog={(dialog) => (openDialog = dialog)}
    onnavigate={() => void closeDrawerAfterNavigation()}
    onopenusersettings={() => (isUserSettingsOpen = true)}
  />
{/snippet}

<div class="flex h-dvh flex-col bg-surface-rail font-sans text-text-normal">
  <ConnectionBanner status={client.status} />
  <!-- As in the reference mockup, the ground shows twice the inset above the cards and after the
       last one (where the mockup keeps a row for its top bar and a column for the side sheet). -->
  <div
    class={[
      "flex min-h-0 flex-1",
      hasRoomForNavigator.current && [
        "gap-(--mw-size-sheet-inset) ps-(--mw-size-sheet-inset) pt-[calc(2*var(--mw-size-sheet-inset))] pb-(--mw-size-sheet-inset)",
        isMemberSheetOpen && hasRoomForSideSheet.current ? "pe-(--mw-size-sheet-inset)" : "pe-[calc(2*var(--mw-size-sheet-inset))]",
      ],
    ]}
  >
    {#if hasRoomForNavigator.current}
      {@render navigator(false)}
    {:else if isNavigationDrawerOpen}
      <Drawer label="Servers and channels" side="start" onclose={() => (isNavigationDrawerOpen = false)}>
        {@render navigator(true)}
      </Drawer>
    {/if}

    <main class="flex min-w-0 flex-1">
      <ChatPane
        bind:this={chatPane}
        {client}
        hasNavigationDrawer={!hasRoomForNavigator.current}
        onopennavigation={() => (isNavigationDrawerOpen = true)}
        onopenquickswitcher={() => (isQuickSwitcherOpen = true)}
        isMemberListOpen={isMemberSheetOpen}
        ontogglememberlist={toggleMemberSheet}
        onaddserver={() => (openDialog = "add-server")}
      />
    </main>

    {#if guild && hasMemberSheet && isMemberSheetOpen}
      {#if hasRoomForSideSheet.current}
        {#key guild.id}
          <MemberList {client} {guild} isDocked onclose={() => void closeMemberSheet()} onopenedconversation={focusComposer} />
        {/key}
      {:else}
        <Drawer
          label="Members of {guild.name}"
          side="end"
          isFullWindow={!hasRoomForNavigator.current}
          onclose={() => (isMemberOverlayOpen = false)}
        >
          <MemberList
            {client}
            {guild}
            isDocked={false}
            isFullWindow={!hasRoomForNavigator.current}
            onclose={() => (isMemberOverlayOpen = false)}
            onopenedconversation={() => void closeMemberSheet()}
          />
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

{#if isUserSettingsOpen}
  <UserSettings {client} isWide={hasRoomForNavigator.current} onclose={() => (isUserSettingsOpen = false)} />
{/if}

{#if isQuickSwitcherOpen}
  <QuickSwitcher {client} onclose={() => (isQuickSwitcherOpen = false)} onnavigated={focusComposer} />
{/if}
