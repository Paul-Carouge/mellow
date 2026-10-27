<script lang="ts">
  import type { PublicUser } from "../lib/protocol";
  import Avatar from "./Avatar.svelte";
  import Icon from "./Icon.svelte";

  // A group's picture (DESIGN.md §5.6, ADR-0026): the first two other people leaning on each other,
  // the front face cut out of the surface behind; one face when only one is left, the users glyph
  // when you are alone. Decorative: the group's name always sits next to it.
  interface Props {
    /** Everyone in the group but you, in the instance's order. */
    people: readonly PublicUser[];
    /** A `--mw-*` length token for the whole avatar. */
    sizeToken: string;
    /** The surface behind, for the cut-out around the front face. */
    ringColorToken: string;
  }

  let { people, sizeToken, ringColorToken }: Props = $props();

  const [back, front] = $derived(people);
  const faceSize = $derived(`calc(var(${sizeToken}) * var(--mw-component-group-avatar-face-scale))`);
</script>

<span
  aria-hidden="true"
  class="relative inline-block shrink-0"
  style:width="var({sizeToken})"
  style:height="var({sizeToken})"
  style:--group-face-size={faceSize}
>
  {#if back && front}
    <span class="absolute start-0 top-0">
      <Avatar user={back} sizeToken="--group-face-size" />
    </span>
    <span
      class="absolute -end-(--mw-component-group-avatar-ring-width) -bottom-(--mw-component-group-avatar-ring-width) inline-flex rounded-full p-(--mw-component-group-avatar-ring-width)"
      style:background-color="var({ringColorToken})"
    >
      <Avatar user={front} sizeToken="--group-face-size" />
    </span>
  {:else if back}
    <Avatar user={back} {sizeToken} />
  {:else}
    <span
      class="grid size-full place-items-center rounded-full bg-(--mw-component-group-avatar-empty-background) text-(--mw-component-group-avatar-empty-icon)"
    >
      <Icon name="users" class="size-(--group-face-size)" />
    </span>
  {/if}
</span>
