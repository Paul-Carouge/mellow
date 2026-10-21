<script lang="ts">
  import type { SvelteSet } from "svelte/reactivity";

  import type { MellowClient } from "../lib/client.svelte";
  import { type DirectMessageChannel, type Guild, Permission, type Snowflake, hasPermission } from "../lib/protocol";
  import Avatar from "./Avatar.svelte";
  import Button from "./Button.svelte";
  import { displayName } from "./format";
  import Icon, { type IconName } from "./Icon.svelte";
  import IconButton from "./IconButton.svelte";
  import { channelSections } from "./message-rows";

  export type ServerDialog = "invite-people" | "create-channel" | "leave-server";

  // The channel sidebar (DESIGN.md §5.3, §5.6): server header with its menu, then channels by
  // category, or the conversations at Home; the user panel at the bottom.
  interface Props {
    client: MellowClient;
    /** Kept by the shell, so collapsing survives the drawer closing. */
    collapsedCategoryIds: SvelteSet<Snowflake>;
    onopendialog: (dialog: ServerDialog) => void;
    /** After a channel or conversation was chosen (the narrow layout closes its drawer). */
    onnavigate: () => void;
  }

  let { client, collapsedCategoryIds, onopendialog, onnavigate }: Props = $props();

  const guild = $derived(client.selectedGuild);
  const user = $derived(client.user);
  const canInvitePeople = $derived(guild ? hasPermission(guild.my_permissions, Permission.CREATE_INVITE) : false);
  const canCreateChannels = $derived(guild ? hasPermission(guild.my_permissions, Permission.MANAGE_CHANNELS) : false);
  const canLeaveServer = $derived(guild !== null && guild.owner_id !== user?.id);
  const canSendMessages = $derived(guild ? hasPermission(guild.my_permissions, Permission.SEND_MESSAGES) : true);
  const hasServerMenu = $derived(canInvitePeople || canCreateChannels || canLeaveServer);

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
    return others.map(displayName).join(", ") || "Just you";
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
    "flex w-full items-center gap-(--mw-component-channel-row-icon-gap) rounded-(--mw-component-channel-row-radius) px-(--mw-component-channel-row-padding-inline) text-start focus-visible:focus-ring-inset",
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
    class="absolute top-1/2 left-0 h-(--mw-component-channel-row-unread-pill-height) w-(--mw-component-channel-row-unread-pill-width) -translate-x-1/2 -translate-y-1/2 rounded-full bg-(--mw-component-channel-row-unread-pill-color)"
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

