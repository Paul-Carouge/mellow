<script lang="ts">
  import type { Snippet } from "svelte";
  import type { HTMLButtonAttributes } from "svelte/elements";
  import { type ButtonVariant, buttonVariantClasses } from "./button-variants";

  // DESIGN.md §6.1 at the web size (44 px). Disabled and busy buttons stay focusable
  // (`aria-disabled`), so focus is not lost while an action runs.
  interface Props extends Omit<HTMLButtonAttributes, "disabled" | "class"> {
    variant?: ButtonVariant;
    isDisabled?: boolean;
    isFullWidth?: boolean;
    children: Snippet;
    onclick?: (event: MouseEvent) => void;
  }

  let {
    variant = "secondary",
    isDisabled = false,
    isFullWidth = false,
    type = "button",
    children,
    onclick,
    ...rest
  }: Props = $props();
</script>

<button
  {...rest}
  {type}
  aria-disabled={isDisabled || undefined}
  class={[
    "inline-flex h-(--mw-component-button-height-large) min-w-(--mw-size-hit-web-min) shrink-0 items-center justify-center rounded-(--mw-component-button-radius) px-(--mw-component-button-padding-inline) type-button transition-colors duration-(--mw-motion-duration-fast)",
    isFullWidth && "w-full",
    isDisabled && "pointer-events-none",
    buttonVariantClasses(variant, isDisabled),
  ]}
  onclick={(event) => {
    if (isDisabled) {
      event.preventDefault();
      return;
    }
    onclick?.(event);
  }}
>
  {@render children()}
</button>
