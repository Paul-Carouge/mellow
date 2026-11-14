<script lang="ts">
  import { strings } from "../i18n";
  import type { MellowClient } from "../lib/client.svelte";
  import type { PublicUser } from "../lib/protocol";
  import Avatar from "./Avatar.svelte";
  import { displayName, mediumDateLabel } from "./format";
  import { type HoverCardSide, hoverCardPosition } from "./hover-card-placement";
  import { parseInlineMarkdown } from "./markdown";
  import MessageMarkdown from "./MessageMarkdown.svelte";
  import { profileFactsOf } from "./profile-facts";
  import {
    closeProfileHoverCard,
    openProfileModal,
    profileHoverCardPointer,
    readRecentUserProfile,
    shownProfileHoverCard,
  } from "./profile-surfaces.svelte";
  import RoleChips from "./RoleChips.svelte";
  import RoleIcon from "./RoleIcon.svelte";
  import { durationTokenInMilliseconds, lengthTokenInPixels, numberTokenValue } from "./tokens";
  import { createUserProfileLoader } from "./user-profile-loader.svelte";

  // A person's hover card (DESIGN.md §5.7.2): a glance at who this is, after the pointer or
  // keyboard focus rests on their name. One per top-layer context: the shell's, and one inside each
  // drawer, since a modal dialog makes everything outside it inert, popovers included; each shows
  // only the cards of the names inside its own dialog.
  interface Props {
    client: MellowClient;
  }

  let { client }: Props = $props();

  let card = $state<HTMLElement | null>(null);
  /** The dialog this host lives in (a drawer), or null for the shell's; undefined until mounted. */
  const hostDialog = $derived(card ? (card.parentElement?.closest("dialog") ?? null) : undefined);

  const target = $derived.by(() => {
    const shown = shownProfileHoverCard();
    return shown !== null && hostDialog !== undefined && shown.anchor.closest("dialog") === hostDialog ? shown : null;
  });

  const guild = $derived(client.selectedGuild);
  const facts = $derived(target ? profileFactsOf(target.user.id, guild, guild ? client.membersByGuild.get(guild.id) : undefined) : null);
  // The freshest copy: a member's row follows USER_UPDATE, the trigger's copy may not.
  const user = $derived<PublicUser | null>(target ? (facts?.member?.user ?? target.user) : null);
  const presence = $derived(user ? client.presenceOf(user.id) : "offline");
  const customStatus = $derived(user ? client.customStatusOf(user.id) : null);

  const details = createUserProfileLoader((userId) =>
    readRecentUserProfile((id) => client.loadUserProfile(id), userId, client.user?.id ?? null),
  );
  const shownUserId = $derived(target?.user.id ?? null);
  // About me is read only once the card has stayed open a moment: sweeping the member list reads
  // nothing for the cards it only crossed (the instance limits profile reads per minute).
  $effect(() => {
    const userId = shownUserId;
    if (userId === null) {
      details.show(null);
      return;
    }
    const timer = setTimeout(
      () => details.show(userId),
      durationTokenInMilliseconds("component-profile-hover-card-details-delay"),
    );
    return () => clearTimeout(timer);
  });
  const aboutBlocks = $derived.by(() => {
    const loaded = details.current;
    return loaded.kind === "loaded" && loaded.userId === shownUserId ? parseInlineMarkdown(loaded.profile.about_me) : [];
  });

  const sinceLabel = $derived.by(() => {
    if (!facts) {
      return "";
    }
    return strings.profileCard.memberSinceDate(
      mediumDateLabel(facts.joinedServerAtUnixMilliseconds ?? facts.joinedMellowAtUnixMilliseconds),
    );
  });

  let position = $state<{ top: number; left: number; side: HoverCardSide } | null>(null);

  function place(element: HTMLElement) {
    if (!target) {
      return;
    }
    const anchor = target.anchor.getBoundingClientRect();
    const column = target.anchor.closest<HTMLElement>("[data-hover-card-column]")?.getBoundingClientRect() ?? null;
    const size = element.getBoundingClientRect();
    position = hoverCardPosition(
      target.placement,
      anchor,
      column,
      { width: size.width, height: size.height },
      { width: window.innerWidth, height: window.innerHeight },
      {
        offset: lengthTokenInPixels("component-profile-hover-card-offset"),
        margin: lengthTokenInPixels("component-profile-hover-card-margin"),
      },
    );
  }

  // Shown in the top layer, placed before the first paint, placed again when About me arrives
  // and the card grows; anything moving under it (a scroll, a resize, leaving the window) closes it.
  // An effect, not a derivation: the position is measured from the laid-out card.
  $effect(() => {
    const element = card;
    if (!element || !target) {
      return;
    }
    if (!element.matches(":popover-open")) {
      element.showPopover();
    }
    place(element);
    const resizeObserver = new ResizeObserver(() => place(element));
    resizeObserver.observe(element);
    const close = () => closeProfileHoverCard();
    window.addEventListener("scroll", close, { capture: true, passive: true });
    window.addEventListener("resize", close);
    window.addEventListener("blur", close);
    return () => {
      resizeObserver.disconnect();
      window.removeEventListener("scroll", close, { capture: true });
      window.removeEventListener("resize", close);
      window.removeEventListener("blur", close);
      if (element.matches(":popover-open")) {
        element.hidePopover();
      }
      position = null;
    };
  });
