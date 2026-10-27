<script lang="ts">
  import { strings } from "../i18n";
  import type { MellowClient } from "../lib/client.svelte";
  import type { DirectMessageChannel, PublicUser } from "../lib/protocol";
  import Avatar from "./Avatar.svelte";
  import { displayName } from "./format";
  import GroupConfirmDialog from "./GroupConfirmDialog.svelte";
  import { conversationDisplayName } from "./group-conversations";
  import Icon, { type IconName } from "./Icon.svelte";
  import IconButton from "./IconButton.svelte";
  import RenameGroupDialog from "./RenameGroupDialog.svelte";

  // A group's members in the side sheet (DESIGN.md §5.6, §5.7, ADR-0026): the group's actions as
  // the server menu words them (Add friends in apricot, Leave group in red, last), then everyone in
  // it, you included, the owner marked in words. The owner removes others from their row.
  interface Props {
    client: MellowClient;
    group: DirectMessageChannel;
    /** Docked beside the conversation, or over it in a drawer (which handles Esc itself). */
    isDocked: boolean;
    /** The phone layout: the sheet covers the window, without corners. */
    isFullWindow?: boolean;
    onclose: () => void;
    /** The "Add friends" picker, owned by the shell like "New message". */
    onaddfriends: () => void;
    /** After leaving: the conversation is gone, Friends shows, and focus goes there. */
    focusAfterLeaving: () => void;
  }

  let { client, group, isDocked, isFullWindow = false, onclose, onaddfriends, focusAfterLeaving }: Props = $props();

  const currentUser = $derived(client.user);
  const isOwner = $derived(currentUser !== null && group.owner_id === currentUser.id);
  const people = $derived.by(() => {
    const everyone: PublicUser[] = [
      ...(currentUser ? [currentUser] : []),
      ...group.recipients.filter((recipient) => recipient.id !== currentUser?.id),
    ];
    return everyone.sort((left, right) => displayName(left).localeCompare(displayName(right)));
  });

  let sheet = $state<HTMLElement | null>(null);
  let dialog = $state<{ kind: "rename" } | { kind: "leave" } | { kind: "remove"; person: PublicUser } | null>(null);
  let announcement = $state("");
  /** Where the removed person's row was, so focus lands on the next row's action once it leaves. */
  let removedRowIndex = 0;

  function focusRemoveActionNear(index: number) {
    const actions = [...(sheet?.querySelectorAll<HTMLElement>("[data-remove-member]") ?? [])];
    const target = actions[Math.min(index, actions.length - 1)] ?? sheet?.querySelector<HTMLElement>("[data-group-action]");
    target?.focus();
  }

  function confirmRemoval(person: PublicUser, button: HTMLElement) {
    removedRowIndex = Math.max([...(sheet?.querySelectorAll("[data-remove-member]") ?? [])].indexOf(button), 0);
    dialog = { kind: "remove", person };
  }

  // Someone else removed them meanwhile (or they left): the question no longer stands.
  $effect(() => {
    const removing = dialog?.kind === "remove" ? dialog.person : null;
    if (removing && !group.recipients.some((recipient) => recipient.id === removing.id)) {
      dialog = null;
    }
  });

  const headButtonClass =
    "size-(--mw-component-user-panel-button-size) text-(--mw-component-user-panel-button-icon) hover:bg-state-hover hover:text-text-normal";
  const headIconClass = "size-(--mw-component-user-panel-button-icon-size)";
</script>

