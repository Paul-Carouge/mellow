<script lang="ts">
  import type { Snippet } from "svelte";

  import { type PublicUser, profileImageUrl } from "../lib/protocol";
  import { isPictureAnimationWanted } from "../lib/motion.svelte";
  import Avatar from "./Avatar.svelte";
  import { avatarTintToken, displayName } from "./format";

  // A member's profile (DESIGN.md §5.7, ADR-0022 §1), in the side sheet and as the live preview in
  // Profile settings (§5.9): the banner picture, or the plain strip, with the avatar overlapping
  // it; both play animated, as in Discord's popout, unless reduced motion keeps pictures still.
  interface Props {
    user: Pick<PublicUser, "id" | "display_name" | "username" | "avatar" | "banner">;
    /** A heading in the side sheet; plain text in the settings preview, under its own heading. */
    nameElement?: "h3" | "p";
    /** The surface behind the card: the avatar's ring is cut from it. */
    surfaceToken?: string;
    /** Actions under the name, such as "Send message". */
    children?: Snippet;
    /** Drawn in the banner's place, in a box of its size: the crop dialog's live framing. */
    bannerPicture?: Snippet;
  }

  let {
    user,
    nameElement = "h3",
    surfaceToken = "--mw-component-profile-popover-background",
    children,
    bannerPicture,
  }: Props = $props();

  let failedBannerId = $state<string | null>(null);
  const banner = $derived(user.banner && user.banner.id !== failedBannerId ? user.banner : null);
</script>

{#if bannerPicture}
  <div class="relative h-(--mw-component-profile-popover-banner-image-height) overflow-hidden bg-(--mw-component-profile-popover-banner)">
    {@render bannerPicture()}
  </div>
{:else if banner}
  {@const bannerId = banner.id}
  <img
    src={profileImageUrl(banner, isPictureAnimationWanted(true))}
    alt=""
    decoding="async"
    draggable="false"
    class="block h-(--mw-component-profile-popover-banner-image-height) w-full bg-(--mw-component-profile-popover-banner) object-cover select-none"
    onerror={() => (failedBannerId = bannerId)}
  />
{:else}
  <!-- No banner picture: the strip takes the person's avatar tint, as in the Dusk mockup. -->
  <div
    aria-hidden="true"
    class="h-(--mw-component-profile-popover-banner-height)"
    style:background-color="var({avatarTintToken(user.id)})"
  ></div>
{/if}
<div class="flex flex-col gap-(--mw-component-profile-popover-text-gap) px-(--mw-component-profile-popover-padding) pb-(--mw-component-profile-popover-padding)">
  <!-- The ring is drawn outside the avatar, so the avatar keeps the card's padding and sits one
       gap below the banner's middle line. -->
  <span
    class="mt-[calc(var(--mw-component-profile-popover-text-gap)_-_var(--mw-component-profile-popover-avatar-size)/2)] w-fit rounded-full"
    style:background-color="var({surfaceToken})"
    style:box-shadow="0 0 0 var(--mw-component-profile-popover-avatar-ring-width) var({surfaceToken})"
  >
    <Avatar {user} sizeToken="--mw-component-profile-popover-avatar-size" animation="always" />
  </span>
  <svelte:element this={nameElement} class="mt-xs type-heading-lg break-words text-(--mw-component-profile-popover-name-text)">
    {displayName(user)}
  </svelte:element>
  {@render children?.()}
</div>
