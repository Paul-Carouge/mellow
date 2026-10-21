<script lang="ts">
  import type { Snippet } from "svelte";

  import { type PublicUser, profileImageUrl } from "../lib/protocol";
  import Avatar, { prefersReducedMotion } from "./Avatar.svelte";
  import { displayName } from "./format";

  // The profile popout's card (DESIGN.md §5.7), also the live preview in Profile settings (§5.9):
  // the banner picture, or the plain strip, with the avatar overlapping it; both play animated,
  // as in Discord's popout, except under reduced motion.
  interface Props {
    user: Pick<PublicUser, "display_name" | "username" | "avatar" | "banner">;
    /** A heading in the popout; plain text in the settings preview, under its own heading. */
    nameElement?: "h2" | "p";
    /** Actions under the name, such as "Message". */
    children?: Snippet;
  }

  let { user, nameElement = "h2", children }: Props = $props();

  let failedBannerId = $state<string | null>(null);
  const banner = $derived(user.banner && user.banner.id !== failedBannerId ? user.banner : null);
</script>

{#if banner}
  {@const bannerId = banner.id}
  <img
    src={profileImageUrl(banner, !prefersReducedMotion.current)}
    alt=""
    decoding="async"
    draggable="false"
    class="block h-(--mw-component-profile-popover-banner-image-height) w-full bg-(--mw-component-profile-popover-banner) object-cover select-none"
    onerror={() => (failedBannerId = bannerId)}
  />
{:else}
  <div aria-hidden="true" class="h-(--mw-component-profile-popover-banner-height) bg-(--mw-component-profile-popover-banner)"></div>
{/if}
<div class="flex flex-col gap-(--mw-component-profile-popover-text-gap) px-(--mw-component-profile-popover-padding) pb-(--mw-component-profile-popover-padding)">
  <span
    class="-mt-[calc(var(--mw-component-profile-popover-avatar-size)/2)] w-fit rounded-full border-(length:--mw-component-profile-popover-avatar-ring-width) border-(--mw-component-profile-popover-background) bg-(--mw-component-profile-popover-background)"
  >
    <Avatar {user} sizeToken="--mw-component-profile-popover-avatar-size" animation="always" />
  </span>
  <div>
    <svelte:element this={nameElement} class="type-heading-md break-words text-(--mw-component-profile-popover-name-text)">
      {displayName(user)}
    </svelte:element>
    <p class="type-body break-words text-(--mw-component-profile-popover-username-text)">@{user.username}</p>
  </div>
  {@render children?.()}
</div>
