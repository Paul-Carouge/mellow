<script lang="ts">
  import { tick } from "svelte";

  import { strings } from "../i18n";
  import { type MellowClient, describeError } from "../lib/client.svelte";
  import type { Guild, Member, PublicUser } from "../lib/protocol";
  import Avatar from "./Avatar.svelte";
  import Button from "./Button.svelte";
  import { displayName } from "./format";
  import IconButton from "./IconButton.svelte";
  import { nameColorsByUserId } from "./name-colors.svelte";
  import ProfileCard from "./ProfileCard.svelte";

  // The members side sheet (DESIGN.md §5.7, ADR-0022 §1): Online then Offline, offline avatars
  // dimmed while names keep full contrast; choosing a member shows their profile in the sheet,
  // with "Send message".
  interface Props {
    client: MellowClient;
    guild: Guild;
    /** Docked beside the conversation, or over it in a drawer (which handles Esc itself). */
    isDocked: boolean;
    /** The phone layout: the sheet covers the window, without corners. */
    isFullWindow?: boolean;
    onclose: () => void;
    /** After "Send message" opened a conversation (the overlay closes). */
    onopenedconversation: () => void;
  }

  let { client, guild, isDocked, isFullWindow = false, onclose, onopenedconversation }: Props = $props();

  const members = $derived(client.membersByGuild.get(guild.id) ?? null);
  // Role colours on names, readable on the side sheet in the current theme (DESIGN.md §2.3).
  const nameColors = $derived(nameColorsByUserId(guild, members ?? undefined, "component-side-sheet-background"));
  const isOnline = (userId: string) => client.presenceOf(userId) !== "offline";
  const groups = $derived.by(() => {
    const byName = (left: Member, right: Member) => displayName(left.user).localeCompare(displayName(right.user));
    const all = [...(members ?? [])].sort(byName);
    return [
      { label: strings.common.online, isOnline: true, members: all.filter((member) => isOnline(member.user.id)) },
      { label: strings.common.offline, isOnline: false, members: all.filter((member) => !isOnline(member.user.id)) },
    ].filter((group) => group.members.length > 0);
  });

  let profileUserId = $state<string | null>(null);
  // From the list, so a USER_UPDATE (a new avatar, say) shows in an open profile too.
  const profile = $derived(members?.find((member) => member.user.id === profileUserId)?.user ?? null);
  let opener: HTMLElement | null = null;
  let backButton = $state<HTMLButtonElement | null>(null);
  let isOpening = $state(false);
  let errorMessage = $state("");

  async function showProfile(user: PublicUser, row: HTMLElement) {
    opener = row;
    profileUserId = user.id;
    errorMessage = "";
    await tick();
    backButton?.focus();
  }

  async function showMembers() {
    profileUserId = null;
    await tick();
    if (opener?.isConnected) {
      opener.focus();
    }
  }

  async function message(user: PublicUser) {
    isOpening = true;
    errorMessage = "";
    try {
      await client.openDirectMessage(user.id);
      onopenedconversation();
    } catch (error) {
      errorMessage = describeError(error, strings.members.openConversationFailed);
    } finally {
      isOpening = false;
    }
  }

  const headButtonClass =
    "size-(--mw-component-user-panel-button-size) text-(--mw-component-user-panel-button-icon) hover:bg-state-hover hover:text-text-normal";
  const headIconClass = "size-(--mw-component-user-panel-button-icon-size)";
</script>

