<script lang="ts">
  import { SvelteSet } from "svelte/reactivity";

  import { strings } from "../i18n";
  import type { MellowClient } from "../lib/client.svelte";
  import {
    type Channel,
    type DirectMessageChannel,
    type Guild,
    Permission,
    type Snowflake,
    hasPermission,
    iconUrl,
  } from "../lib/protocol";
  import Avatar from "./Avatar.svelte";
  import Button from "./Button.svelte";
  import { visibleChannelSections } from "./channel-settings";
  import { displayName, nameWithUnreadState } from "./format";
  import Icon, { type IconName } from "./Icon.svelte";
  import IconButton from "./IconButton.svelte";
  import MentionBadge from "./MentionBadge.svelte";
  import { channelSections } from "./message-rows";
  import { canOpenServerSettings } from "./server-settings";

  export type ServerDialog = "invite-people" | "server-settings" | "create-channel" | "leave-server";

  // The channel list of the navigator card (DESIGN.md §5.3, §5.6, ADR-0022 §1): server header
  // with its menu, then channels by category, or the conversations at Home.
  interface Props {
    client: MellowClient;
    /** Kept by the shell, so collapsing survives the drawer closing. */
    collapsedCategoryIds: SvelteSet<Snowflake>;
    onopendialog: (dialog: ServerDialog) => void;
    /** The "+" beside a category: create a channel inside it. */
    oncreatechannelin: (category: Channel) => void;
    /** The gear of a channel row: its settings (Discord's "Edit channel"). */
    oneditchannel: (channel: Channel) => void;
    /** After a channel or conversation was chosen (the narrow layout closes its drawer). */
    onnavigate: () => void;
  }

  let { client, collapsedCategoryIds, onopendialog, oncreatechannelin, oneditchannel, onnavigate }: Props = $props();

  const guild = $derived(client.selectedGuild);
  const user = $derived(client.user);
  const canInvitePeople = $derived(guild ? hasPermission(guild.my_permissions, Permission.CREATE_INVITE) : false);
  const canCreateChannels = $derived(guild ? hasPermission(guild.my_permissions, Permission.MANAGE_CHANNELS) : false);
  const canLeaveServer = $derived(guild !== null && guild.owner_id !== user?.id);
  const canOpenSettings = $derived(guild !== null && canOpenServerSettings(guild, user?.id));
  // Channel overwrites included: a read-only channel shows a lock (DESIGN.md §5.3).
  function canSendIn(channelId: Snowflake): boolean {
    const permissions = client.myChannelPermissions(channelId);
    return permissions === null || hasPermission(permissions, Permission.SEND_MESSAGES);
  }
  function canEditChannel(channel: Channel): boolean {
    return visibleChannelSections(channel, client.myChannelPermissions(channel.id)).length > 0;
  }
  /** Icons that failed to load show the # glyph instead. */
  const failedIconIds = new SvelteSet<Snowflake>();
  const hasServerMenu = $derived(canInvitePeople || canOpenSettings || canCreateChannels || canLeaveServer);

  let headerButton = $state<HTMLButtonElement | null>(null);
  let menu = $state<HTMLElement | null>(null);
  let isMenuOpen = $state(false);
  let menuPosition = $state({ top: 0, left: 0, width: 0 });

  function openMenu() {
    if (!headerButton || !menu) {
      return;
    }
    const bounds = headerButton.getBoundingClientRect();
    menuPosition = { top: bounds.bottom, left: bounds.left, width: bounds.width };
    menu.showPopover();
    menuItems()[0]?.focus();
  }

  function menuItems(): HTMLElement[] {
    return [...(menu?.querySelectorAll<HTMLElement>("[role=menuitem]") ?? [])];
  }

  function moveMenuFocus(event: KeyboardEvent) {
    const items = menuItems();
    const current = items.indexOf(document.activeElement as HTMLElement);
    const next =
      event.key === "ArrowDown"
        ? (current + 1) % items.length
        : event.key === "ArrowUp"
          ? (current - 1 + items.length) % items.length
          : event.key === "Home"
            ? 0
            : event.key === "End"
              ? items.length - 1
              : null;
    if (next !== null) {
      event.preventDefault();
      items[next]?.focus();
    }
  }

  function chooseMenuItem(dialog: ServerDialog) {
    menu?.hidePopover();
    onopendialog(dialog);
  }

  function conversationName(conversation: DirectMessageChannel): string {
    const others = conversation.recipients.filter((recipient) => recipient.id !== user?.id);
    return others.map(displayName).join(", ") || strings.sidebar.justYou;
  }

  function selectChannel(channelId: Snowflake) {
    client.selectChannel(channelId);
    onnavigate();
  }

  function selectConversation(channelId: Snowflake) {
    client.selectGuild(null, channelId);
    onnavigate();
  }

  function toggleCategory(categoryId: Snowflake) {
    if (collapsedCategoryIds.has(categoryId)) {
      collapsedCategoryIds.delete(categoryId);
    } else {
      collapsedCategoryIds.add(categoryId);
    }
  }

  const rowClasses = (isSelected: boolean, hasUnread: boolean) => [
    "flex w-full items-center gap-(--mw-component-channel-row-icon-gap) rounded-(--mw-component-channel-row-radius) px-(--mw-component-channel-row-padding-inline) text-start transition-colors duration-(--mw-motion-duration-fast) ease-(--mw-motion-easing-standard) focus-visible:focus-ring-inset",
    isSelected
      ? "bg-(--mw-component-channel-row-background-selected) text-(--mw-component-channel-row-text-selected)"
      : [
          hasUnread ? "text-(--mw-component-channel-row-text-unread)" : "text-(--mw-component-channel-row-text-read)",
          "hover:bg-(--mw-component-channel-row-background-hover) hover:text-(--mw-component-channel-row-text-hover)",
        ],
  ];