{#snippet groupAction(label: string, icon: IconName, tone: "accent" | "normal" | "danger", onclick: () => void)}
  <li class="mx-(--mw-component-member-row-margin-inline) mt-(--mw-component-member-row-gap)">
    <button
      type="button"
      data-group-action
      aria-haspopup="dialog"
      class={[
        "flex h-(--mw-component-context-menu-item-height) w-full items-center justify-between gap-xs rounded-(--mw-component-member-row-radius) px-(--mw-component-member-row-padding-inline) type-menu-item transition-colors duration-(--mw-motion-duration-fast) ease-(--mw-motion-easing-standard) hover:bg-(--mw-component-member-row-background-hover) focus-visible:focus-ring-inset pointer-coarse:h-(--mw-size-hit-web-min)",
        tone === "accent"
          ? "text-(--mw-component-context-menu-accent-text)"
          : tone === "danger"
            ? "text-(--mw-component-context-menu-danger-text)"
            : "text-(--mw-component-member-row-name-text) hover:text-(--mw-component-member-row-name-text-hover)",
      ]}
      {onclick}
    >
      {label}
      <Icon name={icon} class="size-(--mw-component-context-menu-item-icon-size)" />
    </button>
  </li>
{/snippet}

<!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
<aside
  bind:this={sheet}
  aria-label={strings.shell.membersOf(conversationDisplayName(group, currentUser?.id ?? null))}
  class={[
    "flex h-full shrink-0 flex-col overflow-hidden bg-(--mw-component-side-sheet-background)",
    isFullWindow ? "w-full safe-area-padding" : "w-(--mw-component-side-sheet-width) rounded-(--mw-component-side-sheet-radius)",
    isDocked && "side-sheet-docked",
  ]}
  onkeydown={(event) => {
    if (isDocked && event.key === "Escape" && dialog === null) {
      event.preventDefault();
      onclose();
    }
  }}
>
  <div class="flex h-(--mw-component-chat-header-height) shrink-0 items-center gap-2xs ps-md pe-2xs text-text-heading">
    <h2 class="min-w-0 flex-1 truncate type-heading-md">{strings.groups.members}</h2>
    <IconButton icon="x" label={strings.common.close} class={headButtonClass} iconClass={headIconClass} onclick={onclose} />
  </div>

  <div class="min-h-0 flex-1 overflow-y-auto pb-md [scrollbar-width:thin]">
    <ul>
      {@render groupAction(strings.groups.addFriends, "user-plus", "accent", onaddfriends)}
      {@render groupAction(strings.groups.rename, "pencil-simple", "normal", () => (dialog = { kind: "rename" }))}
      {@render groupAction(strings.groups.leave, "sign-out", "danger", () => (dialog = { kind: "leave" }))}
    </ul>

    <!-- Sentence case, the count as its own number, as in the member list (ADR-0022 §3). -->
    <h3
      aria-label={strings.members.groupLabel(strings.groups.members, people.length)}
      class="flex h-(--mw-component-member-list-group-height) items-end justify-between px-md pb-2xs type-category text-(--mw-component-member-list-group-text)"
    >
      <span>{strings.groups.members}</span>
      <span aria-hidden="true">{people.length}</span>
    </h3>
    <ul>
      {#each people as person (person.id)}
        {@const presence = client.presenceOf(person.id)}
        {@const isOnline = presence !== "offline"}
        {@const isPersonOwner = person.id === group.owner_id}
        {@const customStatus = client.customStatusOf(person.id)}
        {@const name = displayName(person)}
        <li
          class="group/row mx-(--mw-component-member-row-margin-inline) mt-(--mw-component-member-row-gap) flex min-h-(--mw-component-member-row-height) items-center gap-sm rounded-(--mw-component-member-row-radius) px-(--mw-component-member-row-padding-inline) text-(--mw-component-member-row-name-text) pointer-coarse:min-h-(--mw-size-hit-web-min)"
        >
          <Avatar
            user={person}
            sizeToken="--mw-component-member-row-avatar-size"
            presence={isOnline ? presence : null}
            ringColorToken="--mw-component-side-sheet-background"
            isDimmed={!isOnline}
          />
          <!-- One sentence for screen readers; the owner is said in words, never by a mark alone. -->
          <span class="sr-only">{strings.groups.memberLabel(name, strings.presence[presence], isPersonOwner, customStatus)}</span>
          <span aria-hidden="true" class="flex min-w-0 flex-1 flex-col">
            <span class="truncate type-member-name">{name}</span>
            {#if isPersonOwner || customStatus}
              <span class="truncate type-body-sm text-(--mw-component-member-row-activity-text)">
                {isPersonOwner ? strings.groups.owner : customStatus}
              </span>
            {/if}
          </span>
          {#if isOwner && person.id !== currentUser?.id}
            <IconButton
              icon="user-minus"
              label={strings.groups.remove(name)}
              aria-haspopup="dialog"
              data-remove-member
              class="size-(--mw-component-friends-action-size) rounded-full! text-(--mw-component-friends-action-icon) hover:bg-(--mw-component-friends-action-background) hover:text-(--mw-component-friends-action-icon-danger-hover)"
              iconClass="size-(--mw-component-friends-action-icon-size)"
              onclick={(event) => confirmRemoval(person, event.currentTarget)}
            />
          {/if}
        </li>
      {/each}
    </ul>
    <p class="sr-only" aria-live="polite">{announcement}</p>
  </div>
</aside>

{#if dialog?.kind === "rename"}
  <RenameGroupDialog {client} {group} onclose={() => (dialog = null)} ondone={(sentence) => (announcement = sentence)} />
{:else if dialog?.kind === "leave"}
  <GroupConfirmDialog
    {client}
    {group}
    person={null}
    onclose={() => (dialog = null)}
    ondone={(sentence) => (announcement = sentence)}
    fallbackFocus={focusAfterLeaving}
  />
{:else if dialog?.kind === "remove"}
  <GroupConfirmDialog
    {client}
    {group}
    person={dialog.person}
    onclose={() => (dialog = null)}
    ondone={(sentence) => (announcement = sentence)}
    fallbackFocus={() => focusRemoveActionNear(removedRowIndex)}
  />
{/if}

<style>
  .side-sheet-docked {
    animation: mw-slide-from-end var(--mw-motion-duration-slow) var(--mw-motion-easing-enter) both;
  }
</style>
