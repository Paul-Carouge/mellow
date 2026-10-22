<script lang="ts">
  import { strings } from "../i18n";
  import { prefersReducedMotion } from "../lib/motion.svelte";
  import { type PublicUser, profileImageUrl } from "../lib/protocol";
  import { avatarInitial, avatarTintToken } from "./format";

  // The user's picture (ADR-0021), or their initial on their own tint while it loads, when they
  // have none or when it fails. Decorative: the name always sits next to it.
  interface Props {
    user: Pick<PublicUser, "id" | "display_name" | "username" | "avatar">;
    /** A `--mw-*` length token, for example "--mw-component-member-row-avatar-size". */
    sizeToken: string;
    /**
     * "on-hover": the still picture, animated while the enclosing row (or the avatar itself) is
     * hovered or holds focus, as on Discord; "always": the profile card and its settings preview.
     */
    animation?: "on-hover" | "always";
    /** Shows the presence dot, cut out of `ringColorToken` (the surface behind the avatar). */
    presence?: "online" | "offline" | null;
    ringColorToken?: string;
    isDimmed?: boolean;
  }

  let {
    user,
    sizeToken,
    animation = "on-hover",
    presence = null,
    ringColorToken = "--mw-color-surface-sidebar",
    isDimmed = false,
  }: Props = $props();

  let root = $state<HTMLElement | null>(null);
  let isHoverAreaActive = $state(false);
  /** The picture that arrived: the initial stays behind it only while it loads. */
  let loadedImageId = $state<string | null>(null);
  let failedImageId = $state<string | null>(null);

  const image = $derived(user.avatar && user.avatar.id !== failedImageId ? user.avatar : null);
  const isAnimationWanted = $derived(
    !prefersReducedMotion.current && (animation === "always" || isHoverAreaActive),
  );
  const isPictureShown = $derived(image !== null && loadedImageId === image.id);

  $effect(() => {
    if (!root || animation !== "on-hover" || !image?.is_animated) {
      return;
    }
    // Rows are buttons or list items (member, conversation, message); elsewhere the avatar itself.
    const area: HTMLElement = root.closest("button, li") ?? root;
    const activate = () => (isHoverAreaActive = true);
    const leavePointer = () => (isHoverAreaActive = area.contains(document.activeElement));
    const leaveFocus = (event: FocusEvent) => {
      if (!(event.relatedTarget instanceof Node && area.contains(event.relatedTarget))) {
        isHoverAreaActive = area.matches(":hover");
      }
    };
    area.addEventListener("pointerenter", activate);
    area.addEventListener("pointerleave", leavePointer);
    area.addEventListener("focusin", activate);
    area.addEventListener("focusout", leaveFocus);
    return () => {
      area.removeEventListener("pointerenter", activate);
      area.removeEventListener("pointerleave", leavePointer);
      area.removeEventListener("focusin", activate);
      area.removeEventListener("focusout", leaveFocus);
      isHoverAreaActive = false;
    };
  });
</script>

<!-- The initial is set at 40 % of the avatar, as on desktop (primitives.slint). -->
<span bind:this={root} class="relative inline-flex shrink-0" style:width="var({sizeToken})" style:height="var({sizeToken})">
  <span aria-hidden="true" class={["relative size-full", isDimmed && "opacity-(--mw-component-member-row-offline-avatar-opacity)"]}>
    {#if !isPictureShown}
      <span
        class="grid size-full place-items-center rounded-full font-semibold text-avatar-initial select-none"
        style:background-color="var({avatarTintToken(user.id)})"
        style:font-size="calc(var({sizeToken}) * 0.4)">{avatarInitial(user)}</span
      >
    {/if}
    {#if image}
      {@const imageId = image.id}
      <img
        src={profileImageUrl(image, isAnimationWanted)}
        alt=""
        decoding="async"
        draggable="false"
        class={[
          "size-full rounded-full object-cover select-none",
          !isPictureShown && "invisible absolute inset-0",
        ]}
        onload={() => (loadedImageId = imageId)}
        onerror={() => (failedImageId = imageId)}
      />
    {/if}
  </span>
  {#if presence}
    <!-- Shape and colour (DESIGN.md §2.3): a filled dot online, a hollow ring offline. -->
    <span
      class="absolute -right-(--mw-component-status-ring-width) -bottom-(--mw-component-status-ring-width) grid place-items-center rounded-full"
      style:width="calc(var(--mw-component-status-size-small) + 2 * var(--mw-component-status-ring-width))"
      style:height="calc(var(--mw-component-status-size-small) + 2 * var(--mw-component-status-ring-width))"
      style:background-color="var({ringColorToken})"
    >
      <span
        class={[
          "size-(--mw-component-status-size-small) rounded-full",
          presence === "online"
            ? "bg-(--mw-component-status-online)"
            : "border-(length:--mw-component-status-ring-width) border-(--mw-component-status-offline)",
        ]}
      ></span>
    </span>
    <span class="sr-only">{presence === "online" ? strings.common.online : strings.common.offline}</span>
  {/if}
</span>
