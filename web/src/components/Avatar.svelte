<script lang="ts">
  import type { PublicUser } from "../lib/protocol";
  import { avatarInitial } from "./format";

  // shortcut: initials on the accent fill until avatar images exist (M2 attachments), as on desktop.
  interface Props {
    user: Pick<PublicUser, "display_name" | "username">;
    /** A `--mw-*` length token, for example "--mw-component-member-row-avatar-size". */
    sizeToken: string;
    /** Shows the presence dot, cut out of `ringColorToken` (the surface behind the avatar). */
    presence?: "online" | "offline" | null;
    ringColorToken?: string;
    isDimmed?: boolean;
  }

  let { user, sizeToken, presence = null, ringColorToken = "--mw-color-surface-sidebar", isDimmed = false }: Props =
    $props();
</script>

<!-- The initial is set at 40 % of the avatar, as on desktop (primitives.slint). -->
<span class="relative inline-flex shrink-0" style:width="var({sizeToken})" style:height="var({sizeToken})">
  <span
    aria-hidden="true"
    class={[
      "grid size-full place-items-center rounded-full bg-accent-fill font-medium text-text-on-accent select-none",
      isDimmed && "opacity-(--mw-component-member-row-offline-avatar-opacity)",
    ]}
    style:font-size="calc(var({sizeToken}) * 0.4)">{avatarInitial(user)}</span
  >
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
    <span class="sr-only">{presence === "online" ? "Online" : "Offline"}</span>
  {/if}
</span>
