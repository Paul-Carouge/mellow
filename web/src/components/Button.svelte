<script lang="ts">
  import type { Snippet } from "svelte";
  import type { HTMLButtonAttributes } from "svelte/elements";

  // DESIGN.md §6.1 at the web size (44 px). Disabled and busy buttons dim but stay focusable
  // (`aria-disabled`), so focus is not lost while an action runs.
  interface Props extends Omit<HTMLButtonAttributes, "disabled" | "class"> {
    variant?: "primary" | "secondary" | "danger";
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
    variant === "primary" &&
      "bg-(--mw-component-button-primary-background) text-(--mw-component-button-primary-text) enabled:hover:bg-(--mw-component-button-primary-background-hover) enabled:active:bg-(--mw-component-button-primary-background-pressed)",
    variant === "secondary" &&
      "bg-(--mw-component-button-secondary-background) text-(--mw-component-button-secondary-text) hover:bg-(--mw-component-button-secondary-background-hover)",
    variant === "danger" &&
      "bg-(--mw-component-button-danger-background) text-(--mw-component-button-danger-text) hover:bg-(--mw-component-button-danger-background-hover)",
    isDisabled && "pointer-events-none opacity-(--mw-component-button-disabled-opacity)",
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
