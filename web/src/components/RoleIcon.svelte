<script lang="ts">
  import { isPictureAnimationWanted } from "../lib/motion.svelte";
  import { type Role, iconUrl } from "../lib/protocol";
  import { displayEmoji } from "../lib/reactions";

  // A role's icon next to a member's name (ADR-0050, DESIGN.md §5.4 and §5.7): its picture, still
  // while reduced motion keeps pictures still, or its emoji. The role's name is its accessible
  // label and tooltip, so the icon never says something colour or shape alone carries.
  interface Props {
    role: Role;
    /** A length token, as a CSS custom property name: the icon's side. */
    sizeToken: string;
    /** Inside a control that already names the role (the editor's preview). */
    isDecorative?: boolean;
  }

  let { role, sizeToken, isDecorative = false }: Props = $props();

  const source = $derived(role.icon ? iconUrl(role.icon, isPictureAnimationWanted(true)) : null);
  let failedSource = $state<string | null>(null);
</script>

{#if source && failedSource !== source}
  <img
    src={source}
    alt={isDecorative ? "" : role.name}
    title={isDecorative ? undefined : role.name}
    loading="lazy"
    decoding="async"
    draggable="false"
    width="64"
    height="64"
    class="inline-block size-(--role-icon-size) shrink-0 object-contain align-middle select-none"
    style:--role-icon-size="var({sizeToken})"
    onerror={() => (failedSource = source)}
  />
{:else if role.unicode_emoji}
  <span
    role={isDecorative ? undefined : "img"}
    aria-label={isDecorative ? undefined : role.name}
    aria-hidden={isDecorative ? "true" : undefined}
    title={isDecorative ? undefined : role.name}
    class="inline-flex size-(--role-icon-size) shrink-0 items-center justify-center text-(length:--role-icon-size) leading-none select-none"
    style:--role-icon-size="var({sizeToken})">{displayEmoji(role.unicode_emoji)}</span
  >
{/if}