</script>

<!-- Hidden from screen readers: the trigger's name already says this, and Enter on it opens the
     whole profile (DESIGN.md §5.7.2). Its button is out of the Tab order for the same reason. -->
<div
  bind:this={card}
  popover="manual"
  aria-hidden="true"
  data-side={position?.side}
  class="profile-hover-card fixed inset-auto m-0 w-(--mw-component-profile-hover-card-width) overflow-hidden rounded-(--mw-component-profile-hover-card-radius) border-(length:--mw-component-profile-hover-card-border-width) border-(--mw-component-profile-hover-card-border) bg-(--mw-component-profile-hover-card-background) p-(--mw-component-profile-hover-card-padding) text-start shadow-(--mw-component-profile-hover-card-shadow)"
  style:top={position ? `${position.top}px` : undefined}
  style:left={position ? `${position.left}px` : undefined}
  onpointerenter={profileHoverCardPointer.enter}
  onpointerleave={profileHoverCardPointer.leave}
>
  {#if target && user && facts}
    <div class="flex flex-col gap-(--mw-component-profile-hover-card-gap)">
      <div class="flex items-center gap-(--mw-component-profile-hover-card-gap)">
        <Avatar
          {user}
          sizeToken="--mw-component-profile-hover-card-avatar-size"
          animation="always"
          {presence}
          ringColorToken="--mw-component-profile-hover-card-background"
          statusSizeToken="--mw-component-profile-hover-card-status-size"
        />
        <div class="flex min-w-0 flex-1 flex-col">
          <p class="flex min-w-0 items-center gap-(--mw-component-role-icon-gap) type-heading-md text-(--mw-component-profile-hover-card-name-text)">
            <span class="truncate">{displayName(user)}</span>
            {#if facts.roleIcon}<RoleIcon role={facts.roleIcon} sizeToken="--mw-component-role-icon-member-size" />{/if}
          </p>
          <p class="type-body-sm text-(--mw-component-profile-hover-card-presence-text)">{strings.presence[presence]}</p>
          {#if customStatus}
            <p class="truncate type-body-sm text-(--mw-component-profile-hover-card-status-text)">{customStatus}</p>
          {/if}
        </div>
      </div>

      {#if aboutBlocks.length > 0}
        <!-- A glance: its links wait in the modal, so this part takes no pointer or focus. -->
        <div
          inert
          class="line-clamp-(--mw-component-profile-hover-card-about-lines) type-body-sm break-words text-(--mw-component-profile-hover-card-about-text)"
        >
          <MessageMarkdown blocks={aboutBlocks} />
        </div>
      {/if}

      {#if guild && facts.member && (facts.isOwner || facts.member.role_ids.length > 0)}
        <RoleChips
          {guild}
          roleIds={facts.member.role_ids}
          isOwner={facts.isOwner}
          label={strings.profileCard.rolesOf(displayName(user))}
          areIconsShown
          limit={numberTokenValue("component-profile-hover-card-role-limit")}
        />
      {/if}

      <div
        class="flex flex-wrap items-center justify-between gap-x-xs gap-y-2xs border-t-(length:--mw-component-profile-hover-card-border-width) border-(--mw-component-profile-hover-card-divider) pt-(--mw-component-profile-hover-card-padding)"
      >
        <p class="type-body-sm text-(--mw-component-profile-hover-card-footer-text)">{sinceLabel}</p>
        <button
          type="button"
          tabindex="-1"
          class="rounded-xs type-body-sm font-semibold text-(--mw-component-profile-hover-card-action-text) hover:underline"
          onclick={() => {
            const opener = target.anchor;
            openProfileModal(target.user, opener.isConnected ? opener : null);
          }}>{strings.profileCard.viewProfile}</button
        >
      </div>
    </div>
  {/if}
</div>

<style>
  /* Slides from the side of its trigger (DESIGN.md §5.7.2); reduced motion sets the duration to 0. */
  .profile-hover-card:popover-open {
    animation: profile-hover-card-in var(--mw-motion-duration-fast) var(--mw-motion-easing-enter) both;
  }
  .profile-hover-card:not([data-side]) {
    visibility: hidden;
  }
  .profile-hover-card[data-side="left"] {
    --hover-card-from: translateX(var(--mw-component-profile-hover-card-enter-offset));
  }
  .profile-hover-card[data-side="right"] {
    --hover-card-from: translateX(calc(-1 * var(--mw-component-profile-hover-card-enter-offset)));
  }
  .profile-hover-card[data-side="below"] {
    --hover-card-from: translateY(calc(-1 * var(--mw-component-profile-hover-card-enter-offset)));
  }
  .profile-hover-card[data-side="above"] {
    --hover-card-from: translateY(var(--mw-component-profile-hover-card-enter-offset));
  }
  @keyframes profile-hover-card-in {
    from {
      opacity: 0;
      transform: var(--hover-card-from, none);
    }
  }
</style>
