<script lang="ts">
  import { strings } from "../i18n";
  import type { MellowClient } from "../lib/client.svelte";
  import type { Guild } from "../lib/protocol";
  import Avatar from "./Avatar.svelte";
  import Button from "./Button.svelte";
  import { contextMenuTrigger } from "./context-menu-trigger";
  import { displayName } from "./format";
  import IconButton from "./IconButton.svelte";
  import { memberListGroups, membersSortedByDisplayName } from "./member-order";
  import { nameColorsByUserId } from "./name-colors.svelte";
  import PersonContextMenu from "./PersonContextMenu.svelte";
  import { openProfileModal, profileTrigger } from "./profile-surfaces.svelte";
  import RoleIcon from "./RoleIcon.svelte";
  import { roleIconsByUserId } from "./role-icons";

  // The members side sheet (DESIGN.md §5.7, ADR-0022 §1): Online then Offline, offline avatars
  // dimmed while names keep full contrast. Choosing a member opens their profile modal; resting on
  // one shows their hover card beside the sheet (§5.7.1 to §5.7.3). The sheet stays the list.
  interface Props {
    client: MellowClient;
    guild: Guild;
    /** Docked beside the conversation, or over it in a drawer (which handles Esc itself). */
    isDocked: boolean;
    /** The phone layout: the sheet covers the window, without corners. */
    isFullWindow?: boolean;
    onclose: () => void;
    /** After "Send message" in a person's menu opened a conversation (the overlay closes). */
    onopenedconversation: () => void;
  }

  let { client, guild, isDocked, isFullWindow = false, onclose, onopenedconversation }: Props = $props();

  const members = $derived(client.membersByGuild.get(guild.id) ?? null);
  // Role colours on names, readable on a hovered row, the sheet's least contrasting background,
  // in the current theme (DESIGN.md §2.3).
  const nameColors = $derived(
    nameColorsByUserId(guild, members ?? undefined, "component-member-row-background-hover"),
  );
  // Each member's highest role with an icon, beside their name (ADR-0050).
  const roleIcons = $derived(roleIconsByUserId(guild, members ?? undefined));
  const isOnline = (userId: string) => client.presenceOf(userId) !== "offline";
  // Sorted only when the members change: a presence update just splits the sorted list again.
  const sortedMembers = $derived(membersSortedByDisplayName(members ?? []));
  // Roles displayed separately first, then Online and Offline (DESIGN.md §5.7).
  const groups = $derived(
    memberListGroups(sortedMembers, guild.roles, isOnline).map((group) => ({
      key: group.kind === "role" ? group.role.id : group.kind,
      label:
        group.kind === "role" ? group.role.name : group.kind === "online" ? strings.common.online : strings.common.offline,
      /** Its icon goes before its name (ADR-0050). */
      role: group.kind === "role" ? group.role : null,
      isOnline: group.kind !== "offline",
      members: group.members,
    })),
  );

  let personMenu = $state<PersonContextMenu | null>(null);

  const headButtonClass =
    "size-(--mw-component-user-panel-button-size) text-(--mw-component-user-panel-button-icon) hover:bg-state-hover hover:text-text-normal";
  const headIconClass = "size-(--mw-component-user-panel-button-icon-size)";
</script>

<!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
<aside
  aria-label={strings.shell.membersOf(guild.name)}
  data-hover-card-column
  class={[
    "flex h-full shrink-0 flex-col overflow-hidden bg-(--mw-component-side-sheet-background)",
    isFullWindow ? "w-full safe-area-padding" : "w-(--mw-component-side-sheet-width) rounded-(--mw-component-side-sheet-radius)",
    isDocked && "side-sheet-docked",
  ]}
  onkeydown={(event) => {
    if (isDocked && event.key === "Escape") {
      event.preventDefault();
      onclose();
    }
  }}
