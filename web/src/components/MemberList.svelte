<script lang="ts">
  import { tick } from "svelte";

  import { type MellowClient, describeError } from "../lib/client.svelte";
  import type { Guild, Member, PublicUser } from "../lib/protocol";
  import Avatar from "./Avatar.svelte";
  import Button from "./Button.svelte";
  import { displayName } from "./format";
  import { lengthTokenInPixels } from "./tokens";

  // Member list and profile popover (DESIGN.md §5.7): Online then Offline, offline avatars dimmed
  // while names keep full contrast; a click opens the profile with "Message".
  interface Props {
    client: MellowClient;
    guild: Guild;
    /** After "Message" opened a conversation (the narrow layout closes its overlay). */
    onopenedconversation: () => void;
  }

  let { client, guild, onopenedconversation }: Props = $props();

  const members = $derived(client.membersByGuild.get(guild.id) ?? null);
  // The instance does not list the signed-in user among the online ids: they are online here.
  const isOnline = (userId: string) => userId === client.user?.id || client.onlineUserIds.has(userId);
  const groups = $derived.by(() => {
    const byName = (left: Member, right: Member) => displayName(left.user).localeCompare(displayName(right.user));
    const all = [...(members ?? [])].sort(byName);
    return [
      { label: "Online", members: all.filter((member) => isOnline(member.user.id)) },
      { label: "Offline", members: all.filter((member) => !isOnline(member.user.id)) },
    ].filter((group) => group.members.length > 0);
  });

  let popover = $state<HTMLElement | null>(null);
  let profile = $state<PublicUser | null>(null);
  let opener: HTMLElement | null = null;
  let position = $state({ top: 0, right: 0 });
  let isOpening = $state(false);
  let errorMessage = $state("");

  async function openProfile(user: PublicUser, row: HTMLElement) {
    if (!popover) {
      return;
    }
    opener = row;
    profile = user;
    errorMessage = "";
    popover.showPopover();
    await tick();
    // Beside the row on the leading side, as on Discord, kept inside the window.
    const bounds = row.getBoundingClientRect();
    const margin = lengthTokenInPixels("space-md");
    const gap = lengthTokenInPixels("space-xs");
    position = {
      top: Math.max(margin, Math.min(bounds.top, window.innerHeight - popover.offsetHeight - margin)),
      right: Math.min(window.innerWidth - bounds.left + gap, window.innerWidth - popover.offsetWidth - margin),
    };
    // "Message" when there is one; otherwise the card itself, so Esc and screen readers start there.
    (popover.querySelector<HTMLElement>("button") ?? popover).focus();
  }

  async function message(user: PublicUser) {
    isOpening = true;
    errorMessage = "";
    try {
      await client.openDirectMessage(user.id);
      popover?.hidePopover();
      onopenedconversation();
    } catch (error) {
      errorMessage = describeError(error, "Couldn't open the conversation");
    } finally {
      isOpening = false;
    }
  }
</script>

<aside
  aria-label="Members of {guild.name}"
  class="flex h-full w-(--mw-component-member-list-width) shrink-0 flex-col overflow-y-auto bg-(--mw-component-member-list-background) pb-xs"
>
  {#if members === null}
    <p role="status" class="px-md pt-lg type-body text-text-muted">Loading members…</p>
  {/if}
  {#each groups as group (group.label)}
    <h2
      class="flex h-(--mw-component-member-list-group-height) items-end px-md pb-2xs type-category text-(--mw-component-member-list-group-text)"
    >
      {group.label} — {group.members.length}
    </h2>
    <ul>
      {#each group.members as member (member.user.id)}
        {@const isOnline = group.label === "Online"}
        <li class="mb-(--mw-component-member-row-gap) px-(--mw-component-member-row-margin-inline)">
          <button
            type="button"
            aria-haspopup="dialog"
            aria-label="{displayName(member.user)}, {isOnline ? 'online' : 'offline'}"
            class="group flex h-(--mw-component-member-row-height) w-full items-center gap-sm rounded-(--mw-component-member-row-radius) px-(--mw-component-member-row-padding-inline) text-start text-(--mw-component-member-row-name-text) hover:bg-(--mw-component-member-row-background-hover) hover:text-(--mw-component-member-row-name-text-hover) focus-visible:focus-ring-inset pointer-coarse:min-h-(--mw-size-hit-web-min)"
            onclick={(event) => void openProfile(member.user, event.currentTarget)}
          >
            <Avatar
              user={member.user}
              sizeToken="--mw-component-member-row-avatar-size"
              presence={isOnline ? "online" : null}
              ringColorToken="--mw-component-member-list-background"
              isDimmed={!isOnline}
            />
            <span class="min-w-0 flex-1 truncate type-member-name">{displayName(member.user)}</span>
          </button>
        </li>
      {/each}
    </ul>
  {/each}
</aside>

<!-- Light dismiss and Esc come from the popover; focus goes back to the member who opened it. -->
<div
  bind:this={popover}
  popover="auto"
  role="dialog"
  tabindex="-1"
  aria-label={profile ? `Profile of ${displayName(profile)}` : "Profile"}
  class="fixed inset-auto m-0 w-(--mw-component-profile-popover-width) outline-none max-w-[calc(100vw_-_2*var(--mw-space-md))] overflow-hidden rounded-(--mw-component-profile-popover-radius) bg-(--mw-component-profile-popover-background) p-0 shadow-(--mw-component-profile-popover-shadow)"
  style:top="{position.top}px"
  style:right="{position.right}px"
  ontoggle={(event) => {
    if (event.newState === "closed" && (popover?.contains(document.activeElement) || document.activeElement === document.body)) {
      opener?.focus();
    }
  }}
>
  {#if profile}
    {@const isSelf = profile.id === client.user?.id}
    <div aria-hidden="true" class="h-(--mw-component-profile-popover-banner-height) bg-(--mw-component-profile-popover-banner)"></div>
    <div class="flex flex-col gap-(--mw-component-profile-popover-text-gap) px-(--mw-component-profile-popover-padding) pb-(--mw-component-profile-popover-padding)">
      <span
        class="-mt-[calc(var(--mw-component-profile-popover-avatar-size)/2)] w-fit rounded-full border-(length:--mw-component-profile-popover-avatar-ring-width) border-(--mw-component-profile-popover-background)"
      >
        <Avatar user={profile} sizeToken="--mw-component-profile-popover-avatar-size" />
      </span>
      <div>
        <h2 class="type-heading-md break-words text-(--mw-component-profile-popover-name-text)">{displayName(profile)}</h2>
        <p class="type-body break-words text-(--mw-component-profile-popover-username-text)">
          @{profile.username}
        </p>
      </div>
      {#if !isSelf}
        <Button variant="primary" isFullWidth isDisabled={isOpening} onclick={() => profile && message(profile)}>
          {isOpening ? "Opening…" : "Message"}
        </Button>
      {/if}
      <div role="alert" class="contents">
        {#if errorMessage}
          <p class="type-body text-(--mw-component-profile-popover-error-text)">{errorMessage}</p>
        {/if}
      </div>
    </div>
  {/if}
</div>