<!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
<aside
  aria-label={profile ? strings.members.profileOf(displayName(profile)) : strings.shell.membersOf(guild.name)}
  class={[
    "flex h-full shrink-0 flex-col overflow-hidden bg-(--mw-component-side-sheet-background)",
    isFullWindow ? "w-full" : "w-(--mw-component-side-sheet-width) rounded-(--mw-component-side-sheet-radius)",
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
    {#if profile}
      <!-- The caret points back, towards the list. -->
      <IconButton
        bind:element={backButton}
        icon="caret-right"
        label={strings.members.backToMembers}
        class={headButtonClass}
        iconClass="{headIconClass} rotate-180"
        onclick={() => void showMembers()}
      />
    {/if}
    <h2 class="min-w-0 flex-1 truncate type-heading-md">{profile ? strings.members.profile : strings.members.members}</h2>
    <IconButton icon="x" label={strings.common.close} class={headButtonClass} iconClass={headIconClass} onclick={onclose} />
  </div>

  <div class="min-h-0 flex-1 overflow-y-auto pb-md [scrollbar-width:thin]">
    {#if profile}
      {@const isSelf = profile.id === client.user?.id}
      <div class="profile-enter">
        <ProfileCard user={profile} surfaceToken="--mw-component-side-sheet-background">
          {#if !isSelf}
            <div class="mt-xs">
              <Button variant="primary" isFullWidth isDisabled={isOpening} onclick={() => profile && message(profile)}>
                {isOpening ? strings.members.opening : strings.members.sendMessage}
              </Button>
            </div>
          {/if}
          <div role="alert" class="contents">
            {#if errorMessage}
              <p class="type-body text-(--mw-component-profile-popover-error-text)">{errorMessage}</p>
            {/if}
          </div>
        </ProfileCard>
      </div>
    {:else}
      {#if members === null && client.failedMemberGuildIds.has(guild.id)}
        <div role="alert" class="flex flex-col items-start gap-sm px-md pt-xs">
          <p class="type-body text-text-muted">{strings.members.loadFailed}</p>
          <Button onclick={() => client.retryLoadingMembers()}>{strings.common.tryAgain}</Button>
        </div>
      {:else if members === null}
        <p role="status" class="px-md pt-xs type-body text-text-muted">{strings.members.loading}</p>
      {/if}
      {#each groups as group (group.label)}
        <!-- Sentence case, the count as its own number (ADR-0022 §3). -->
        <h3
          aria-label={strings.members.groupLabel(group.label, group.members.length)}
          class="flex h-(--mw-component-member-list-group-height) items-end justify-between px-md pb-2xs type-category text-(--mw-component-member-list-group-text)"
        >
          <span>{group.label}</span>
          <span aria-hidden="true">{group.members.length}</span>
        </h3>
        <ul>
          {#each group.members as member (member.user.id)}
            {@const isOnline = group.isOnline}
            {@const presence = client.presenceOf(member.user.id)}
            {@const customStatus = client.customStatusOf(member.user.id)}
            <li class="mx-(--mw-component-member-row-margin-inline) mt-(--mw-component-member-row-gap)">
              <button
                type="button"
                aria-label={strings.members.memberLabel(displayName(member.user), strings.presence[presence], customStatus)}
                class="flex h-(--mw-component-member-row-height) w-full items-center gap-sm rounded-(--mw-component-member-row-radius) px-(--mw-component-member-row-padding-inline) text-start text-(--mw-component-member-row-name-text) transition-colors duration-(--mw-motion-duration-fast) ease-(--mw-motion-easing-standard) hover:bg-(--mw-component-member-row-background-hover) hover:text-(--mw-component-member-row-name-text-hover) focus-visible:focus-ring-inset pointer-coarse:min-h-(--mw-size-hit-web-min)"
                onclick={(event) => void showProfile(member.user, event.currentTarget)}
              >
                <Avatar
                  user={member.user}
                  sizeToken="--mw-component-member-row-avatar-size"
                  presence={isOnline ? presence : null}
                  ringColorToken="--mw-component-side-sheet-background"
                  isDimmed={!isOnline}
                />
                <span class="flex min-w-0 flex-1 flex-col">
                  <span class="truncate type-member-name" style:color={nameColors.get(member.user.id)}
                    >{displayName(member.user)}</span
                  >
                  {#if customStatus}
                    <span class="truncate type-body-sm text-(--mw-component-member-row-activity-text)">{customStatus}</span>
                  {/if}
                </span>
              </button>
            </li>
          {/each}
        </ul>
      {/each}
    {/if}
  </div>
</aside>

<style>
  .side-sheet-docked {
    animation: mw-slide-from-end var(--mw-motion-duration-slow) var(--mw-motion-easing-enter) both;
  }
  .profile-enter {
    animation: mw-rise var(--mw-motion-duration-base) var(--mw-motion-easing-enter) both;
  }
</style>
