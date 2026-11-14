<script lang="ts">
  import type { Snippet } from "svelte";

  import { type PublicUser, type Role, profileImageUrl } from "../lib/protocol";
  import { isPictureAnimationWanted } from "../lib/motion.svelte";
  import Avatar from "./Avatar.svelte";
  import { avatarTintToken, displayName } from "./format";
  import RoleIcon from "./RoleIcon.svelte";
  import type { ShownStatus } from "./StatusIndicator.svelte";

  // A member's profile (DESIGN.md §5.7, ADR-0022 §1), in the side sheet and as the live preview in
  // Profile settings (§5.9): the banner picture, or the plain strip, with the avatar overlapping
  // it; both play animated, as in Discord's popout, unless reduced motion keeps pictures still.
  interface Props {
    user: Pick<PublicUser, "id" | "display_name" | "username" | "avatar" | "banner">;
    /** The modal's heading (it names the dialog); plain text in the settings preview, under its own heading. */
    nameElement?: "h2" | "h3" | "p";
    /** Gives the name (without the role icon) an id, for the dialog that it names. */
    nameId?: string;
    /** The presence shape on the avatar (the profile modal); the preview shows none. */
    presence?: ShownStatus | null;
    /** The person's highest role with an icon here, after their name (ADR-0050). */
    roleIcon?: Role | null;
    /** The banner across a wider card (the phone sheet) keeps its 5:2 framing instead of 120 px. */
    isBannerProportional?: boolean;
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
    nameId,
    presence = null,
    roleIcon = null,
    isBannerProportional = false,
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
    class={[
      "block w-full bg-(--mw-component-profile-popover-banner) object-cover select-none",
      isBannerProportional ? "aspect-5/2" : "h-(--mw-component-profile-popover-banner-image-height)",
    ]}
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
    <Avatar
      {user}
      sizeToken="--mw-component-profile-popover-avatar-size"
      animation="always"
      {presence}
      ringColorToken={surfaceToken}
      statusSizeToken="--mw-component-status-size-large"
    />
  </span>
  <svelte:element this={nameElement} class="mt-xs type-heading-lg break-words text-(--mw-component-profile-popover-name-text)">
    <!-- The name alone names the dialog; the role icon after it keeps its own label. -->
    <span id={nameId}>{displayName(user)}</span>
    {#if roleIcon}
      <!-- After the name, as in messages; the role's name is its label. -->
      <span class="ms-(--mw-component-role-icon-gap) inline-flex align-middle"
        ><RoleIcon role={roleIcon} sizeToken="--mw-component-role-icon-message-size" /></span
      >
    {/if}
  </svelte:element>
  {@render children?.()}
</div>