>
  <div class="flex h-(--mw-component-chat-header-height) shrink-0 items-center gap-2xs ps-md pe-2xs text-text-heading">
    <h2 class="min-w-0 flex-1 truncate type-heading-md">{strings.members.members}</h2>
    <IconButton icon="x" label={strings.common.close} class={headButtonClass} iconClass={headIconClass} onclick={onclose} />
  </div>

  <div class="min-h-0 flex-1 overflow-y-auto pb-md [scrollbar-width:thin]">
    {#if members === null && client.failedMemberGuildIds.has(guild.id)}
      <div role="alert" class="flex flex-col items-start gap-sm px-md pt-xs">
        <p class="type-body text-text-muted">{strings.members.loadFailed}</p>
        <Button onclick={() => client.retryLoadingMembers()}>{strings.common.tryAgain}</Button>
      </div>
    {:else if members === null}
      <p role="status" class="px-md pt-xs type-body text-text-muted">{strings.members.loading}</p>
    {/if}
    {#each groups as group (group.key)}
      <!-- Sentence case, the count as its own number (ADR-0022 §3). -->
      <h3
        aria-label={strings.members.groupLabel(group.label, group.members.length)}
        class="flex h-(--mw-component-member-list-group-height) items-end justify-between px-md pb-2xs type-category text-(--mw-component-member-list-group-text)"
      >
        <span class="flex min-w-0 items-center gap-(--mw-component-role-icon-gap)">
          {#if group.role}<RoleIcon
              role={group.role}
              sizeToken="--mw-component-role-icon-group-size"
              isDecorative
            />{/if}<span class="truncate">{group.label}</span>
        </span>
        <span aria-hidden="true">{group.members.length}</span>
      </h3>
      <ul>
        {#each group.members as member (member.user.id)}
          {@const isOnline = group.isOnline}
          {@const presence = client.presenceOf(member.user.id)}
          {@const customStatus = client.customStatusOf(member.user.id)}
          {@const roleIcon = roleIcons.get(member.user.id)}
          <li class="mx-(--mw-component-member-row-margin-inline) mt-(--mw-component-member-row-gap)">
            <button
              type="button"
              aria-haspopup="dialog"
              aria-label={strings.members.memberLabel(displayName(member.user), strings.presence[presence], customStatus) +
                (roleIcon ? `, ${roleIcon.name}` : "")}
              class="flex h-(--mw-component-member-row-height) w-full items-center gap-sm rounded-(--mw-component-member-row-radius) px-(--mw-component-member-row-padding-inline) text-start text-(--mw-component-member-row-name-text) transition-colors duration-(--mw-motion-duration-fast) ease-(--mw-motion-easing-standard) hover:bg-(--mw-component-member-row-background-hover) hover:text-(--mw-component-member-row-name-text-hover) focus-visible:focus-ring-inset pointer-coarse:min-h-(--mw-size-hit-web-min)"
              onclick={(event) => openProfileModal(member.user, event.currentTarget)}
              {@attach profileTrigger(member.user, { placement: "beside" })}
              {@attach contextMenuTrigger((origin, invoker) => personMenu?.openFor(member.user, origin, invoker) ?? false)}
            >
              <Avatar
                user={member.user}
                sizeToken="--mw-component-member-row-avatar-size"
                presence={isOnline ? presence : null}
                ringColorToken="--mw-component-side-sheet-background"
                isDimmed={!isOnline}
              />
              <span class="flex min-w-0 flex-1 flex-col">
                <span class="flex min-w-0 items-center gap-(--mw-component-role-icon-gap)">
                  <span class="truncate type-member-name" style:color={nameColors.get(member.user.id)}
                    >{displayName(member.user)}</span
                  >
                  {#if roleIcon}<RoleIcon role={roleIcon} sizeToken="--mw-component-role-icon-member-size" />{/if}
                </span>
                {#if customStatus}
                  <span class="truncate type-body-sm text-(--mw-component-member-row-activity-text)">{customStatus}</span>
                {/if}
              </span>
            </button>
          </li>
        {/each}
      </ul>
    {/each}
  </div>
</aside>

<PersonContextMenu bind:this={personMenu} {client} {onopenedconversation} />

<style>
  .side-sheet-docked {
    animation: mw-slide-from-end var(--mw-motion-duration-slow) var(--mw-motion-easing-enter) both;
  }
</style>
