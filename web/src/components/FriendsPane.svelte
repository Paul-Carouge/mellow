<script lang="ts">
  import { tick } from "svelte";

  import { strings } from "../i18n";
  import { type MellowClient, describeError } from "../lib/client.svelte";
  import type { PublicUser, Relationship } from "../lib/protocol";
  import AddFriendForm from "./AddFriendForm.svelte";
  import Avatar from "./Avatar.svelte";
  import Button from "./Button.svelte";
  import { displayName } from "./format";
  import { type FriendsGroup, type FriendsTab, LIST_TABS, friendsGroupsIn } from "./friends";
  import Icon, { type IconName } from "./Icon.svelte";
  import IconButton from "./IconButton.svelte";
  import Melli from "./Melli.svelte";
  import type { MelliState } from "./melli";
  import MentionBadge from "./MentionBadge.svelte";
  import { menuItemsIn, moveFocusWithinMenu } from "./menu-keyboard";
  import RelationshipConfirmDialog from "./RelationshipConfirmDialog.svelte";

  // The Friends space (DESIGN.md §5.6.1, ADR-0025): Home's page when no conversation is open, in
  // the sheet. Tabs Online, All, Pending, Blocked and Add friend, as on Discord; each row has its
  // actions as round buttons, and Melli keeps the empty tabs company.
  interface Props {
    client: MellowClient;
    /** Kept by the shell, so the tab survives a visit to a conversation. */
    tab: FriendsTab;
    hasNavigationDrawer: boolean;
    onopennavigation: () => void;
    onaddserver: () => void;
    /** After Message opened the conversation: the composer takes focus. */
    onopenedconversation: () => void;
  }

  let { client, tab = $bindable(), hasNavigationDrawer, onopennavigation, onaddserver, onopenedconversation }: Props =
    $props();

  const id = $props.id();
  const TAB_LABELS: Record<FriendsTab, string> = strings.friends.tabs;
  const GROUP_LABELS: Record<FriendsGroup["kind"], string> = strings.friends.groups;

  const incomingCount = $derived(client.incomingRequestCount());
  const hasFriends = $derived(client.relationshipsOfKind("friend").length > 0);
  const groups = $derived(tab === "add" ? [] : friendsGroupsIn(client, tab));

  let panel = $state<HTMLElement | null>(null);
  let addFriendForm = $state<{ focusField: () => void } | null>(null);
  let announcement = $state("");
  let actionError = $state("");
  /** The person whose row is waiting for the instance: its actions dim. */
  let busyUserId = $state<string | null>(null);

  let menu = $state<HTMLElement | null>(null);
  let menuRelationship = $state<Relationship | null>(null);
  let menuButton: HTMLElement | null = null;
  let menuPosition = $state({ top: 0, right: 0 });
  /** Whose menu was open when a More button was pressed: that press closed it already. */
  let menuUserIdAtPress: string | null = null;
  let confirming = $state<{ action: "remove" | "block"; user: PublicUser } | null>(null);

  /** The selected tab, for the shell after the drawer closes. */
  export function focusStart(): void {
    if (tab === "add") {
      addFriendForm?.focusField();
    } else {
      document.getElementById(`${id}-tab-${tab}`)?.focus();
    }
  }

  async function chooseTab(next: FriendsTab) {
    tab = next;
    actionError = "";
    if (next === "add") {
      await tick();
      addFriendForm?.focusField();
    }
  }

  function moveTabFocus(event: KeyboardEvent) {
    const tabs = [...(event.currentTarget as HTMLElement).querySelectorAll<HTMLButtonElement>("[role=tab]")];
    const current = tabs.indexOf(document.activeElement as HTMLButtonElement);
    const next =
      event.key === "ArrowRight"
        ? (current + 1) % tabs.length
        : event.key === "ArrowLeft"
          ? (current - 1 + tabs.length) % tabs.length
          : event.key === "Home"
            ? 0
            : event.key === "End"
              ? tabs.length - 1
              : null;
    if (next !== null) {
      event.preventDefault();
      tabs[next]?.focus();
      tabs[next]?.click();
    }
  }

  /** Runs one row action; the row may leave the list, so focus moves to its neighbour. */
  async function act(
    relationship: Relationship,
    action: () => Promise<void>,
    done: (person: string) => string,
    failed: (person: string) => string,
  ) {
    const person = displayName(relationship.user);
    const rows = [...(panel?.querySelectorAll<HTMLElement>("[data-friend-row]") ?? [])];
    const index = rows.findIndex((row) => row.dataset.friendRow === relationship.user.id);
    busyUserId = relationship.user.id;
    actionError = "";
    try {
      await action();
      announcement = done(person);
      await focusRowAfterChange(index);
    } catch (error) {
      actionError = describeError(error, failed(person));
    } finally {
      busyUserId = null;
    }
  }

  async function focusRowAfterChange(index: number) {
    await tick();
    const rows = [...(panel?.querySelectorAll<HTMLElement>("[data-friend-row]") ?? [])];
    const row = rows[Math.min(index, rows.length - 1)];
    const target = row?.querySelector<HTMLElement>("button") ?? document.getElementById(`${id}-tab-${tab}`);
    target?.focus();
  }

  async function message(user: PublicUser) {
    busyUserId = user.id;
    actionError = "";
    try {
      await client.openDirectMessage(user.id);
      onopenedconversation();
    } catch (error) {
      actionError = describeError(error, strings.members.openConversationFailed);
    } finally {
      busyUserId = null;
    }
  }

  function openMenu(relationship: Relationship, button: HTMLElement) {
    if (!menu) {
      return;
    }
    if (menuUserIdAtPress === relationship.user.id) {
      // The press already closed this row's menu: the click must not open it again.
      menuUserIdAtPress = null;
      return;
    }
    menuUserIdAtPress = null;
    const bounds = button.getBoundingClientRect();
    menuPosition = { top: bounds.bottom, right: window.innerWidth - bounds.right };
    menuRelationship = relationship;
    menuButton = button;
    menu.showPopover();
    menuItemsIn(menu)[0]?.focus();
  }

  function chooseMenuAction(action: "remove" | "block") {
    const relationship = menuRelationship;
    menu?.hidePopover();
    if (relationship) {
      confirming = { action, user: relationship.user };
    }
  }

  function detailOf(relationship: Relationship): string {
    switch (relationship.kind) {
      case "friend":
        return client.customStatusOf(relationship.user.id) ?? strings.presence[client.presenceOf(relationship.user.id)];
      case "incoming_request":
        return strings.friends.incomingRequest;
      case "outgoing_request":
        return strings.friends.outgoingRequest;
      case "blocked":
        return strings.friends.blockedDetail;
    }
  }

  const tabClass = (isSelected: boolean, isAdd: boolean) => [
    "flex h-(--mw-component-friends-tab-height) shrink-0 items-center gap-2xs rounded-(--mw-component-friends-tab-radius) px-(--mw-component-friends-tab-padding-inline) type-button whitespace-nowrap transition-colors duration-(--mw-motion-duration-fast) ease-(--mw-motion-easing-standard) pointer-coarse:h-(--mw-size-hit-web-min)",
    isAdd
      ? isSelected
        ? "text-(--mw-component-friends-add-tab-text-selected)"
        : "bg-(--mw-component-friends-add-tab-background) text-(--mw-component-friends-add-tab-text) hover:bg-(--mw-component-friends-add-tab-background-hover)"
      : isSelected
        ? "bg-(--mw-component-friends-tab-background-selected) text-(--mw-component-friends-tab-text-selected)"
        : "text-(--mw-component-friends-tab-text) hover:bg-(--mw-component-friends-tab-background-hover) hover:text-(--mw-component-friends-tab-text-hover)",
  ];

  const actionClass = (tone: "neutral" | "positive" | "danger") =>
    [
      // `!`: IconButton's own small radius would otherwise win; these are Discord's round buttons.
      "size-(--mw-component-friends-action-size) rounded-full! bg-(--mw-component-friends-action-background) text-(--mw-component-friends-action-icon)",
      tone === "positive"
        ? "hover:text-(--mw-component-friends-action-icon-positive-hover)"
        : tone === "danger"
          ? "hover:text-(--mw-component-friends-action-icon-danger-hover)"
          : "hover:text-(--mw-component-friends-action-icon-hover)",
    ].join(" ");
  const actionIconClass = "size-(--mw-component-friends-action-icon-size)";
  const headerButtonClass =
    "size-(--mw-component-chat-header-button-size) text-(--mw-component-chat-header-icon) hover:bg-state-hover hover:text-text-normal";
