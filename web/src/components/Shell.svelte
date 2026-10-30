<script lang="ts">
  import { onMount, tick } from "svelte";
  import { MediaQuery, SvelteSet } from "svelte/reactivity";

  import { strings } from "../i18n";
  import type { MellowClient } from "../lib/client.svelte";
  import type { Channel, Snowflake } from "../lib/protocol";
  import AddServerDialog from "./AddServerDialog.svelte";
  import ChannelSettings from "./ChannelSettings.svelte";
  import type { ServerDialog } from "./ChannelSidebar.svelte";
  import ChatPane from "./ChatPane.svelte";
  import ConnectionBanner from "./ConnectionBanner.svelte";
  import CreateChannelDialog from "./CreateChannelDialog.svelte";
  import Drawer from "./Drawer.svelte";
  import type { FriendsTab } from "./friends";
  import FriendsPane from "./FriendsPane.svelte";
  import { conversationDisplayName, isGroupConversation } from "./group-conversations";
  import GroupMembersSheet from "./GroupMembersSheet.svelte";
  import InvitePeopleDialog from "./InvitePeopleDialog.svelte";
  import LeaveServerDialog from "./LeaveServerDialog.svelte";
  import MemberList from "./MemberList.svelte";
  import Navigator from "./Navigator.svelte";
  import {
    closeNotificationDialog,
    loadNotificationPreferences,
    notificationDialogRequest,
  } from "./notification-preferences.svelte";
  import NotificationPermissionBar from "./NotificationPermissionBar.svelte";
  import NotificationSettingsDialog from "./NotificationSettingsDialog.svelte";
  import { attentionCount, noteChannelSeen, showAttentionCount, startNotifier } from "./notifier.svelte";
  import PeoplePickerDialog from "./PeoplePickerDialog.svelte";
  import QuickSwitcher from "./QuickSwitcher.svelte";
  import ServerSettings from "./ServerSettings.svelte";
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
  let friendsPane = $state<{ focusStart: () => void } | null>(null);
  let friendsTab = $state<FriendsTab>("online");
  const collapsedCategoryIds = new SvelteSet<Snowflake>();
  // Closed by default, remembered per server or group while the tab lives (ADR-0022 §1).
  const placeIdsWithMemberSheet = new SvelteSet<Snowflake>();
  let isMemberOverlayOpen = $state(false);
  let isNavigationDrawerOpen = $state(false);
  let openDialog = $state<"add-server" | "channel-settings" | "new-message" | "add-to-group" | ServerDialog | null>(
    null,
  );
  /** Group results said by the shell, which outlives the group's own views (ADR-0026). */
  let announcement = $state("");
  let isUserSettingsOpen = $state(false);
  let isQuickSwitcherOpen = $state(false);
  /** The server the invite, settings or leave dialog is about, kept from the menu that opened it
   * (the strip's menu works on any server): a deleted server must not hand its settings to the next. */
  let dialogGuildId = $state<Snowflake | null>(null);
  const dialogGuild = $derived(client.guilds.find((candidate) => candidate.id === dialogGuildId) ?? null);
  /** The channel whose gear opened its settings. */
  let channelSettingsChannelId = $state<Snowflake | null>(null);
  /** The category whose "+" opened "Create channel"; null from the server menu. */
  let newChannelCategory = $state<Channel | null>(null);

  // Notifications (docs/design/web-notifications.md): the reader's choices, the alerts, the tab's
  // count, and the open channel counting as seen while the window has focus.
  $effect(() => loadNotificationPreferences(client.user?.id ?? null));
  onMount(() => startNotifier(client));
  $effect(() => showAttentionCount(attentionCount(client)));
  $effect(() => {
    const channelId = client.selectedChannelId;
    if (channelId !== null && document.visibilityState === "visible" && document.hasFocus()) {
      noteChannelSeen(channelId);
    }
  });
  const notificationDialog = $derived(notificationDialogRequest());

  const guild = $derived(client.selectedGuild);
  const group = $derived(
    client.selectedConversation && isGroupConversation(client.selectedConversation) ? client.selectedConversation : null,
  );
  /** The server or the group whose members the side sheet shows. */
  const memberSheetPlaceId = $derived(guild && client.selectedChannel ? guild.id : (group?.id ?? null));
  const hasMemberSheet = $derived(memberSheetPlaceId !== null);
  const isMemberSheetOpen = $derived(
    memberSheetPlaceId !== null &&
      (hasRoomForSideSheet.current ? placeIdsWithMemberSheet.has(memberSheetPlaceId) : isMemberOverlayOpen),
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

  // Home, a deleted channel or a left server has no members to show: the overlay must not come
  // back by itself when a server is opened again.
  $effect(() => {
    if (!hasMemberSheet) {
      isMemberOverlayOpen = false;
    }
  });

  /** The composer, or the Friends space's tab when Home shows it. */
  function focusComposer() {
    if (client.isFriendsSpaceOpen) {
      friendsPane?.focusStart();
    } else {
      chatPane?.focusComposer();
    }
  }

  function openChannelSettings(channel: Channel) {
    channelSettingsChannelId = channel.id;
    openDialog = "channel-settings";
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
    if (memberSheetPlaceId === null) {
      return;
    }
    if (!hasRoomForSideSheet.current) {
      isMemberOverlayOpen = !isMemberOverlayOpen;
    } else if (placeIdsWithMemberSheet.has(memberSheetPlaceId)) {
      placeIdsWithMemberSheet.delete(memberSheetPlaceId);
    } else {
      placeIdsWithMemberSheet.add(memberSheetPlaceId);
    }
  }

  async function closeMemberSheet() {
    if (hasRoomForSideSheet.current && memberSheetPlaceId !== null) {
      placeIdsWithMemberSheet.delete(memberSheetPlaceId);
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
    if (!isShortcut || !hasMemberSheet || isCoveredByModal()) {
      return;
    }
    event.preventDefault();
    if (isMemberSheetOpen) {
      // Focus may be inside the sheet that goes away: it moves to the composer.
      void closeMemberSheet();
    } else {
      toggleMemberSheet();
    }
  }

  /** Ctrl+K, as on Discord, from anywhere in the shell (the composer included); again closes it. */
  function toggleQuickSwitcherWithShortcut(event: KeyboardEvent) {
    const isShortcut =
      (event.ctrlKey || event.metaKey) && !event.shiftKey && !event.altKey && event.key.toLowerCase() === "k";
    if (!isShortcut) {
      return;
    }
    // Never the browser's own Ctrl+K (its search bar), even while something covers the shell.
    event.preventDefault();
    if (isQuickSwitcherOpen) {
      isQuickSwitcherOpen = false;
    } else if (!isCoveredByModal() && !isMemberOverlayOpen) {
      isQuickSwitcherOpen = true;
    }
  }

  /** After a dialog opened a conversation from the drawer's "+": the drawer goes, the composer focuses. */
  async function focusComposerAfterDialog() {
    isNavigationDrawerOpen = false;
    await tick();
    focusComposer();
  }

  /** "Add friend" from the picker's empty state: Friends opens on its Add friend tab. */
  async function openAddFriend() {
    openDialog = null;
    isNavigationDrawerOpen = false;
    friendsTab = "add";
    client.selectGuild(null, null);
    // After the picker's own focus return, which runs as it closes.
    await tick();
    friendsPane?.focusStart();
  }

  // "Add friends to …" belongs to the open group: when the group goes (removed, last one out), the
  // picker goes too, or it would keep the shell covered with nothing shown and reopen on the next group.
  $effect(() => {
    if (openDialog === "add-to-group" && group === null) {
      openDialog = null;
    }
  });

  // The open group closed under the user (they left, or its owner removed them): Friends shows,
  // the reason is read out, and focus that fell with the conversation goes to Friends' tab.
  $effect(() => {
    const closed = client.closedConversation;
    if (!closed) {
      return;
    }
    const name = conversationDisplayName(closed.conversation, client.user?.id ?? null);
    announcement = closed.reason === "left" ? strings.groups.left(name) : strings.groups.removedYou(name);
    client.closedConversation = null;
    void tick().then(() => {
      if (document.activeElement === null || document.activeElement === document.body) {
        focusComposer();
      }
    });
  });

  /** A dialog, the settings, the navigation drawer or the switcher covers the shell. */
  function isCoveredByModal(): boolean {
    return (
      openDialog !== null ||
      notificationDialog !== null ||
      isUserSettingsOpen ||
      isNavigationDrawerOpen ||
      isQuickSwitcherOpen
    );
  }
</script>

<svelte:window
  onkeydown={(event) => {
    toggleMemberSheetWithShortcut(event);
    toggleQuickSwitcherWithShortcut(event);
  }}
/>

{#snippet navigator(isInDrawer: boolean)}
  <Navigator
    {client}
    {collapsedCategoryIds}
    {isInDrawer}
    onaddserver={() => (openDialog = "add-server")}
    onnewmessage={() => (openDialog = "new-message")}
    onopendialog={(dialog, guildId) => {
      newChannelCategory = null;
      dialogGuildId = guildId;
      openDialog = dialog;
    }}
    oncreatechannelin={(category) => {
      newChannelCategory = category;
      openDialog = "create-channel";
    }}
    oneditchannel={openChannelSettings}
    onnavigate={() => void closeDrawerAfterNavigation()}
    onopenusersettings={() => (isUserSettingsOpen = true)}
  />
{/snippet}

<div class="safe-area-padding flex h-dvh flex-col bg-surface-rail font-sans text-text-normal">
  <ConnectionBanner status={client.status} />
  <NotificationPermissionBar status={client.status} />
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
      <Drawer label={strings.shell.serversAndChannels} side="start" onclose={() => (isNavigationDrawerOpen = false)}>
        {@render navigator(true)}
      </Drawer>
    {/if}

    <main class="flex min-w-0 flex-1">
      {#if client.isFriendsSpaceOpen}
        <FriendsPane
          bind:this={friendsPane}
          bind:tab={friendsTab}
          {client}
          hasNavigationDrawer={!hasRoomForNavigator.current}
          onopennavigation={() => (isNavigationDrawerOpen = true)}
          onaddserver={() => (openDialog = "add-server")}
          onopenedconversation={() => void tick().then(focusComposer)}
        />
      {:else}
        <ChatPane
          bind:this={chatPane}
          {client}
          hasNavigationDrawer={!hasRoomForNavigator.current}
          onopennavigation={() => (isNavigationDrawerOpen = true)}
          onopenquickswitcher={() => (isQuickSwitcherOpen = true)}
          isMemberListOpen={isMemberSheetOpen}
          ontogglememberlist={toggleMemberSheet}
          oneditchannel={openChannelSettings}
          onaddtogroup={() => (openDialog = "add-to-group")}
        />
      {/if}
    </main>

    {#if group && isMemberSheetOpen}
      {#if hasRoomForSideSheet.current}
        {#key group.id}
          <GroupMembersSheet
            {client}
            {group}
            isDocked
            onclose={() => void closeMemberSheet()}
            onaddfriends={() => (openDialog = "add-to-group")}
            focusAfterLeaving={focusComposer}
          />
        {/key}
      {:else}
        <Drawer
          label={strings.shell.membersOf(conversationDisplayName(group, client.user?.id ?? null))}
          side="end"
          isFullWindow={!hasRoomForNavigator.current}
          onclose={() => (isMemberOverlayOpen = false)}
        >
          {#key group.id}
            <GroupMembersSheet
              {client}
              {group}
              isDocked={false}
              isFullWindow={!hasRoomForNavigator.current}
              onclose={() => (isMemberOverlayOpen = false)}
              onaddfriends={() => (openDialog = "add-to-group")}
              focusAfterLeaving={focusComposer}
            />
          {/key}
        </Drawer>
      {/if}
    {:else if guild && hasMemberSheet && isMemberSheetOpen}
      {#if hasRoomForSideSheet.current}
        {#key guild.id}
          <MemberList {client} {guild} isDocked onclose={() => void closeMemberSheet()} onopenedconversation={focusComposer} />
        {/key}
      {:else}
        <Drawer
          label={strings.shell.membersOf(guild.name)}
          side="end"
          isFullWindow={!hasRoomForNavigator.current}
          onclose={() => (isMemberOverlayOpen = false)}
        >
          <!-- Another server starts on its member list, never on the previous server's profile. -->
          {#key guild.id}
            <MemberList
              {client}
              {guild}
              isDocked={false}
              isFullWindow={!hasRoomForNavigator.current}
              onclose={() => (isMemberOverlayOpen = false)}
              onopenedconversation={() => void closeMemberSheet()}
            />
          {/key}
        </Drawer>
      {/if}
    {/if}
  </div>
</div>

<p class="sr-only" aria-live="polite">{announcement}</p>

{#if openDialog === "new-message" || (openDialog === "add-to-group" && group)}
  <PeoplePickerDialog
    {client}
    group={openDialog === "add-to-group" ? group : null}
    onclose={() => (openDialog = null)}
    onaddfriend={() => void openAddFriend()}
    onadded={(names) => (announcement = strings.groups.added(names))}
    fallbackFocus={() => void focusComposerAfterDialog()}
  />
{:else if openDialog === "add-server"}
  <AddServerDialog {client} onclose={() => (openDialog = null)} fallbackFocus={focusComposer} />
{:else if openDialog === "invite-people" && dialogGuild}
  <InvitePeopleDialog
    {client}
    guildId={dialogGuild.id}
    serverName={dialogGuild.name}
    onclose={() => (openDialog = null)}
    fallbackFocus={focusComposer}
  />
{:else if openDialog === "create-channel" || openDialog === "create-category"}
  <CreateChannelDialog
    {client}
    category={newChannelCategory}
    initialKind={openDialog === "create-category" ? "category" : "text"}
    onclose={() => (openDialog = null)}
    fallbackFocus={focusComposer}
  />
{:else if openDialog === "server-settings" && dialogGuildId}
  <ServerSettings
    {client}
    guildId={dialogGuildId}
    isWide={hasRoomForNavigator.current}
    onclose={() => (openDialog = null)}
  />
{:else if openDialog === "channel-settings" && channelSettingsChannelId}
  <ChannelSettings
    {client}
    channelId={channelSettingsChannelId}
    isWide={hasRoomForNavigator.current}
    onclose={() => (openDialog = null)}
  />
{:else if openDialog === "leave-server" && dialogGuild}
  <LeaveServerDialog
    {client}
    guildId={dialogGuild.id}
    serverName={dialogGuild.name}
    onclose={() => (openDialog = null)}
    fallbackFocus={focusComposer}
  />
{/if}

{#if notificationDialog}
  <NotificationSettingsDialog
    target={notificationDialog.target}
    placeName={notificationDialog.placeName}
    initialGroup={notificationDialog.initialGroup}
    onclose={closeNotificationDialog}
    ondone={(sentence) => (announcement = sentence)}
  />
{/if}

{#if isUserSettingsOpen}
  <UserSettings {client} isWide={hasRoomForNavigator.current} onclose={() => (isUserSettingsOpen = false)} />
{/if}

{#if isQuickSwitcherOpen}
  <QuickSwitcher {client} onclose={() => (isQuickSwitcherOpen = false)} onnavigated={focusComposer} />
{/if}
