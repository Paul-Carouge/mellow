<script lang="ts">
  import type { HTMLButtonAttributes } from "svelte/elements";

  import Icon, { type IconName } from "./Icon.svelte";

  // DESIGN.md §6.2: an icon, an accessible label and a tooltip; 44 px under a coarse pointer.
  interface Props extends Omit<HTMLButtonAttributes, "class" | "children"> {
    icon: IconName;
    label: string;
    /** Size, colour and hover utilities from the component's tokens. */
    class?: string;
    iconClass?: string;
    element?: HTMLButtonElement | null;
  }

  let { icon, label, class: className = "", iconClass = "", element = $bindable(null), ...rest }: Props = $props();
</script>

<button
  bind:this={element}
  type="button"
  aria-label={label}
  title={label}
  {...rest}
  class={[
    "inline-flex shrink-0 items-center justify-center rounded-sm transition-colors duration-(--mw-motion-duration-fast) ease-(--mw-motion-easing-standard) pointer-coarse:size-(--mw-size-hit-web-min)",
    className,
  ]}
>
  <Icon name={icon} class={iconClass} />
</button>