</script>

{#snippet tabList()}
  <!-- svelte-ignore a11y_interactive_supports_focus -->
  <div
    role="tablist"
    aria-label={strings.friends.title}
    class={[
      "flex items-center gap-(--mw-component-friends-tab-gap)",
      // Phones wrap the tabs rather than hide one off the edge (long French labels, large text).
      hasNavigationDrawer && "flex-wrap",
    ]}
    onkeydown={moveTabFocus}
  >
    {#each [...LIST_TABS, "add" as const] as candidate (candidate)}
      {@const isSelected = candidate === tab}
      <button
        type="button"
        role="tab"
        id="{id}-tab-{candidate}"
        aria-selected={isSelected}
        aria-controls="{id}-panel"
        aria-label={candidate === "pending" && incomingCount > 0 ? strings.friends.pendingWithCount(incomingCount) : undefined}
        tabindex={isSelected ? 0 : -1}
        class={tabClass(isSelected, candidate === "add")}
        onclick={() => void chooseTab(candidate)}
      >
        {TAB_LABELS[candidate]}
        {#if candidate === "pending" && incomingCount > 0}
          <MentionBadge mentionCount={incomingCount} />
        {/if}
      </button>
    {/each}
  </div>
{/snippet}

{#snippet action(icon: IconName, label: string, tone: "neutral" | "positive" | "danger", onclick: (event: MouseEvent) => void, isBusy: boolean, extra: Record<string, unknown> = {})}
  <IconButton
    {icon}
    {label}
    aria-disabled={isBusy || undefined}
    class="{actionClass(tone)} {isBusy ? 'pointer-events-none opacity-(--mw-component-button-disabled-opacity)' : ''}"
    iconClass={actionIconClass}
    onclick={(event) => {
      if (!isBusy) {
        onclick(event);
      }
    }}
    {...extra}
  />
{/snippet}

{#snippet emptyState(melli: MelliState, sentence: string, withAddFriend: boolean)}
  <div class="flex flex-1 flex-col items-center justify-center gap-(--mw-component-friends-empty-gap) p-xl text-center">
    <Melli state={melli} sizeToken="--mw-component-friends-empty-melli-size" />
    <p class="max-w-(--mw-component-auth-card-max-width) type-body-lg text-text-secondary">{sentence}</p>
    {#if withAddFriend}
      <div class="flex flex-wrap justify-center gap-xs">
        <Button variant="primary" onclick={() => void chooseTab("add")}>{strings.friends.addFriend}</Button>
        {#if client.guilds.length === 0}
          <Button onclick={onaddserver}>{strings.friends.addServer}</Button>
        {/if}
      </div>
    {/if}
  </div>
{/snippet}

<section
  aria-labelledby="{id}-title"
  class={[
    "relative flex h-full min-w-0 flex-1 flex-col overflow-hidden bg-surface-chat",
    !hasNavigationDrawer && "rounded-(--mw-component-sheet-radius)",
  ]}
>
  <header
    class="flex h-(--mw-component-chat-header-height) shrink-0 items-center gap-xs ps-(--mw-component-chat-header-padding-inline) pe-xs"
  >
    {#if hasNavigationDrawer}
      <IconButton
        icon="list"
        label={strings.chat.openNavigation}
        class={headerButtonClass}
        iconClass="size-(--mw-component-chat-header-icon-size)"
        onclick={onopennavigation}
      />
    {/if}
    <div class="flex shrink-0 items-center gap-2xs text-(--mw-component-chat-header-name-text)">
      <Icon name="users" class="size-(--mw-component-chat-header-icon-size) text-text-muted" />
      <h1 id="{id}-title" class="type-heading-md">{strings.friends.title}</h1>
    </div>
    {#if !hasNavigationDrawer}
      <span aria-hidden="true" class="mx-xs h-(--mw-component-chat-header-divider-height) w-px shrink-0 bg-border-divider"></span>
      <!-- Scrolls sideways only under large text; the padding keeps the focus ring unclipped. -->
      <div class="min-w-0 flex-1 overflow-x-auto p-2xs [scrollbar-width:none]">
        {@render tabList()}
      </div>
    {/if}
  </header>
  {#if hasNavigationDrawer}
    <!-- Phones: the tabs get their own rows under the title. -->
    <div class="shrink-0 px-(--mw-component-friends-list-padding-inline) pb-xs">
      {@render tabList()}
    </div>
  {/if}

  <div role="alert" class="contents">
    {#if actionError}
      <p class="flex gap-2xs px-(--mw-component-friends-list-padding-inline) py-xs type-body text-(--mw-component-friends-result-error-text)">
        <Icon name="warning-circle" class="mt-px size-(--mw-component-friends-result-icon-size) shrink-0" />
        <span>{actionError}</span>
      </p>
    {/if}
  </div>

  {#key tab}
    <div
      bind:this={panel}
      role="tabpanel"
      id="{id}-panel"
      aria-labelledby="{id}-tab-{tab}"
      class="tab-enter flex min-h-0 flex-1 flex-col overflow-y-auto px-(--mw-component-friends-list-padding-inline) pb-md [scrollbar-width:thin]"
    >
      {#if tab === "add"}
        <div class="pt-md">
          <AddFriendForm bind:this={addFriendForm} {client} />
        </div>
      {:else if groups.length === 0}
        {#if (tab === "online" || tab === "all") && !hasFriends}
          {@render emptyState("idle", strings.friends.noFriendsYet, true)}
        {:else if tab === "online"}
          {@render emptyState("sleeping", strings.friends.emptyOnline, false)}
        {:else if tab === "pending"}
          {@render emptyState("idle", strings.friends.emptyPending, false)}
        {:else}
          {@render emptyState("idle", strings.friends.emptyBlocked, false)}
        {/if}
      {:else}
        {#each groups as group (group.kind)}
          {@const label = GROUP_LABELS[group.kind]}
          <!-- Sentence case, the count as its own number, as in the member list (ADR-0022 §3). -->
          <h2
            aria-label={strings.friends.groupLabel(label, group.relationships.length)}
            class="flex h-(--mw-component-friends-group-height) items-end justify-between px-(--mw-component-friends-row-padding-inline) pb-2xs type-category text-(--mw-component-friends-group-text)"
          >
            <span>{label}</span>
            <span aria-hidden="true">{group.relationships.length}</span>
          </h2>
          <ul>
            {#each group.relationships as relationship (relationship.user.id)}
              {@const user = relationship.user}
              {@const person = displayName(user)}
              {@const presence = client.presenceOf(user.id)}
              {@const isBusy = busyUserId === user.id}
              <li
                data-friend-row={user.id}
                class="mt-(--mw-component-friends-row-gap) flex h-(--mw-component-friends-row-height) items-center gap-sm rounded-(--mw-component-friends-row-radius) px-(--mw-component-friends-row-padding-inline) transition-colors duration-(--mw-motion-duration-fast) ease-(--mw-motion-easing-standard) focus-within:bg-(--mw-component-friends-row-background-hover) hover:bg-(--mw-component-friends-row-background-hover)"
              >
                <Avatar
                  {user}
                  sizeToken="--mw-component-friends-avatar-size"
                  presence={relationship.kind === "friend" && presence !== "offline" ? presence : null}
                  ringColorToken="--mw-color-surface-chat"
                  isDimmed={relationship.kind === "friend" && presence === "offline"}
                />
                <span class="flex min-w-0 flex-1 flex-col">
                  <span class="truncate type-member-name text-(--mw-component-friends-name-text)">{person}</span>
                  <span class="truncate type-body-sm text-(--mw-component-friends-detail-text)">{detailOf(relationship)}</span>
                </span>
                <span class="flex shrink-0 items-center gap-(--mw-component-friends-action-gap)">
                  {#if relationship.kind === "friend"}
                    {@render action("chat-circle", strings.friends.message(person), "neutral", () => void message(user), isBusy)}
                    {@render action(
                      "dots-three",
                      strings.friends.more(person),
                      "neutral",
                      (event) => openMenu(relationship, event.currentTarget as HTMLElement),
                      isBusy,
                      {
                        "aria-haspopup": "menu",
                        "aria-expanded": menuRelationship?.user.id === user.id,
                        onpointerdown: () => (menuUserIdAtPress = menuRelationship?.user.id ?? null),
                      },
                    )}
                  {:else if relationship.kind === "incoming_request"}
                    {@render action(
                      "check",
                      strings.friends.accept(person),
                      "positive",
                      () =>
                        void act(
                          relationship,
                          () => client.acceptFriendRequest(user.id),
                          strings.friends.accepted,
                          strings.friends.acceptFailed,
                        ),
                      isBusy,
                    )}
                    {@render action(
                      "x",
                      strings.friends.ignore(person),
                      "danger",
                      () =>
                        void act(
                          relationship,
                          () => client.removeRelationship(user.id),
                          strings.friends.ignored,
                          strings.friends.ignoreFailed,
                        ),
                      isBusy,
                    )}
                  {:else if relationship.kind === "outgoing_request"}
                    {@render action(
                      "x",
                      strings.friends.cancel(person),
                      "danger",
                      () =>
                        void act(
                          relationship,
                          () => client.removeRelationship(user.id),
                          strings.friends.cancelled,
                          strings.friends.cancelFailed,
                        ),
                      isBusy,
                    )}
                  {:else}
                    <Button
                      aria-label={strings.friends.unblockLabel(person)}
                      isDisabled={isBusy}
                      onclick={() =>
                        void act(
                          relationship,
                          () => client.removeRelationship(user.id),
                          strings.friends.unblocked,
                          strings.friends.unblockFailed,
                        )}>{strings.friends.unblock}</Button
                    >
                  {/if}
                </span>
              </li>
            {/each}
          </ul>
        {/each}
      {/if}
    </div>
  {/key}

  <p class="sr-only" aria-live="polite">{announcement}</p>
</section>

<!-- One menu for every row's More button, placed under the button that opened it. -->
<div
  bind:this={menu}
  popover="auto"
  role="menu"
  tabindex="-1"
  aria-label={menuRelationship ? strings.friends.menu(displayName(menuRelationship.user)) : strings.friends.title}
  class="fixed inset-auto m-0 w-max max-w-(--mw-component-context-menu-max-width) min-w-(--mw-component-context-menu-min-width) flex-col rounded-(--mw-component-context-menu-radius) border-(length:--mw-component-context-menu-border-width) border-(--mw-component-context-menu-border) bg-(--mw-component-context-menu-background) p-(--mw-component-context-menu-padding) shadow-(--mw-component-context-menu-shadow) [&:popover-open]:flex [&:popover-open]:animate-[mw-drop-in_var(--mw-motion-duration-fast)_var(--mw-motion-easing-enter)_both]"
  style:top="calc({menuPosition.top}px + var(--mw-space-2xs))"
  style:right="{menuPosition.right}px"
  onkeydown={(event) => moveFocusWithinMenu(event, menu)}
  ontoggle={(event) => {
    if (event.newState === "open") {
      return;
    }
    menuRelationship = null;
    if (menu?.contains(document.activeElement) || document.activeElement === document.body) {
      menuButton?.focus();
    }
  }}
>
  {#each [{ action: "remove" as const, label: strings.friends.removeFriend, icon: "user-minus" as const }, { action: "block" as const, label: strings.friends.block, icon: "prohibit" as const }] as item (item.action)}
    <button
      type="button"
      role="menuitem"
      class="flex h-(--mw-component-context-menu-item-height) w-full items-center justify-between gap-xs rounded-(--mw-component-context-menu-item-radius) px-(--mw-component-context-menu-item-padding-inline) type-menu-item text-(--mw-component-context-menu-danger-text) hover:bg-(--mw-component-context-menu-danger-background-focus) hover:text-(--mw-component-context-menu-danger-text-focus) focus-visible:bg-(--mw-component-context-menu-danger-background-focus) focus-visible:text-(--mw-component-context-menu-danger-text-focus) pointer-coarse:h-(--mw-size-hit-web-min)"
      onclick={() => chooseMenuAction(item.action)}
    >
      {item.label}
      <Icon name={item.icon} class="size-(--mw-component-context-menu-item-icon-size)" />
    </button>
  {/each}
</div>

{#if confirming}
  {@const pending = confirming}
  <RelationshipConfirmDialog
    {client}
    action={pending.action}
    user={pending.user}
    onclose={() => (confirming = null)}
    ondone={(sentence) => (announcement = sentence)}
    fallbackFocus={() => document.getElementById(`${id}-tab-${tab}`)?.focus()}
  />
{/if}

<style>
  /* Changing tab answers with the channel switch's small rise (DESIGN.md §2.8); reduced motion
     sets the duration to 0. */
  .tab-enter {
    animation: mw-rise var(--mw-motion-duration-base) var(--mw-motion-easing-enter) both;
  }
</style>
