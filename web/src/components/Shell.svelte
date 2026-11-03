<script lang="ts">
  import { onMount, tick } from "svelte";
  import { MediaQuery, SvelteSet } from "svelte/reactivity";

  import { strings } from "../i18n";
  import { MAXIMUM_MESSAGES_LOADED_TO_REACH_A_MESSAGE, type MellowClient } from "../lib/client.svelte";
  import type { Channel, MessageSearchHit, Snowflake } from "../lib/protocol";
  import { browserStorage } from "../lib/reactions";
  import type { ServerDialog } from "./ChannelSidebar.svelte";
  import ChatPane from "./ChatPane.svelte";
  import {
    type ColumnBounds,
    fitColumnWidths,
    largestColumnWidth,
    readColumnWidths,
    type ResizableColumn,
    writeColumnWidths,
  } from "./column-widths";
  import ColumnResizeHandle from "./ColumnResizeHandle.svelte";
  import ConnectionBanner from "./ConnectionBanner.svelte";
  import CreateChannelDialog from "./CreateChannelDialog.svelte";
  import Drawer from "./Drawer.svelte";
  import type { FriendsTab } from "./friends";
  import FriendsPane from "./FriendsPane.svelte";
  import { conversationDisplayName, isGroupConversation } from "./group-conversations";
  import GroupMembersSheet from "./GroupMembersSheet.svelte";
  import IconButton from "./IconButton.svelte";
  import LazyComponent from "./LazyComponent.svelte";
  import LeaveServerDialog from "./LeaveServerDialog.svelte";
  import MemberList from "./MemberList.svelte";
  import { readMemberSheetPlaceIds, writeMemberSheetPlaceIds } from "./member-sheet-places";
  import { MessageSearch } from "./message-search.svelte";
  import MessageSearchField from "./MessageSearchField.svelte";
  import MessageSearchPanel from "./MessageSearchPanel.svelte";
  import Navigator from "./Navigator.svelte";
  import {
    closeNotificationDialog,
    loadNotificationPreferences,
    notificationDialogRequest,
  } from "./notification-preferences.svelte";
  import NotificationPermissionBar from "./NotificationPermissionBar.svelte";
  import { attentionCount, noteChannelSeen, showAttentionCount, startNotifier } from "./notifier.svelte";
  // Not lazy: Ctrl+K is followed by typing at once, and keys typed before a chunk arrived would
  // land in the composer.
  import QuickSwitcher from "./QuickSwitcher.svelte";
  import QuickSwitcherTrigger from "./QuickSwitcherTrigger.svelte";
  import type { ServerSettingsSection } from "./server-settings";
  import { lengthTokenInPixels, minimumWidthQueryForPanes } from "./tokens";
  import { prepareChimeSound } from "./chime-sound";
  import { listenForPushToTalk } from "./push-to-talk-keys";
  import { playVoiceSoundsWhileMounted } from "./voice-sound-playback.svelte";

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
  // Search results are wider than the members: they dock from 1224 px, and the header then holds
  // the search field; below, a magnifier opens them over the sheet with their own field.
  const hasRoomForSearchSheet = new MediaQuery(
    minimumWidthQueryForPanes([...NAVIGATOR_AND_SHEET, "component-search-panel-width", "size-sheet-inset"]),
  );

  let chatPane = $state<{
    focusComposer: () => void;
    revealSearchResult: (messageId: Snowflake) => Promise<void>;
    showNotice: (text: string) => void;
  } | null>(null);
  let friendsPane = $state<{ focusStart: () => void } | null>(null);
  let friendsTab = $state<FriendsTab>("online");
  const collapsedCategoryIds = new SvelteSet<Snowflake>();
  // Closed by default, remembered per server or group on this device (ADR-0022 §1).
  const placeIdsWithMemberSheet = new SvelteSet<Snowflake>(readMemberSheetPlaceIds(browserStorage()));
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
  /** The page Server settings opens at; `null` for the first one the reader may see. */
  let serverSettingsSection = $state<ServerSettingsSection | null>(null);
  /** The channel whose gear opened its settings. */
  let channelSettingsChannelId = $state<Snowflake | null>(null);
  /** The category whose "+" opened "Create channel"; null from the server menu. */
  let newChannelCategory = $state<Channel | null>(null);

  // Notifications (docs/design/web-notifications.md): the reader's choices, the alerts, the tab's
  // count, and the open channel counting as seen while the window has focus.
  $effect(() => loadNotificationPreferences(client.user?.id ?? null));
  onMount(() => startNotifier(client));
  // Voice (docs/design/web-voice-channels.md): its sounds and the soundboard's, push-to-talk, and a
  // fresh offer when the network returns.
  playVoiceSoundsWhileMounted(() => client);
  onMount(() => {
    const noteBackOnline = () => client.voice.noteBackOnline();
    window.addEventListener("online", noteBackOnline);
    const stopPushToTalk = listenForPushToTalk(window, client.voice);
    return () => {
      window.removeEventListener("online", noteBackOnline);
      stopPushToTalk();
    };
  });
  $effect(() => showAttentionCount(attentionCount(client)));
  $effect(() => {
    const channelId = client.selectedChannelId;
    if (channelId !== null && document.visibilityState === "visible" && document.hasFocus()) {
      noteChannelSeen(channelId);
    }
  });
  const notificationDialog = $derived(notificationDialogRequest());

  // Message search (DESIGN.md 5.12): one search per server, or the conversations at Home. Its
  // results take the side sheet's place; the members come back when it closes.
  const messageSearch = new MessageSearch((guildId, query) => client.searchMessages(guildId, query));
  $effect(() => messageSearch.setScope(client.selectedGuildId));
  const isSearchDocked = $derived(messageSearch.isPanelOpen && hasRoomForSearchSheet.current);
  const isSearchOverlayOpen = $derived(messageSearch.isPanelOpen && !hasRoomForSearchSheet.current);
  /** The result being opened, while its page loads. */
  let openingMessageId = $state<Snowflake | null>(null);
  let latestSearchResultRequest = 0;
  /** Why a result could not open, said in the results themselves. */
  let searchNotice = $state("");
  // A new search, or none, says nothing about an older result.
  $effect(() => {
    void messageSearch.query;
    searchNotice = "";
  });

  const guild = $derived(client.selectedGuild);
  const group = $derived(
    client.selectedConversation && isGroupConversation(client.selectedConversation) ? client.selectedConversation : null,
  );
  /** The server or the group whose members the side sheet shows. */
  const memberSheetPlaceId = $derived(guild && client.selectedChannel ? guild.id : (group?.id ?? null));
  const hasMemberSheet = $derived(memberSheetPlaceId !== null);
  const isMemberSheetOpen = $derived(
    memberSheetPlaceId !== null &&
      !messageSearch.isPanelOpen &&
      (hasRoomForSideSheet.current ? placeIdsWithMemberSheet.has(memberSheetPlaceId) : isMemberOverlayOpen),
  );

  // Resizable columns (DESIGN.md 4.5): the reader's widths, remembered on this device, and the
  // widths laid out, fitted to the window so the conversation keeps its minimum. The breakpoints
  // above are where even the minimum widths no longer fit, so they hold whatever the reader chose.
  const COLUMN_BOUNDS: Record<ResizableColumn, ColumnBounds> = {
    channels: {
      minimum: lengthTokenInPixels("size-sidebar-width-min"),
      maximum: lengthTokenInPixels("size-sidebar-width-max"),
      default: lengthTokenInPixels("size-sidebar-width"),
    },
    members: {
      minimum: lengthTokenInPixels("size-member-list-width-min"),
      maximum: lengthTokenInPixels("size-member-list-width-max"),
      default: lengthTokenInPixels("size-member-list-width"),
    },
    search: {
      minimum: lengthTokenInPixels("component-search-panel-width"),
      maximum: lengthTokenInPixels("component-search-panel-width-max"),
      default: lengthTokenInPixels("component-search-panel-width"),
    },
  };
  /** The strip, a conversation at its minimum and the row's four insets (two ends, two gaps). */
  const WIDTH_BESIDE_COLUMNS =
    lengthTokenInPixels("component-navigator-strip-width") +
    lengthTokenInPixels("size-chat-min-width") +
    4 * lengthTokenInPixels("size-sheet-inset");
  const columnWidths = $state(readColumnWidths(browserStorage(), COLUMN_BOUNDS));
  let rowWidth = $state(window.innerWidth);
  const columnRoom = $derived(rowWidth - WIDTH_BESIDE_COLUMNS);
  const dockedSideColumn = $derived<"members" | "search" | null>(
    isSearchDocked ? "search" : isMemberSheetOpen && hasRoomForSideSheet.current ? "members" : null,
  );
  const fittedColumnWidths = $derived(
    fitColumnWidths(
      columnRoom,
      { width: columnWidths.channels, bounds: COLUMN_BOUNDS.channels },
      dockedSideColumn && { width: columnWidths[dockedSideColumn], bounds: COLUMN_BOUNDS[dockedSideColumn] },
    ),
  );
  /** What each handle may reach now: the docked side sheet keeps its minimum beside the channel list. */
  const channelsColumnBounds = $derived({
    ...COLUMN_BOUNDS.channels,
    maximum: largestColumnWidth(
      COLUMN_BOUNDS.channels,
      columnRoom - (dockedSideColumn ? COLUMN_BOUNDS[dockedSideColumn].minimum : 0),
    ),
  });
  const sideColumnBounds = $derived(
    dockedSideColumn && {
      ...COLUMN_BOUNDS[dockedSideColumn],
      maximum: largestColumnWidth(COLUMN_BOUNDS[dockedSideColumn], columnRoom - fittedColumnWidths.channels),
    },
  );

  function rememberColumnWidths() {
    writeColumnWidths(browserStorage(), columnWidths);
  }

  /** Esc during a drag: storage is written only when a gesture ends, so it still holds the widths
   * from before it, the reader's own rather than the fitted ones. */
  function restoreRememberedColumnWidths() {
    Object.assign(columnWidths, readColumnWidths(browserStorage(), COLUMN_BOUNDS));
  }

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
    if (messageSearch.isPanelOpen) {
      // The members take the side sheet back from the results, which stay for the next search.
      messageSearch.isPanelOpen = false;
      if (isMemberSheetOpen) {
        return;
      }
    }
    if (!hasRoomForSideSheet.current) {
      isMemberOverlayOpen = !isMemberOverlayOpen;
    } else if (placeIdsWithMemberSheet.has(memberSheetPlaceId)) {
      forgetMemberSheet(memberSheetPlaceId);
    } else {
      placeIdsWithMemberSheet.add(memberSheetPlaceId);
      writeMemberSheetPlaceIds(browserStorage(), placeIdsWithMemberSheet);
    }
  }

  function forgetMemberSheet(placeId: Snowflake) {
    placeIdsWithMemberSheet.delete(placeId);
    writeMemberSheetPlaceIds(browserStorage(), placeIdsWithMemberSheet);
  }

  async function closeMemberSheet() {
    if (hasRoomForSideSheet.current && memberSheetPlaceId !== null) {
      forgetMemberSheet(memberSheetPlaceId);
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

  /** Ctrl+Shift+M and Ctrl+Shift+D, as on Discord, from anywhere: the microphone and the headset. */
  function toggleVoiceWithShortcut(event: KeyboardEvent) {
    if (!(event.ctrlKey || event.metaKey) || !event.shiftKey || event.altKey) {
      return;
    }
    const key = event.key.toLowerCase();
    if (key !== "m" && key !== "d") {
      return;
    }
    event.preventDefault();
    void prepareChimeSound();
    if (key === "m") {
      void client.voice.toggleMicrophone();
    } else {
      client.voice.toggleDeafened();
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

  /** Ctrl+F (Cmd+F), as on Discord: the search field, or the results sheet with its own field. */
  function focusSearchWithShortcut(event: KeyboardEvent) {
    const isShortcut =
      (event.ctrlKey || event.metaKey) && !event.shiftKey && !event.altKey && event.key.toLowerCase() === "f";
    if (!isShortcut || (isCoveredByModal() && !isSearchOverlayOpen) || isMemberOverlayOpen) {
      return;
    }
    event.preventDefault();
    if (!messageSearch.focusField()) {
      void openSearchSheet();
    }
  }

  async function openSearchSheet() {
    messageSearch.isPanelOpen = true;
    // After the drawer has put focus on itself.
    await tick();
    messageSearch.focusField();
  }

  /** The close button: the search ends and its field empties. */
  async function closeSearch() {
    messageSearch.clear();
    await tick();
    if (!messageSearch.focusField()) {
      focusComposer();
    }
  }

  /** "Go to a channel, server or person" at the end of the search suggestions. */
  function openQuickSwitcherFromSearch() {
    if (!hasRoomForSearchSheet.current) {
      messageSearch.isPanelOpen = false;
    }
    isQuickSwitcherOpen = true;
  }

  /** Opens the result's channel at the message, loading older pages if needed (DESIGN.md 5.12). */
  async function openSearchResult(hit: MessageSearchHit) {
    const message = hit.message;
    const scopeGuildId = messageSearch.scopeGuildId;
    // A later choice wins: an earlier one still loading its pages must not reveal or speak after it.
    const request = ++latestSearchResultRequest;
    const isSuperseded = () => request !== latestSearchResultRequest;
    openingMessageId = message.id;
    searchNotice = "";
    try {
      if (client.selectedChannelId !== message.channel_id) {
        client.selectGuild(scopeGuildId, message.channel_id);
      }
      if (client.selectedChannelId !== message.channel_id) {
        // The conversation left the list, or the channel was hidden since the search: said in the
        // results, which stay open (at Home the Friends space has no chat to say it in).
        searchNotice = scopeGuildId === null ? strings.search.conversationGone : strings.search.jumpMissing;
        return;
      }
      const reach = await client.loadHistoryUntilMessage(message.channel_id, message.id);
      if (reach === "left" || isSuperseded()) {
        return;
      }
      if (!hasRoomForSearchSheet.current) {
        messageSearch.isPanelOpen = false;
      }
      await tick();
      if (isSuperseded()) {
        return;
      }
      switch (reach) {
        case "loaded":
          await chatPane?.revealSearchResult(message.id);
          break;
        case "too-far":
          chatPane?.showNotice(strings.search.jumpTooFar(placeNameOf(message.channel_id), MAXIMUM_MESSAGES_LOADED_TO_REACH_A_MESSAGE));
          break;
        case "missing":
          chatPane?.showNotice(strings.search.jumpMissing);
          break;
        case "failed":
          chatPane?.showNotice(strings.search.jumpFailed);
          break;
      }
    } finally {
      if (openingMessageId === message.id && !isSuperseded()) {
        openingMessageId = null;
      }
    }
  }

  function placeNameOf(channelId: Snowflake): string {
    const channel = client.selectedGuild?.channels.find((candidate) => candidate.id === channelId);
    if (channel) {
      return `#${channel.name}`;
    }
    const conversation = client.conversations.find((candidate) => candidate.id === channelId);
    return conversation ? conversationDisplayName(conversation, client.user?.id ?? null) : strings.search.unknownChannel;
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
      isQuickSwitcherOpen ||
      isSearchOverlayOpen
    );
  }
</script>

<svelte:window
  onkeydown={(event) => {
    toggleMemberSheetWithShortcut(event);
    toggleQuickSwitcherWithShortcut(event);
    focusSearchWithShortcut(event);
    toggleVoiceWithShortcut(event);
  }}
/>

{#snippet searchControl()}
  <!-- Both ways around the header, as the owner asked: the quick switcher ("Go to…", Ctrl K) and
       message search (Ctrl F). -->
  <QuickSwitcherTrigger isBesideSearchField={hasRoomForSearchSheet.current} onopen={() => (isQuickSwitcherOpen = true)} />
  {#if hasRoomForSearchSheet.current}
    <MessageSearchField {client} search={messageSearch} placement="header" onopenquickswitcher={openQuickSwitcherFromSearch} />
  {:else}
    <IconButton
      icon="magnifying-glass"
      label={strings.search.openSearch}
      aria-haspopup="dialog"
      aria-expanded={messageSearch.isPanelOpen}
      aria-keyshortcuts="Control+F"
      class="size-(--mw-component-chat-header-button-size) text-(--mw-component-chat-header-icon) hover:bg-state-hover hover:text-text-normal"
      iconClass="size-(--mw-component-chat-header-icon-size)"
      onclick={() => void openSearchSheet()}
    />
  {/if}
{/snippet}

{#snippet searchPanel(isDocked: boolean)}
  <MessageSearchPanel
    {client}
    search={messageSearch}
    {isDocked}
    isFullWindow={!isDocked && !hasRoomForNavigator.current}
    {openingMessageId}
    notice={searchNotice}
    onopenresult={(hit) => void openSearchResult(hit)}
    onclose={() => void closeSearch()}
    onopenquickswitcher={openQuickSwitcherFromSearch}
  />
{/snippet}

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
      serverSettingsSection = dialog === "server-soundboard" ? "soundboard" : null;
      openDialog = dialog === "server-soundboard" ? "server-settings" : dialog;
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
    bind:clientWidth={rowWidth}
    class={[
      "flex min-h-0 flex-1",
      hasRoomForNavigator.current && [
        "gap-(--mw-size-sheet-inset) ps-(--mw-size-sheet-inset) pt-[calc(2*var(--mw-size-sheet-inset))] pb-(--mw-size-sheet-inset)",
        dockedSideColumn !== null
          ? "pe-(--mw-size-sheet-inset)"
          : "pe-[calc(2*var(--mw-size-sheet-inset))]",
      ],
    ]}
  >
    {#if hasRoomForNavigator.current}
      <!-- The reader's width overrides the token for the docked navigator only, never the drawer's. -->
      <div id="navigator-column" class="relative flex" style:--mw-component-sidebar-width="{fittedColumnWidths.channels}px">
        {@render navigator(false)}
        <ColumnResizeHandle
          label={strings.shell.resizeChannels}
          controls="navigator-column"
          edge="end"
          width={fittedColumnWidths.channels}
          bounds={channelsColumnBounds}
          onresize={(width) => (columnWidths.channels = width)}
          onresizeend={rememberColumnWidths}
          onresizecancel={restoreRememberedColumnWidths}
        />
      </div>
    {:else if isNavigationDrawerOpen}
      <Drawer label={strings.shell.serversAndChannels} side="start" onclose={() => (isNavigationDrawerOpen = false)}>
        {@render navigator(true)}
      </Drawer>
    {/if}

    <main class="relative flex min-w-0 flex-1">
      <!-- A watched screen (ADR-0037) or the call view (ADR-0035) takes the sheet's place; the
           conversation stays underneath, unreachable, so its scroll and draft are there when it closes. -->
      <div class="contents" inert={client.voice.watchedStreamUserId !== null || client.voice.isCallViewOpen}>
      {#if client.isFriendsSpaceOpen}
        <FriendsPane
          bind:this={friendsPane}
          bind:tab={friendsTab}
          {client}
          hasNavigationDrawer={!hasRoomForNavigator.current}
          onopennavigation={() => (isNavigationDrawerOpen = true)}
          onaddserver={() => (openDialog = "add-server")}
          onopenedconversation={() => void tick().then(focusComposer)}
          {searchControl}
        />
      {:else}
        <ChatPane
          bind:this={chatPane}
          {client}
          hasNavigationDrawer={!hasRoomForNavigator.current}
          onopennavigation={() => (isNavigationDrawerOpen = true)}
          {searchControl}
          isMemberListOpen={isMemberSheetOpen}
          ontogglememberlist={toggleMemberSheet}
          oneditchannel={openChannelSettings}
          onaddtogroup={() => (openDialog = "add-to-group")}
        />
      {/if}
      </div>
      {#if client.voice.watchedStreamUserId !== null}
        {@const watchedUserId = client.voice.watchedStreamUserId}
        <LazyComponent load={() => import("./StreamView.svelte")} onclose={() => client.voice.watchStream(null)}>
          {#snippet children({ default: StreamView })}
            {#key watchedUserId}
              <StreamView
                {client}
                userId={watchedUserId}
                hasNavigationDrawer={!hasRoomForNavigator.current}
                onopennavigation={() => (isNavigationDrawerOpen = true)}
              />
            {/key}
          {/snippet}
        </LazyComponent>
      {:else if client.voice.isCallViewOpen}
        <!-- While a screen is watched the call view steps aside (unmounted, so it asks for no picture) and comes back when the screen closes. -->
        <LazyComponent load={() => import("./CallView.svelte")} onclose={() => client.voice.closeCallView()}>
          {#snippet children({ default: CallView })}
            <CallView {client} hasNavigationDrawer={!hasRoomForNavigator.current} onopennavigation={() => (isNavigationDrawerOpen = true)} />
          {/snippet}
        </LazyComponent>
      {/if}
    </main>

    {#if dockedSideColumn && sideColumnBounds && fittedColumnWidths.sideSheet !== null}
      <!-- Docked results or members at the reader's width; the overlays keep the token's. -->
      <div
        id="side-sheet-column"
        class="relative flex"
        style:--mw-component-search-panel-width={dockedSideColumn === "search" ? `${fittedColumnWidths.sideSheet}px` : undefined}
        style:--mw-component-side-sheet-width={dockedSideColumn === "members" ? `${fittedColumnWidths.sideSheet}px` : undefined}
      >
        <!-- Members and results each get their own handle: a drag never carries over to the other. -->
        {#key dockedSideColumn}
          <ColumnResizeHandle
            label={dockedSideColumn === "search" ? strings.shell.resizeSearchResults : strings.shell.resizeMembers}
            controls="side-sheet-column"
            edge="start"
            width={fittedColumnWidths.sideSheet}
            bounds={sideColumnBounds}
            onresize={(width) => (columnWidths[dockedSideColumn] = width)}
            onresizeend={rememberColumnWidths}
            onresizecancel={restoreRememberedColumnWidths}
          />
        {/key}
        {#if isSearchDocked}
          {@render searchPanel(true)}
        {:else if group}
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
        {:else if guild}
          {#key guild.id}
            <MemberList {client} {guild} isDocked onclose={() => void closeMemberSheet()} onopenedconversation={focusComposer} />
          {/key}
        {/if}
      </div>
    {:else if isSearchOverlayOpen}
      <Drawer
        label={strings.search.results}
        side="end"
        isFullWindow={!hasRoomForNavigator.current}
        onclose={() => (messageSearch.isPanelOpen = false)}
      >
        {@render searchPanel(false)}
      </Drawer>
    {:else if group && isMemberSheetOpen}
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
    {:else if guild && hasMemberSheet && isMemberSheetOpen}
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
  </div>
</div>

<p class="sr-only" aria-live="polite">{announcement}</p>

{#if openDialog === "new-message" || (openDialog === "add-to-group" && group)}
  <LazyComponent load={() => import("./PeoplePickerDialog.svelte")} onclose={() => (openDialog = null)}>
    {#snippet children({ default: PeoplePickerDialog })}
      <PeoplePickerDialog
        {client}
        group={openDialog === "add-to-group" ? group : null}
        onclose={() => (openDialog = null)}
        onaddfriend={() => void openAddFriend()}
        onadded={(names) => (announcement = strings.groups.added(names))}
        fallbackFocus={() => void focusComposerAfterDialog()}
      />
    {/snippet}
  </LazyComponent>
{:else if openDialog === "add-server"}
  <LazyComponent load={() => import("./AddServerDialog.svelte")} onclose={() => (openDialog = null)}>
    {#snippet children({ default: AddServerDialog })}
      <AddServerDialog {client} onclose={() => (openDialog = null)} fallbackFocus={focusComposer} />
    {/snippet}
  </LazyComponent>
{:else if openDialog === "invite-people" && dialogGuild}
  <LazyComponent load={() => import("./InvitePeopleDialog.svelte")} onclose={() => (openDialog = null)}>
    {#snippet children({ default: InvitePeopleDialog })}
      <InvitePeopleDialog
        {client}
        guildId={dialogGuild.id}
        serverName={dialogGuild.name}
        onclose={() => (openDialog = null)}
        fallbackFocus={focusComposer}
      />
    {/snippet}
  </LazyComponent>
{:else if openDialog === "create-channel" || openDialog === "create-category"}
  <CreateChannelDialog
    {client}
    category={newChannelCategory}
    initialKind={openDialog === "create-category" ? "category" : "text"}
    onclose={() => (openDialog = null)}
    fallbackFocus={focusComposer}
  />
{:else if openDialog === "server-settings" && dialogGuildId}
  {@const guildId = dialogGuildId}
  <LazyComponent load={() => import("./ServerSettings.svelte")} onclose={() => (openDialog = null)}>
    {#snippet children({ default: ServerSettings })}
      <ServerSettings
        {client}
        {guildId}
        initialSection={serverSettingsSection}
        isWide={hasRoomForNavigator.current}
        onclose={() => (openDialog = null)}
      />
    {/snippet}
  </LazyComponent>
{:else if openDialog === "channel-settings" && channelSettingsChannelId}
  {@const channelId = channelSettingsChannelId}
  <LazyComponent load={() => import("./ChannelSettings.svelte")} onclose={() => (openDialog = null)}>
    {#snippet children({ default: ChannelSettings })}
      <ChannelSettings
        {client}
        {channelId}
        isWide={hasRoomForNavigator.current}
        onclose={() => (openDialog = null)}
      />
    {/snippet}
  </LazyComponent>
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
  <LazyComponent load={() => import("./NotificationSettingsDialog.svelte")} onclose={closeNotificationDialog}>
    {#snippet children({ default: NotificationSettingsDialog })}
      <NotificationSettingsDialog
        target={notificationDialog.target}
        placeName={notificationDialog.placeName}
        initialGroup={notificationDialog.initialGroup}
        onclose={closeNotificationDialog}
        ondone={(sentence) => (announcement = sentence)}
      />
    {/snippet}
  </LazyComponent>
{/if}

{#if isUserSettingsOpen}
  <LazyComponent load={() => import("./UserSettings.svelte")} onclose={() => (isUserSettingsOpen = false)}>
    {#snippet children({ default: UserSettings })}
      <UserSettings {client} isWide={hasRoomForNavigator.current} onclose={() => (isUserSettingsOpen = false)} />
    {/snippet}
  </LazyComponent>
{/if}

{#if isQuickSwitcherOpen}
  <QuickSwitcher {client} onclose={() => (isQuickSwitcherOpen = false)} onnavigated={focusComposer} />
{/if}