<div class="flex h-full w-(--mw-component-sidebar-width) min-w-0 flex-col bg-(--mw-component-sidebar-background)">
  <header class="relative z-10 h-(--mw-component-sidebar-header-height) shrink-0 shadow-(--mw-component-sidebar-header-shadow)">
    {#if guild && hasServerMenu}
      <button
        bind:this={headerButton}
        type="button"
        aria-haspopup="menu"
        aria-expanded={isMenuOpen}
        aria-controls="server-menu"
        class={[
          "flex size-full items-center gap-xs px-(--mw-component-sidebar-header-padding-inline) text-start text-(--mw-component-sidebar-header-text) hover:bg-(--mw-component-sidebar-header-background-hover) focus-visible:focus-ring-inset",
          isMenuOpen && "bg-(--mw-component-sidebar-header-background-hover)",
        ]}
        onclick={() => (isMenuOpen ? menu?.hidePopover() : openMenu())}
      >
        <h2 class="min-w-0 flex-1 truncate type-heading-md">{guild.name}</h2>
        <Icon name={isMenuOpen ? "x" : "caret-down"} class="size-(--mw-component-sidebar-header-chevron-size)" />
      </button>
    {:else}
      <h2
        class="flex size-full items-center truncate px-(--mw-component-sidebar-header-padding-inline) type-heading-md text-(--mw-component-sidebar-header-text)"
      >
        {guild ? guild.name : "Direct messages"}
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
    aria-label="Server menu"
    class="fixed inset-auto m-0 flex-col rounded-(--mw-component-context-menu-radius) border-(length:--mw-component-context-menu-border-width) border-(--mw-component-context-menu-border) bg-(--mw-component-context-menu-background) p-(--mw-component-context-menu-padding) shadow-(--mw-component-context-menu-shadow) [&:popover-open]:flex"
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
      {@render menuItem("Invite people", "user-plus", "accent", "invite-people")}
    {/if}
    {#if canCreateChannels}
      {@render menuItem("Create channel", "plus-circle", "normal", "create-channel")}
    {/if}
    {#if canLeaveServer}
      {#if canInvitePeople || canCreateChannels}
        <hr
          class="mx-(--mw-component-context-menu-separator-margin-inline) my-(--mw-component-context-menu-separator-margin-block) h-(--mw-component-context-menu-separator-height) border-0 bg-(--mw-component-context-menu-separator)"
        />
      {/if}
      {@render menuItem("Leave server", "sign-out", "danger", "leave-server")}
    {/if}
  </div>

  <nav
    aria-label={guild ? `Channels in ${guild.name}` : "Direct messages"}
    class="min-h-0 flex-1 overflow-y-auto pt-(--mw-component-sidebar-list-padding-top) pb-xs"
  >
    {#if guild}
      {@render guildChannels(guild)}
    {:else if client.conversations.length === 0}
      <p class="px-md py-xs type-body text-text-muted">
        No conversations yet. Open someone's profile in a server's member list and choose Message.
      </p>
    {:else}
      <ul class="flex flex-col gap-(--mw-component-member-row-gap)">
        {#each client.conversations as conversation (conversation.id)}
          {@const isSelected = conversation.id === client.selectedChannelId}
          {@const hasUnread = !isSelected && client.hasUnread(conversation.id)}
          {@const recipient = conversation.recipients.find((candidate) => candidate.id !== user?.id)}
          <li class="relative px-(--mw-component-member-row-margin-inline)">
            {#if hasUnread}{@render unreadPill()}{/if}
            <button
              type="button"
              aria-current={isSelected ? "page" : undefined}
              aria-label={hasUnread ? `${conversationName(conversation)}, unread` : conversationName(conversation)}
              class={[rowClasses(isSelected, hasUnread), "h-(--mw-component-member-row-height) gap-sm"]}
              onclick={() => selectConversation(conversation.id)}
            >
              {#if recipient}
                <Avatar
                  user={recipient}
                  sizeToken="--mw-component-member-row-avatar-size"
                  presence={client.onlineUserIds.has(recipient.id) ? "online" : "offline"}
                  ringColorToken="--mw-component-sidebar-background"
                />
              {/if}
              <span class="min-w-0 flex-1 truncate type-channel-name">
                {conversationName(conversation)}
              </span>
            </button>
          </li>
        {/each}
      </ul>
    {/if}
  </nav>

  {#if user}
    <div
      class="flex h-(--mw-component-user-panel-height) shrink-0 items-center gap-xs bg-(--mw-component-user-panel-background) px-(--mw-component-user-panel-padding-inline)"
    >
      <Avatar
        {user}
        sizeToken="--mw-component-user-panel-avatar-size"
        presence="online"
        ringColorToken="--mw-component-user-panel-background"
      />
      <div class="min-w-0 flex-1">
        <p class="truncate type-user-panel-name text-(--mw-component-user-panel-name-text)">{displayName(user)}</p>
        <p class="truncate type-body-sm text-(--mw-component-user-panel-status-text)">@{user.username}</p>
      </div>
      <IconButton
        icon="sign-out"
        label="Sign out"
        class="size-(--mw-component-user-panel-button-size) text-(--mw-component-user-panel-button-icon) hover:bg-(--mw-component-user-panel-button-background-hover)"
        iconClass="size-(--mw-component-user-panel-button-icon-size)"
        onclick={() => void client.signOut()}
      />
    </div>
  {/if}
</div>

{#snippet guildChannels(guild: Guild)}
  {@const sections = channelSections(guild)}
  {#if sections.every((section) => section.channels.length === 0)}
    <div class="flex flex-col items-start gap-xs px-md py-xs">
      {#if canCreateChannels}
        <p class="type-body text-text-muted">No channels yet. Create the first one to get the conversation going.</p>
        <Button
          variant="primary"
          onclick={() => onopendialog("create-channel")}>Create a channel</Button
        >
      {:else}
        <p class="type-body text-text-muted">No channels you can see yet. Ask a moderator for access.</p>
      {/if}
    </div>
  {/if}
  {#each sections as section (section.category?.id ?? "uncategorised")}
    {@const category = section.category}
    {@const isCollapsed = category !== null && collapsedCategoryIds.has(category.id)}
    {#if category}
      <h3 class="mt-(--mw-component-category-margin-top) px-(--mw-component-category-padding-inline)">
        <button
          type="button"
          aria-expanded={!isCollapsed}
          class="flex h-(--mw-component-category-height) w-full items-center gap-(--mw-space-3xs) rounded-(--mw-component-channel-row-radius) type-category text-(--mw-component-category-text) hover:text-(--mw-component-category-text-hover) focus-visible:focus-ring-inset pointer-coarse:h-(--mw-size-hit-web-min)"
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
      </h3>
    {/if}
    <ul>
      {#each section.channels as channel (channel.id)}
        {@const isSelected = channel.id === client.selectedChannelId}
        {@const hasUnread = !isSelected && client.hasUnread(channel.id)}
        <!-- Collapsed categories still show the selected and unread channels, as on Discord. -->
        {#if !isCollapsed || isSelected || hasUnread}
          <li class="relative mb-(--mw-component-channel-row-gap) px-(--mw-component-channel-row-margin-inline)">
            {#if hasUnread}{@render unreadPill()}{/if}
            <button
              type="button"
              aria-current={isSelected ? "page" : undefined}
              aria-label={[channel.name, !canSendMessages && "read-only", hasUnread && "unread"].filter(Boolean).join(", ")}
              class={[rowClasses(isSelected, hasUnread), "h-(--mw-component-channel-row-height) pointer-coarse:h-(--mw-size-hit-web-min)"]}
              onclick={() => selectChannel(channel.id)}
            >
              <Icon name={canSendMessages ? "hash" : "lock"} class="size-(--mw-component-channel-row-icon-size)" />
              <span class="min-w-0 flex-1 truncate type-channel-name">
                {channel.name}
              </span>
            </button>
          </li>
        {/if}
      {/each}
    </ul>
  {/each}
{/snippet}