</script>

{#snippet unreadPill()}
  <span
    aria-hidden="true"
    class="absolute top-1/2 -left-(--mw-component-channel-row-unread-pill-offset) h-(--mw-component-channel-row-unread-pill-height) w-(--mw-component-channel-row-unread-pill-width) -translate-y-1/2 rounded-full bg-(--mw-component-channel-row-unread-pill-color)"
  ></span>
{/snippet}

{#snippet menuItem(label: string, icon: IconName, tone: "accent" | "normal" | "danger", dialog: ServerDialog)}
  <button
    type="button"
    role="menuitem"
    class={[
      "flex h-(--mw-component-context-menu-item-height) w-full items-center justify-between gap-xs rounded-(--mw-component-context-menu-item-radius) px-(--mw-component-context-menu-item-padding-inline) type-menu-item pointer-coarse:h-(--mw-size-hit-web-min)",
      tone === "danger"
        ? "text-(--mw-component-context-menu-danger-text) hover:bg-(--mw-component-context-menu-danger-background-focus) hover:text-(--mw-component-context-menu-danger-text-focus)"
        : [
            tone === "accent"
              ? "text-(--mw-component-context-menu-accent-text)"
              : "text-(--mw-component-context-menu-item-text)",
            "hover:bg-(--mw-component-context-menu-item-background-focus) hover:text-(--mw-component-context-menu-item-text-focus)",
          ],
    ]}
    onclick={() => chooseMenuItem(dialog)}
  >
    {label}
    <Icon name={icon} class="size-(--mw-component-context-menu-item-icon-size)" />
  </button>
{/snippet}

<div class="flex min-h-0 min-w-0 flex-col">
  <header class="mt-xs me-xs h-(--mw-component-sidebar-header-height) shrink-0">
    {#if guild && hasServerMenu}
      <button
        bind:this={headerButton}
        type="button"
        aria-haspopup="menu"
        aria-expanded={isMenuOpen}
        aria-controls="server-menu"
        class={[
          "flex size-full items-center gap-xs rounded-sm px-xs text-start text-(--mw-component-sidebar-header-text) transition-colors duration-(--mw-motion-duration-fast) ease-(--mw-motion-easing-standard) hover:bg-(--mw-component-sidebar-header-background-hover)",
          isMenuOpen && "bg-(--mw-component-sidebar-header-background-hover)",
        ]}
        onclick={() => (isMenuOpen ? menu?.hidePopover() : openMenu())}
      >
        <h2 class="min-w-0 flex-1 truncate type-heading-md">{guild.name}</h2>
        <Icon
          name={isMenuOpen ? "x" : "caret-down"}
          class="size-(--mw-component-sidebar-header-chevron-size) text-text-secondary"
        />
      </button>
    {:else}
      <h2
        class="flex size-full items-center truncate px-xs type-heading-md text-(--mw-component-sidebar-header-text)"
      >
        {guild ? guild.name : strings.sidebar.directMessages}
      </h2>
    {/if}
  </header>

  <!-- Light dismiss and Esc come from the popover; focus goes back to the header when it closes. -->
  <div
    bind:this={menu}
    id="server-menu"
    popover="auto"
    role="menu"
    tabindex="-1"
    aria-label={strings.sidebar.serverMenu}
    class="fixed inset-auto m-0 flex-col rounded-(--mw-component-context-menu-radius) border-(length:--mw-component-context-menu-border-width) border-(--mw-component-context-menu-border) bg-(--mw-component-context-menu-background) p-(--mw-component-context-menu-padding) shadow-(--mw-component-context-menu-shadow) [&:popover-open]:flex [&:popover-open]:animate-[mw-drop-in_var(--mw-motion-duration-fast)_var(--mw-motion-easing-enter)_both]"
    style:top="calc({menuPosition.top}px + var(--mw-component-sidebar-header-menu-margin))"
    style:left="calc({menuPosition.left}px + var(--mw-component-sidebar-header-menu-margin))"
    style:width="calc({menuPosition.width}px - 2 * var(--mw-component-sidebar-header-menu-margin))"
    onkeydown={moveMenuFocus}
    ontoggle={(event) => {
      isMenuOpen = event.newState === "open";
      if (!isMenuOpen && (menu?.contains(document.activeElement) || document.activeElement === document.body)) {
        headerButton?.focus();
      }
    }}
  >
    {#if canInvitePeople}
      {@render menuItem(strings.sidebar.invitePeople, "user-plus", "accent", "invite-people")}
    {/if}
    {#if canOpenSettings}
      {@render menuItem(strings.sidebar.serverSettings, "gear-six", "normal", "server-settings")}
    {/if}
    {#if canCreateChannels}
      {@render menuItem(strings.sidebar.createChannel, "plus-circle", "normal", "create-channel")}
    {/if}
    {#if canLeaveServer}
      {#if canInvitePeople || canOpenSettings || canCreateChannels}
        <hr
          class="mx-(--mw-component-context-menu-separator-margin-inline) my-(--mw-component-context-menu-separator-margin-block) h-(--mw-component-context-menu-separator-height) border-0 bg-(--mw-component-context-menu-separator)"
        />
      {/if}
      {@render menuItem(strings.sidebar.leaveServer, "sign-out", "danger", "leave-server")}
    {/if}
  </div>

  <nav
    aria-label={guild ? strings.sidebar.channelsIn(guild.name) : strings.sidebar.directMessages}
    class="-ms-(--mw-component-channel-row-unread-pill-offset) min-h-0 flex-1 overflow-y-auto ps-(--mw-component-channel-row-unread-pill-offset) pb-xs [scrollbar-width:thin]"
  >
    <!-- The negative margin lets the unread pills sit in the strip's margin without being clipped. -->
    {#if guild}
      {@render guildChannels(guild)}
    {:else if client.conversations.length === 0}
      <p class="mt-xs me-(--mw-component-channel-row-margin-inline) px-(--mw-component-channel-row-padding-inline) type-body text-text-muted">
        {strings.sidebar.noConversations}
      </p>
    {:else}
      <ul class="mt-xs">
        {#each client.conversations as conversation (conversation.id)}
          {@const isSelected = conversation.id === client.selectedChannelId}
          {@const hasUnread = !isSelected && client.hasUnread(conversation.id)}
          {@const mentionCount = isSelected ? 0 : client.mentionCount(conversation.id)}
          {@const recipient = conversation.recipients.find((candidate) => candidate.id !== user?.id)}
          <li class="relative mt-(--mw-component-member-row-gap) me-(--mw-component-channel-row-margin-inline)">
            {#if hasUnread}{@render unreadPill()}{/if}
            <button
              type="button"
              aria-current={isSelected ? "page" : undefined}
              aria-label={nameWithUnreadState(conversationName(conversation), hasUnread, mentionCount)}
              class={[rowClasses(isSelected, hasUnread), "h-(--mw-component-member-row-height) gap-sm"]}
              onclick={() => selectConversation(conversation.id)}
            >
              {#if recipient}
                <Avatar
                  user={recipient}
                  sizeToken="--mw-component-member-row-avatar-size"
                  presence={client.onlineUserIds.has(recipient.id) ? "online" : "offline"}
                  ringColorToken="--mw-component-navigator-background"
                />
              {/if}
              <span class="min-w-0 flex-1 truncate type-channel-name">
                {conversationName(conversation)}
              </span>
              {#if mentionCount > 0}<MentionBadge {mentionCount} />{/if}
            </button>
          </li>
        {/each}
      </ul>
    {/if}
  </nav>
</div>

{#snippet guildChannels(guild: Guild)}
  {@const sections = channelSections(guild)}
  {#if sections.every((section) => section.channels.length === 0)}
    <div class="mt-xs me-(--mw-component-channel-row-margin-inline) flex flex-col items-start gap-xs px-(--mw-component-channel-row-padding-inline)">
      {#if canCreateChannels}
        <p class="type-body text-text-muted">{strings.sidebar.noChannelsYet}</p>
        <Button
          variant="primary"
          onclick={() => onopendialog("create-channel")}>{strings.sidebar.createAChannel}</Button
        >
      {:else}
        <p class="type-body text-text-muted">{strings.sidebar.noVisibleChannelsYet}</p>
      {/if}
    </div>
  {/if}
  {#each sections as section (section.category?.id ?? "uncategorised")}
    {@const category = section.category}
    {@const isCollapsed = category !== null && collapsedCategoryIds.has(category.id)}
    {#if category}
      <h3 class="mt-(--mw-component-category-margin-top) flex items-center gap-2xs pe-xs">
        <button
          type="button"
          aria-expanded={!isCollapsed}
          class="flex h-(--mw-component-category-height) min-w-0 flex-1 items-center gap-2xs rounded-xs type-category text-(--mw-component-category-text) transition-colors duration-(--mw-motion-duration-fast) ease-(--mw-motion-easing-standard) hover:text-(--mw-component-category-text-hover) focus-visible:focus-ring-inset pointer-coarse:h-(--mw-size-hit-web-min)"
          onclick={() => toggleCategory(category.id)}
          onkeydown={(event) => {
            if ((event.key === "ArrowLeft" && !isCollapsed) || (event.key === "ArrowRight" && isCollapsed)) {
              event.preventDefault();
              toggleCategory(category.id);
            }
          }}
        >
          <Icon name={isCollapsed ? "caret-right" : "caret-down"} class="size-(--mw-component-category-chevron-size)" />
          <span class="truncate">{category.name}</span>
        </button>
        {#if canCreateChannels}
          <IconButton
            icon="plus"
            label={strings.sidebar.createChannelIn(category.name)}
            class="size-(--mw-component-category-height) rounded-xs text-(--mw-component-category-text) hover:text-(--mw-component-category-text-hover)"
            iconClass="size-(--mw-component-category-add-icon-size)"
            onclick={() => oncreatechannelin(category)}
          />
        {/if}
      </h3>
    {/if}
    <ul>
      {#each section.channels as channel (channel.id)}
        {@const isSelected = channel.id === client.selectedChannelId}
        {@const hasUnread = !isSelected && client.hasUnread(channel.id)}
        {@const mentionCount = isSelected ? 0 : client.mentionCount(channel.id)}
        {@const canSendMessages = canSendIn(channel.id)}
        {@const canEdit = canEditChannel(channel)}
        {@const icon = channel.icon && !failedIconIds.has(channel.icon.id) ? channel.icon : null}
        <!-- Collapsed categories still show the selected and unread channels, as on Discord. -->
        {#if !isCollapsed || isSelected || hasUnread}
          <li class="group/row relative mt-(--mw-component-channel-row-gap) me-(--mw-component-channel-row-margin-inline)">
            {#if hasUnread}{@render unreadPill()}{/if}
            <button
              type="button"
              aria-current={isSelected ? "page" : undefined}
              aria-label={nameWithUnreadState(
                [channel.name, !canSendMessages && strings.common.readOnly].filter(Boolean).join(", "),
                hasUnread,
                mentionCount,
              )}
              class={[
                rowClasses(isSelected, hasUnread),
                "h-(--mw-component-channel-row-height) pointer-coarse:h-(--mw-size-hit-web-min)",
                // Room for the gear, so a long name never runs under it; a badge already keeps it.
                canEdit &&
                  mentionCount === 0 &&
                  "pe-[calc(var(--mw-component-channel-row-settings-button-size)+var(--mw-component-channel-row-padding-inline))] pointer-coarse:pe-(--mw-size-hit-web-min)",
              ]}
              onclick={() => selectChannel(channel.id)}
            >
              {#if icon}
                {@const iconId = icon.id}
                <!-- Channel icons are always still (DESIGN.md §6.54): a rounded square, since circles are people. -->
                <img
                  src={iconUrl(icon, false)}
                  alt=""
                  draggable="false"
                  class="size-(--mw-component-channel-row-icon-size) shrink-0 rounded-(--mw-component-channel-row-image-radius) object-cover"
                  onerror={() => failedIconIds.add(iconId)}
                />
              {:else}
                <Icon name={canSendMessages ? "hash" : "lock"} class="size-(--mw-component-channel-row-icon-size)" />
              {/if}
              <span class="min-w-0 flex-1 truncate type-channel-name">
                {channel.name}
              </span>
              {#if icon && !canSendMessages}
                <!-- The icon took the lock's place: it moves after the name. -->
                <Icon name="lock" class="size-(--mw-component-channel-row-trailing-icon-size) shrink-0" />
              {/if}
              {#if mentionCount > 0}
                <!-- The gear takes the badge's place while it shows. -->
                <MentionBadge
                  {mentionCount}
                  class={canEdit ? "group-focus-within/row:invisible group-hover/row:invisible" : ""}
                />
              {/if}
            </button>
            {#if canEdit}
              <!-- Shown on hover, on focus within the row and on the selected row (touch has no hover). -->
              <IconButton
                icon="gear-six"
                label={strings.sidebar.editChannel(channel.name)}
                aria-haspopup="dialog"
                class={[
                  "absolute inset-y-0 end-2xs my-auto size-(--mw-component-channel-row-settings-button-size) text-(--mw-component-channel-row-settings-icon) hover:text-(--mw-component-channel-row-settings-icon-hover) pointer-coarse:end-0",
                  isSelected ? "visible" : "invisible group-focus-within/row:visible group-hover/row:visible",
                ].join(" ")}
                iconClass="size-(--mw-component-channel-row-settings-icon-size)"
                onclick={() => oneditchannel(channel)}
              />
            {/if}
          </li>
        {/if}
      {/each}
    </ul>
  {/each}
{/snippet}
